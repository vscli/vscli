'use strict';

// Source-guided workspace metadata API, VS Code stable 1.141.0 (MIT).
// Microsoft commit 2a59476c9bfcb90b3ddc372c36762471b7dfad1c:
// src/vs/workbench/api/common/extHostWorkspace.ts, resources.ts, ternarySearchTree.ts.
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
const path = require('node:path');
const MAX_FOLDERS = 128, MAX_URI_BYTES = 8192, MAX_NAME_BYTES = 4096;
const MAX_TOTAL_BYTES = 512 * 1024, MAX_METADATA_BYTES = 2 * 1024 * 1024;
function text(value, limit, field) {
  if (typeof value !== 'string' || value.length > limit || Buffer.byteLength(value) > limit) {
    throw new TypeError(`Workspace ${field} exceeds its bounded string contract`);
  }
  return value;
}
function originalFilePath(uri, windows) {
  let result;
  if (uri.authority && uri.path.length > 1 && uri.scheme === 'file') result = `//${uri.authority}${uri.path}`;
  else if (/^\/[A-Za-z]:/.test(uri.path)) result = uri.path.slice(1);
  else result = uri.path;
  return windows ? result.replace(/\//g, '\\') : result;
}
function compare(a, b) { return a < b ? -1 : a > b ? 1 : 0; }
// Preserve pinned strings.ts comparison bounds, including Unicode lower-case
// expansions. A whole-segment toLowerCase equality is not equivalent here.
function compareSubstring(a, b, aStart = 0, aEnd = a.length, bStart = 0, bEnd = b.length) {
  for (; aStart < aEnd && bStart < bEnd; aStart++, bStart++) {
    const codeA = a.charCodeAt(aStart), codeB = b.charCodeAt(bStart);
    if (codeA < codeB) return -1;
    if (codeA > codeB) return 1;
  }
  const aLen = aEnd - aStart, bLen = bEnd - bStart;
  return aLen < bLen ? -1 : aLen > bLen ? 1 : 0;
}
function compareSubstringIgnoreCase(a, b, aStart = 0, aEnd = a.length, bStart = 0, bEnd = b.length, aLower, bLower) {
  for (; aStart < aEnd && bStart < bEnd; aStart++, bStart++) {
    let codeA = a.charCodeAt(aStart), codeB = b.charCodeAt(bStart);
    if (codeA === codeB) continue;
    if (codeA >= 128 || codeB >= 128) return compareSubstring(aLower ?? a.toLowerCase(), bLower ?? b.toLowerCase(), aStart, aEnd, bStart, bEnd);
    if (codeA >= 97 && codeA <= 122) codeA -= 32;
    if (codeB >= 97 && codeB <= 122) codeB -= 32;
    const diff = codeA - codeB;
    if (diff !== 0) return diff;
  }
  const aLen = aEnd - aStart, bLen = bEnd - bStart;
  return aLen < bLen ? -1 : aLen > bLen ? 1 : 0;
}
function equalAuthority(a, b) { return a === b || a.length === b.length && compareSubstringIgnoreCase(a, b) === 0; }

function createWorkspaceScope({ Uri, EventEmitter, platform = process.platform, pathCaseSensitive = {} } = {}) {
  if (typeof Uri?.from !== 'function' || typeof Uri?.file !== 'function' || typeof EventEmitter !== 'function') {
    throw new TypeError('Workspace scope requires the actual host Uri and EventEmitter types');
  }
  const windows = platform === 'win32', nativePath = windows ? path.win32 : path.posix;
  const caseSensitive = new Map([['file', platform === 'linux']]);
  const capability = typeof pathCaseSensitive === 'function' ? pathCaseSensitive : undefined;
  const capabilityKeys = capability ? [] : Object.keys(pathCaseSensitive);
  if (capabilityKeys.length > MAX_FOLDERS) throw new Error('Workspace provider-capability budget exceeded');
  for (const key of capabilityKeys) {
    text(key, 128, 'scheme');
    const value = pathCaseSensitive[key];
    if (typeof value !== 'boolean') throw new TypeError('Workspace path capability must be boolean');
    caseSensitive.set(key, value);
  }
  const changed = new EventEmitter(), granted = new EventEmitter();
  let generation = -1, current, trusted = false, disposed = false;
  const ignoreCase = record => !record.caseSensitive;
  function captureCase(scheme) {
    const value = capability?.(scheme);
    if (value !== undefined && typeof value !== 'boolean') throw new TypeError('Workspace path capability must be boolean');
    return value === undefined ? !!caseSensitive.get(scheme) : value;
  }
  function captureUri(value) {
    if (!value || typeof value !== 'object') throw new TypeError('Workspace resource requires a Uri');
    // Capture once before any lookup/publication. The private URI is made from
    // those fields, so extension-supplied property accessors cannot change keys.
    const parts = { scheme: value.scheme, authority: value.authority, path: value.path, query: value.query, fragment: value.fragment };
    let bytes = 0;
    for (const key of ['scheme', 'authority', 'path', 'query', 'fragment']) {
      text(parts[key], key === 'scheme' ? 128 : MAX_URI_BYTES, `Uri.${key}`);
      bytes += Buffer.byteLength(parts[key]);
    }
    if (bytes > MAX_URI_BYTES) throw new Error('Workspace URI aggregate exceeds 8 KiB');
    const uri = Uri.from(parts);
    let segmentCount = 0, inSegment = false;
    for (const ch of parts.path) {
      if (ch === '/') inSegment = false;
      else if (!inSegment) { segmentCount++; inSegment = true; }
    }
    // Conservative preflight for retained URI/token/lowercase and staging metadata
    // before token arrays or lower-case strings are allocated. Not a V8 heap meter.
    const metadata = 512 + bytes * 16 + (segmentCount + 2) * 192;
    if (metadata > MAX_METADATA_BYTES) throw new Error('Workspace URI metadata budget exceeded');
    return { uri, scheme: uri.scheme, authority: uri.authority, path: uri.path,
      query: uri.query, fragment: uri.fragment, serialized: uri.toString(), bytes, metadata, segmentCount, caseSensitive: captureCase(uri.scheme) };
  }
  function tokens(record) {
    const result = [];
    if (record.scheme) result.push({ text: record.scheme, lower: record.scheme.toLowerCase(), source: record.scheme, sourceLower: record.scheme.toLowerCase(), start: 0, end: record.scheme.length });
    if (record.authority) result.push({ text: record.authority, lower: record.authority.toLowerCase(), source: record.authority, sourceLower: record.authority.toLowerCase(), start: 0, end: record.authority.length });
    const pathLower = record.path.toLowerCase();
    // PathIterator comparisons retain offsets into the original whole path;
    // Unicode case expansions before a segment affect its pinned fallback.
    let start = 0;
    for (let index = 0; index <= record.path.length; index++) {
      if (index === record.path.length || record.path.charCodeAt(index) === 47) {
        if (index > start) {
          const segment = record.path.slice(start, index);
          result.push({ text: segment, lower: segment.toLowerCase(), source: record.path, sourceLower: pathLower, start, end: index });
        }
        start = index + 1;
      }
    }
    return result;
  }
  function sameToken(left, right, index, query) {
    const insensitive = index === 0 || (query.authority && index === 1) || ignoreCase(query);
    return insensitive
      ? compareSubstringIgnoreCase(left.text, right.source, 0, left.text.length, right.start, right.end, left.lower, right.sourceLower) === 0
      : left.text === right.text;
  }
  function prefix(entry, query, record) {
    return entry.tokens.length <= query.length && entry.tokens.every((part, index) => sameToken(part, query[index], index, record));
  }
  function identity(record) {
    // ExtUri's folder equality keeps query/fragment and literal slash spelling;
    // lookup's segment tree intentionally ignores them and trailing separators.
    return record.uri.with({ path: ignoreCase(record) ? record.path.toLowerCase() : undefined }).toString();
  }
  function find(record, resolveParent = false) {
    let query = tokens(record);
    const exact = current?.entries.some(entry => entry.tokens.length === query.length && prefix(entry, query, record));
    if (resolveParent && exact && record.path.length > 0) {
      const parent = record.scheme === 'file'
        ? Uri.file(nativePath.dirname(originalFilePath(record, windows))).path
        : path.posix.dirname(record.path);
      record = { ...record, path: record.authority && parent && parent[0] !== '/' ? '/' : parent };
      query = tokens(record);
    }
    let selected;
    for (const entry of current?.entries || []) {
      if (prefix(entry, query, record)
        && (!selected || entry.tokens.length >= selected.tokens.length)) selected = entry;
    }
    return selected;
  }
  function relative(from, to) {
    if (from.scheme !== to.scheme || !equalAuthority(from.authority, to.authority)) return undefined;
    if (from.scheme === 'file') {
      const result = nativePath.relative(originalFilePath(from, windows), originalFilePath(to, windows));
      return windows ? result.replace(/\\/g, '/') : result;
    }
    // Public asRelativePath uses the default case-sensitive ExtUri for remote
    // path math, even when provider-capability folder lookup ignored path case.
    return path.posix.relative(from.path || '/', to.path || '/');
  }
  function getWorkspaceFolder(uri) { return find(captureUri(uri))?.folder; }
  function asRelativePath(pathOrUri, includeWorkspaceFolder) {
    if (pathOrUri === undefined) return '';
    let resource, original;
    if (typeof pathOrUri === 'string') {
      original = text(pathOrUri, MAX_URI_BYTES, 'input path');
      resource = captureUri(Uri.file(original)); // Never resolve against process.cwd().
    } else {
      resource = captureUri(pathOrUri);
      original = text(resource.uri.fsPath, MAX_URI_BYTES, 'fsPath');
    }
    const folder = find(resource, true);
    if (!folder) return original;
    if (includeWorkspaceFolder === undefined) includeWorkspaceFolder = current.entries.length > 1;
    const result = relative(folder.resource, resource);
    return includeWorkspaceFolder && folder.name ? `${folder.name}/${result}` : result;
  }
  const api = Object.freeze({
    get workspaceFolders() { return current?.entries.map(entry => entry.folder); },
    get rootPath() { return current?.entries[0]?.resource.uri.fsPath; },
    get name() { return current?.name; },
    get workspaceFile() { return current?.workspaceFile; },
    get isTrusted() { return trusted; },
    getWorkspaceFolder,
    asRelativePath,
    onDidChangeWorkspaceFolders: changed.event,
    onDidGrantWorkspaceTrust: granted.event,
    // The native editor has no multi-root mutation/acknowledgment transaction
    // yet. A successful start or fabricated event would be dishonest here.
    updateWorkspaceFolders() { return false; },
  });
  function publish(snapshot, { emit = true } = {}) {
    if (disposed) throw new Error('Workspace scope has been disposed');
    const oldGeneration = generation, previous = current, previouslyTrusted = trusted;
    const nextGeneration = snapshot.generation, workspace = snapshot.workspace, nextTrusted = snapshot.trusted;
    if (!Number.isSafeInteger(nextGeneration) || nextGeneration < 0 || typeof nextTrusted !== 'boolean' || typeof emit !== 'boolean') {
      throw new TypeError('Invalid authoritative workspace snapshot');
    }
    if (nextGeneration <= generation) return false;
    // Folder equality is evaluated with current provider capabilities, not a
    // stale normalization key from a previous provider registration.
    const old = (previous?.entries || []).map(entry => ({ ...entry, key: identity({ ...entry.resource, caseSensitive: captureCase(entry.resource.scheme) }) }));
    let next;
    if (workspace !== null) {
      if (!workspace || typeof workspace !== 'object') throw new TypeError('Workspace must be a bounded object or null');
      const name = text(workspace.name, MAX_NAME_BYTES, 'name'), input = workspace.folders;
      const configuration = workspace.configuration, isUntitled = workspace.isUntitled === true;
      const count = input?.length;
      if (!Array.isArray(input) || !Number.isSafeInteger(count) || count < 0 || count > MAX_FOLDERS) throw new Error('Workspace folder budget exceeded (128)');
      let bytes = Buffer.byteLength(name), metadata = 256;
      const staged = [];
      for (let index = 0; index < count; index++) {
        const value = input[index], folderUri = value?.uri, folderName = value?.name;
        const resource = captureUri(folderUri);
        const resolvedName = text(folderName === undefined ? path.posix.basename(resource.path) || resource.authority : folderName, MAX_NAME_BYTES, 'folder name');
        bytes += resource.bytes + Buffer.byteLength(resolvedName);
        metadata += resource.metadata + 512 + Buffer.byteLength(resolvedName) * 16;
        if (bytes > MAX_TOTAL_BYTES || metadata > MAX_METADATA_BYTES) throw new Error('Workspace aggregate metadata budget exceeded');
        staged.push({ resource, name: resolvedName, index });
      }
      if (input.length !== count) throw new Error('Workspace folder array changed during publication');
      let workspaceFile;
      if (configuration !== undefined && configuration !== null) {
        const resource = captureUri(configuration);
        bytes += resource.bytes; metadata += resource.metadata;
        if (bytes > MAX_TOTAL_BYTES || metadata > MAX_METADATA_BYTES) throw new Error('Workspace aggregate metadata budget exceeded');
        workspaceFile = isUntitled
          ? Uri.from({ scheme: 'untitled', path: path.posix.basename(path.posix.dirname(resource.path)) })
          : resource.uri;
      }
      // Stable source rebuilds a URI segment tree with later identical keys
      // winning. The bounded linear index has the same longest-prefix policy.
      const entries = staged.map(row => {
        const key = identity(row.resource);
        const retained = old.find(entry => entry.key === key);
        const entry = { ...row, resource: retained?.resource || row.resource, key, tokens: tokens(row.resource), folder: retained?.folder };
        if (!entry.folder) {
          // Public declarations are readonly. Backing metadata lets retained
          // references stay live without trusting extension-written properties.
          const binding = { name: row.name, index: row.index };
          entry.binding = binding;
          entry.folder = Object.freeze({ uri: entry.resource.uri, get name() { return binding.name; }, get index() { return binding.index; } });
        } else entry.binding = retained.binding;
        return entry;
      });
      next = { name, workspaceFile, entries };
    }
    const now = next?.entries || [];
    const added = now.filter(entry => !old.some(before => before.key === entry.key)).sort((a, b) => compare(a.resource.serialized, b.resource.serialized)).map(entry => entry.folder);
    // toExtHostWorkspace(null) clears the workspace but emits empty arrays.
    const removed = workspace === null ? [] : old.filter(entry => !now.some(after => after.key === entry.key)).sort((a, b) => compare(a.resource.serialized, b.resource.serialized)).map(entry => entry.folder);
    const notification = Object.freeze({ added, removed });
    if (generation !== oldGeneration || current !== previous || trusted !== previouslyTrusted || disposed) throw new Error('Workspace publication became stale during normalization');
    // Every allocation/getter/URI normalization finished before live bindings
    // change. No native models or filesystem state are owned by this module.
    for (const entry of next?.entries || []) {
      entry.binding.name = entry.name; entry.binding.index = entry.index;
    }
    current = next; trusted = nextTrusted; generation = nextGeneration;
    if (emit && oldGeneration >= 0) {
      // Stable freezes the event object, not its arrays. Arrays are independent
      // snapshots; mutating them cannot change the private folder inventory.
      changed.fire(notification);
      // A synchronous metadata listener may publish a newer workspace or
      // dispose this scope. Never grant the retired outer snapshot afterward.
      if (!previouslyTrusted && nextTrusted && generation === nextGeneration && trusted && !disposed) granted.fire(undefined);
    }
    return true;
  }
  return Object.freeze({ api, publish, get generation() { return generation; }, dispose() { disposed = true; changed.dispose(); granted.dispose(); } });
}

module.exports = { createWorkspaceScope, MAX_FOLDERS, MAX_URI_BYTES, MAX_NAME_BYTES, MAX_TOTAL_BYTES, MAX_METADATA_BYTES };
