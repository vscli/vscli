'use strict';
// Only normalized, bounded display data crosses the native boundary. Original
// SignatureHelp objects (including opaque extension data) remain in this host.
const { SignatureHelp, SignatureInformation, ParameterInformation, MarkdownString } = require('./provider-types.cjs');
const MAX_BYTES = 256 * 1024, MAX_CACHE_BYTES = 2 * 1024 * 1024, MAX_HANDLES = 32;
const HANDLE = '_vscliSignatureHelpHandle';

function array(value, maximum) {
  if (!Array.isArray(value)) throw new TypeError('Signature value must be an array');
  const count = value.length;
  if (!Number.isSafeInteger(count) || count < 0 || count > maximum) throw new Error(`Signature array exceeds ${maximum} entries`);
  const result = [];
  for (let index = 0; index < count; index++) result.push(value[index]);
  if (value.length !== count) throw new Error('Signature array changed during validation');
  return result;
}
function string(value, maximum = 8192) {
  if (typeof value !== 'string' || Buffer.byteLength(value) > maximum) throw new Error('Signature string exceeds its budget or has invalid type');
  return value;
}
function index(value) {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error('Invalid signature active index');
  return value;
}
function characters(value) {
  return array(value, 16).map(value => {
    string(value, 4);
    if ([...value].length !== 1) throw new Error('Invalid signature trigger character');
    return value;
  });
}
function documentation(value) {
  if (typeof value === 'string') return string(value);
  if (!value || typeof value !== 'object') throw new Error('Invalid signature documentation');
  const trusted = value.isTrusted, html = value.supportHtml, kind = value.kind, content = value.value;
  if (trusted || html) throw new Error('Trusted or HTML signature markdown is not supported');
  if (kind !== undefined && kind !== 'markdown' && kind !== 'plaintext') throw new Error('Invalid signature documentation kind');
  return { kind: kind || 'markdown', value: string(content) };
}
function boundary(label, offset) {
  if (offset < 0 || offset > label.length) throw new Error('Signature parameter offset is outside its label');
  if (offset > 0 && offset < label.length && /[\uD800-\uDBFF]/.test(label[offset - 1]) && /[\uDC00-\uDFFF]/.test(label[offset])) throw new Error('Signature parameter offset splits UTF-16');
}
function normalize(value) {
  if (value === undefined || value === null) return null;
  if (typeof value !== 'object') throw new Error('Invalid signature help');
  let bytes = 64, labels = 0;
  function charge(value, overhead = 32) {
    bytes += overhead + (typeof value === 'string' ? Buffer.byteLength(value) : Buffer.byteLength(value.value));
    if (bytes > MAX_BYTES) throw new Error('Signature help exceeds 256 KiB');
    return value;
  }
  const rawSignatures = value.signatures, activeSignature = value.activeSignature, activeParameter = value.activeParameter;
  const result = { signatures: [], activeSignature: index(activeSignature === undefined ? 0 : activeSignature), activeParameter: index(activeParameter === undefined ? 0 : activeParameter) };
  for (const raw of array(rawSignatures, 32)) {
    if (!raw || typeof raw !== 'object') throw new Error('Invalid signature information');
    const rawLabel = raw.label, rawDocs = raw.documentation, rawParameters = raw.parameters, rawActive = raw.activeParameter;
    const label = charge(string(rawLabel), 64);
    labels += Buffer.byteLength(label);
    if (labels > 65536) throw new Error('Signature labels exceed 64 KiB');
    const signature = { label, parameters: [] };
    if (rawDocs !== undefined) signature.documentation = charge(documentation(rawDocs));
    if (rawActive !== undefined) signature.activeParameter = index(rawActive);
    for (const rawParameter of array(rawParameters === undefined ? [] : rawParameters, 128)) {
      if (!rawParameter || typeof rawParameter !== 'object') throw new Error('Invalid signature parameter information');
      const rawLabel = rawParameter.label, rawDocs = rawParameter.documentation;
      let label;
      if (typeof rawLabel === 'string') label = charge(string(rawLabel));
      else {
        label = array(rawLabel, 2);
        if (label.length !== 2 || !label.every(Number.isSafeInteger) || label[1] < label[0]) throw new Error('Invalid signature parameter offset pair');
        boundary(signature.label, label[0]); boundary(signature.label, label[1]);
        bytes += 64;
        if (bytes > MAX_BYTES) throw new Error('Signature help exceeds 256 KiB');
      }
      const parameter = { label };
      if (rawDocs !== undefined) parameter.documentation = charge(documentation(rawDocs));
      signature.parameters.push(parameter);
    }
    result.signatures.push(signature);
  }
  if (Buffer.byteLength(JSON.stringify(result)) > MAX_BYTES) throw new Error('Signature help exceeds 256 KiB');
  return result;
}
function reviveDocumentation(value) {
  if (value === undefined || typeof value === 'string') return value;
  return value.kind === 'plaintext' ? value.value : new MarkdownString(value.value);
}
function revive(value) {
  const result = new SignatureHelp();
  result.activeSignature = value.activeSignature; result.activeParameter = value.activeParameter;
  result.signatures = value.signatures.map(value => {
    const signature = new SignatureInformation(value.label, reviveDocumentation(value.documentation));
    signature.parameters = value.parameters.map(value => new ParameterInformation(value.label, reviveDocumentation(value.documentation)));
    if (value.activeParameter !== undefined) signature.activeParameter = value.activeParameter;
    return signature;
  });
  return result;
}
function createSignatures(options) {
  const retained = new Map(); let nextHandle = 0, occupied;
  function current(cached) {
    return options.registered(cached.entry, cached.epoch) && !cached.document.isClosed &&
      options.document(cached.documentId) === cached.document && cached.document.uri.toString() === cached.uri;
  }
  function purge() { for (const [handle, cached] of retained) if (!current(cached)) retained.delete(handle); }
  function reserve(call, request) {
    if (occupied) throw new Error('A signature callback is already running');
    occupied = { call, request };
  }
  function released(params, call) {
    if (call && occupied?.call === call) occupied = undefined;
    if (params.session === options.session && Number.isSafeInteger(params.request) && params.request > 0 && typeof params.owner === 'string' && Number.isSafeInteger(params.provider)) {
      options.notify('signatureReleased', { session: options.session, owner: params.owner, provider: params.provider, request: params.request });
    }
  }
  function args(params, call) {
    const raw = params.signatureContext;
    if (raw !== undefined && raw !== null && (typeof raw !== 'object' || Object.keys(raw).some(key => !['triggerKind', 'triggerCharacter', 'isRetrigger', 'activeSignatureHelp'].includes(key)))) throw new Error('Unsupported signature context');
    const rawTrigger = raw?.triggerKind, rawRetrigger = raw?.isRetrigger, triggerCharacter = raw?.triggerCharacter, rawHelp = raw?.activeSignatureHelp;
    const triggerKind = rawTrigger === undefined ? 1 : rawTrigger, isRetrigger = rawRetrigger === undefined ? false : rawRetrigger;
    if (![1, 2, 3].includes(triggerKind) || typeof isRetrigger !== 'boolean') throw new Error('Invalid signature context');
    if (triggerCharacter !== undefined) characters([triggerCharacter]);
    if (triggerKind === 2 && triggerCharacter === undefined) throw new Error('Signature trigger character is missing');
    const context = { triggerKind, isRetrigger, ...(triggerCharacter === undefined ? {} : { triggerCharacter }) };
    if (rawHelp !== undefined && rawHelp !== null) {
      const handle = rawHelp[HANDLE], normalized = normalize(rawHelp);
      options.assertCurrent(call); purge();
      if (handle !== undefined) {
        if (!Number.isSafeInteger(handle) || handle < 1) throw new Error('Invalid signature help handle');
        const cached = retained.get(handle);
        if (!cached || cached.entry !== call.entry || cached.document !== call.document || cached.epoch !== call.epoch || !current(cached)) throw new Error('Stale signature help handle, owner, or document');
        // The pinned VS Code adapter restores this original object and updates
        // only the two indices selected by the editor. Never serialize its data.
        cached.original.activeSignature = normalized.activeSignature;
        options.assertCurrent(call);
        if (retained.get(handle) !== cached || !current(cached)) throw new Error('Signature help became stale while restoring context');
        cached.original.activeParameter = normalized.activeParameter;
        options.assertCurrent(call);
        if (retained.get(handle) !== cached || !current(cached)) throw new Error('Signature help became stale while restoring context');
        context.activeSignatureHelp = cached.original;
      } else context.activeSignatureHelp = revive(normalized);
    }
    options.assertCurrent(call);
    return [call.document, options.position(params.position, call.document), call.source.token, context];
  }
  function retain(call, original, result, request) {
    if (!result || !result.signatures.length) return;
    purge(); options.assertCurrent(call);
    if (nextHandle >= Number.MAX_SAFE_INTEGER) throw new Error('Signature help handle exhausted');
    const handle = nextHandle + 1;
    result[HANDLE] = handle;
    const bytes = Buffer.byteLength(JSON.stringify(result));
    if (bytes > MAX_BYTES) throw new Error('Signature help exceeds 256 KiB');
    let total = bytes;
    for (const cached of retained.values()) total += cached.bytes;
    if (retained.size >= MAX_HANDLES || total > MAX_CACHE_BYTES) throw new Error('Signature help cache budget exceeded');
    const cached = { entry: call.entry, epoch: call.epoch, document: call.document, documentId: call.document._snapshot.id, uri: call.document.uri.toString(), original, request, bytes };
    options.assertCurrent(call);
    if (!current(cached)) throw new Error('Signature help became stale before publication');
    options.assertCurrent(call);
    if (!current(cached)) throw new Error('Signature help became stale before retaining its handle');
    nextHandle = handle;
    retained.set(handle, cached);
  }
  function retire(params) {
    if (params.session !== options.session) return false;
    for (const [handle, cached] of retained) {
      if (cached.entry.owner !== params.owner) continue;
      if (params.releaseSignatureHelp === true) {
        if (handle === params.signatureHandle && cached.request === params.request && cached.entry.id === params.provider && cached.documentId === params.document) retained.delete(handle);
      } else if (cached.request === params.request) retained.delete(handle);
    }
    return true;
  }
  return { args, retain, retire, reserve, released, purge, clear: () => retained.clear(), count: () => { purge(); return retained.size; }, pending: () => !!occupied };
}
module.exports = { normalize, createSignatures, characters, HANDLE };
