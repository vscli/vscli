'use strict';
// Source-guided native bridge, VS Code stable 1.141.0, Microsoft MIT.
// RelativePattern follows extHostTypes.ts at immutable 2a59476c9bfcb90b3ddc372c36762471b7dfad1c.
/*
MIT License

Copyright (c) 2015 - present Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/
const { Uri } = require('./api-types.cjs');
const { es5ClassCompat } = require('./es5-class-compat.cjs');
const { CancellationToken } = require('./cancellation.cjs');
const { bindCurrentListener } = require('./caller-context.cjs');
const uriFrom = Uri.from, uriFormat = Uri.prototype.toString;
const MAX_PATTERN = 4096, MAX_RESULTS = 4096, MAX_WIRE = 2 * 1024 * 1024;
class RelativePattern {
  get base() { return this._base; }
  set base(base) { this._base = base; this._baseUri = Uri.file(base); }
  get baseUri() { return this._baseUri; }
  set baseUri(baseUri) { this._baseUri = baseUri; this._base = baseUri.fsPath; }
  constructor(base, pattern) {
    if (typeof base !== 'string' && (!base || !Uri.isUri(base) && !Uri.isUri(base.uri))) throw new Error('Illegal argument: base');
    if (typeof pattern !== 'string') throw new Error('Illegal argument: pattern');
    if (typeof base === 'string') this.baseUri = Uri.file(base);
    else if (Uri.isUri(base)) this.baseUri = base;
    else this.baseUri = base.uri;
    this.pattern = pattern;
  }
  toJSON() { return { pattern:this.pattern, base:this.base, baseUri:this.baseUri.toJSON() }; }
}
es5ClassCompat(RelativePattern);
function pattern(value, label) {
  if (typeof value !== 'string') throw new TypeError(`${label} must be a glob string`);
  if (value.length > MAX_PATTERN || Buffer.byteLength(value) > MAX_PATTERN || value.includes('\0')) throw new Error(`${label} exceeds bounded glob input`);
  return value || null;
}
function fileBase(value) {
  if (!Uri.isUri(value)) throw new TypeError('RelativePattern baseUri must be a URI');
  const scheme = value.scheme, authority = value.authority, path = value.path, query = value.query, fragment = value.fragment;
  let total = 0;
  for (const component of [scheme,authority,path,query,fragment]) {
    if (typeof component !== 'string' || component.length > 4096 || component.includes('\0')) throw new Error('Invalid findFiles base URI');
    total += Buffer.byteLength(component); if (total > 4096) throw new Error('findFiles base URI exceeds 4096 bytes');
  }
  if (scheme !== 'file' || query || fragment) throw new Error('Native findFiles requires an ordinary file base URI');
  const result = Reflect.apply(uriFormat, Reflect.apply(uriFrom, Uri, [{scheme,authority,path,query,fragment},true]), []);
  if (result.length > 4096 || Buffer.byteLength(result) > 4096) throw new Error('findFiles encoded base URI exceeds 4096 bytes');
  return result;
}
function include(value) {
  if (value == null || typeof value === 'string') return { pattern:value == null ? null : pattern(value,'Include'), baseUri:null };
  if (!value || typeof value !== 'object') throw new TypeError('Invalid findFiles include pattern');
  const baseUri = value.baseUri;
  const base = baseUri == null ? value.base : undefined;
  const glob = value.pattern;
  const uri = baseUri ?? (typeof base === 'string' ? Uri.file(base) : undefined);
  return { pattern:pattern(glob,'Include'), baseUri:fileBase(uri) };
}
function exclude(value) {
  if (value === null) return { mode:'none', pattern:null };
  if (value === undefined) return { mode:'default', pattern:null };
  // Pinned legacy findFiles intentionally ignores RelativePattern exclude base.
  const glob = pattern(typeof value === 'string' ? value : value?.pattern,'Exclude');
  return { mode:glob ? 'pattern' : 'default', pattern:glob };
}
function isFindFilesResult(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const keys = Object.keys(value);
  if (keys.length !== 2 || !keys.includes('uris') || !keys.includes('cancelled') || typeof value.cancelled !== 'boolean' || !Array.isArray(value.uris)) return false;
  const length = value.uris.length;
  if (length > MAX_RESULTS || value.cancelled && length !== 0) return false;
  let bytes = Buffer.byteLength(value.cancelled ? '{"uris":[],"cancelled":true}' : '{"uris":[],"cancelled":false}');
  for (let i = 0; i < length; i++) {
    const uri = value.uris[i];
    if (typeof uri !== 'string' || uri.length > 4096 || uri.includes('\0') || Buffer.byteLength(uri) > 4096) return false;
    bytes += Buffer.byteLength(JSON.stringify(uri)) + (i > 0 ? 1 : 0);
    if (bytes > MAX_WIRE) return false;
  }
  return value.uris.length === length;
}
function createFileSearch(call, options, model) {
  const unowned = options.unowned || (callback => callback());
  function forOwner(owner) {
    async function findFiles(input, excluded, maxResults, token = CancellationToken.None) {
      const guard = model.captureCaller(owner); guard();
      const workspace = model.workspace();
      // Preserve pinned ordering: legacy exclude normalization precedes the
      // pre-cancel shortcut; include conversion only happens after that check.
      const excludedPattern = exclude(excluded);
      const initiallyCancelled = token.isCancellationRequested;
      guard();
      if (initiallyCancelled) return [];
      const included = include(input);
      if (maxResults !== undefined && (!Number.isSafeInteger(maxResults) || maxResults < 0 || maxResults > MAX_RESULTS)) throw new RangeError('Native findFiles maxResults must be an integer between 0 and 4096');
      const args = { include:included, exclude:excludedPattern, maxResults:maxResults ?? null };
      const event = token.onCancellationRequested;
      if (typeof event !== 'function') throw new TypeError('findFiles requires a cancellation event');
      if (typeof initiallyCancelled !== 'boolean') throw new TypeError('Invalid cancellation token state');
      let cancelled = false, closed = false, requestId, cancelSent = false, registration, dispose;
      let resolveCancellation;
      const cancellation = new Promise(resolve => { resolveCancellation = resolve; });
      function sendCancellation() {
        if (!requestId || cancelSent) return;
        cancelSent = true;
        try {
          const result = unowned(() => options.cancel(owner, requestId));
          Promise.resolve(result).catch(error => options.cancelError?.(error));
        } catch (error) { options.cancelError?.(error); }
      }
      const onCancel = () => {
        if (closed || cancelled) return;
        cancelled = true;
        sendCancellation();
        resolveCancellation([]);
      };
      try {
        // Core observers stay unowned: retiring the extension cannot hide a
        // cancellation or prevent exact terminal-capacity release.
        registration = unowned(() => Reflect.apply(event, token, [onCancel]));
        const cleanup = registration?.dispose;
        if (registration && typeof cleanup !== 'function') throw new TypeError('Invalid cancellation listener disposal');
        // Foreign token cleanup is extension code. Preserve its original caller
        // lease while the internal cancellation callback remains unowned.
        if (typeof cleanup === 'function') dispose = bindCurrentListener(cleanup);
        guard();
        const finallyCancelled = token.isCancellationRequested;
        guard();
        if (model.workspace().identity !== workspace.identity) throw new Error('findFiles workspace/configuration changed during capture');
        if (cancelled || finallyCancelled) return [];
        // A relative file base remains useful even with no opened workspace.
        if (!workspace.root && !included.baseUri) return [];
        const pending = call(owner, 'nativeFindFiles', args, {
          onRequest(id) {
            if (typeof id !== 'string' || !id || id.length > 128) throw new Error('Invalid native findFiles request identity');
            requestId = id;
            if (cancelled) sendCancellation();
          },
        });
        const result = Promise.resolve(pending).then(value => {
          guard();
          if (!isFindFilesResult(value)) throw new Error('Invalid native findFiles acknowledgement');
          if (cancelled || value.cancelled) return [];
          const uris = value.uris.map(uri => {
            const result = Uri.parse(uri, true);
            if (result.scheme !== 'file' || result.query || result.fragment || result.path.includes('\0')) throw new Error('Invalid native findFiles resource');
            return result;
          });
          guard();
          return uris;
        });
        const values = await Promise.race([result,cancellation]);
        guard();
        return values;
      } finally {
        closed = true;
        if (typeof dispose === 'function') {
          try { unowned(() => Reflect.apply(dispose, registration, [])); }
          catch (error) { options.cleanupError?.(error); }
        }
      }
    }
    return { findFiles };
  }
  return { forOwner };
}
module.exports = { RelativePattern, createFileSearch, isFindFilesResult };
