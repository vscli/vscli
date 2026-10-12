'use strict';
// Native gateway adaptation of VS Code 1.141.0 (2a59476c9bfcb90b3ddc372c36762471b7dfad1c)
// extHostBulkEdits.ts and extHostTypeConverters.WorkspaceEdit.from. Value objects
// preserve upstream ordered entries; only native grouped text transactions are
// admitted here. This module performs no filesystem writes or document edits.
const { Uri } = require('./api-types.cjs');
const { captureWorkspaceEntries } = require('./edit-values.cjs');
const coordinates = require('./coordinates.cjs');
const createUri = Uri.from, formatUri = Uri.prototype.toString;
const MAX_TARGETS = 128, MAX_EDITS = 4096, MAX_RAW = 1024 * 1024, MAX_WIRE = 2 * MAX_RAW;
function isApplyEditResult(value, expectedTargets) {
  if(!value||typeof value!=='object'||Array.isArray(value)||typeof value.applied!=='boolean'||Object.keys(value).some(key=>!['applied','reason',...(expectedTargets?['targets']:[])].includes(key))||value.reason!==undefined&&(typeof value.reason!=='string'||value.reason.length>4096||Buffer.byteLength(value.reason)>4096))return false;
  if(!expectedTargets)return true;
  if(!value.applied)return value.targets===undefined;
  if(!Array.isArray(value.targets)||value.targets.length!==expectedTargets.size||value.targets.length>128)return false;
  const ids=new Set(),uris=new Set();
  for(const target of value.targets){
    if(!target||typeof target!=='object'||Array.isArray(target)||Object.keys(target).length!==2||Object.keys(target).some(key=>!['uri','document'].includes(key))||typeof target.uri!=='string'||target.uri.length>4096||Buffer.byteLength(target.uri)>4096||!expectedTargets.has(target.uri)||!Number.isSafeInteger(target.document)||target.document<=0||ids.has(target.document)||uris.has(target.uri))return false;
    const original=expectedTargets.get(target.uri);
    if(original!==undefined&&original!==target.document)return false;
    ids.add(target.document);uris.add(target.uri);
  }
  return true;
}
function captureMirrors(model) {
  const result = new Map(), ids = new Set();
  // The iterable and its records are private runtime data, not public documents.
  // Never inspect document._snapshot, .version or .uri to obtain authority.
  for (const value of model.snapshots()) {
    if (result.size >= MAX_TARGETS) throw new Error('Workspace edit mirrored targets exceed 128');
    const { document, version, uri, model: identity } = value;
    if (!Number.isSafeInteger(document) || document <= 0 || !Number.isSafeInteger(version) || version <= 0
        || typeof uri !== 'string' || uri.length > 4096 || Buffer.byteLength(uri) > 4096 || !identity
        || result.has(uri) || ids.has(document)) throw new Error('Invalid or ambiguous native workspace edit mirror');
    result.set(uri, Object.freeze({ document, version, uri, model: identity })); ids.add(document);
  }
  return result;
}
function uriKey(value) {
  if (!(value instanceof Uri)) throw new TypeError('Workspace edit requires a URI');
  // Capture components once and format a new trusted URI. User toString/toJSON
  // overrides and cached _formatted values cannot select a different document.
  const scheme = value.scheme, authority = value.authority, path = value.path,
    query = value.query, fragment = value.fragment;
  let length = 0;
  for (const part of [scheme, authority, path, query, fragment]) {
    if (typeof part !== 'string' || part.length > 4096) throw new TypeError('Invalid workspace edit URI component');
    length += part.length;
    if (length > 4096) throw new Error('Workspace edit URI exceeds 4096 bytes');
  }
  const copy = Reflect.apply(createUri, Uri, [{ scheme, authority, path, query, fragment }, true]);
  const key = Reflect.apply(formatUri, copy, []);
  if (key.length > 4096 || Buffer.byteLength(key) > 4096) throw new Error('Workspace edit URI exceeds 4096 bytes');
  return key;
}
function createWorkspaceEdits(call, options, model) {
  function forOwner(owner) {
    async function applyEdit(value, metadata) {
      // Capture the exact caller lease and all native mirror IDs/versions before
      // reading a single extension-owned edit, URI, coordinate or metadata getter.
      const guard = model.captureCaller(owner);
      guard();
      const mirrors = captureMirrors(model), policy = model.refactoringPolicy();
      let captured;
      try { captured = captureWorkspaceEntries(value); }
      catch (error) {
        if (error instanceof TypeError && error.message === 'WorkspaceEdit receiver is invalid') {
          // Upstream converts an unknown brand to an empty DTO, then STILL reads
          // metadata. Its getter errors and caller retirement must not disappear.
          captured = { generation: 0, entries: [], current: () => 0 };
        } else throw error;
      }
      const expected = captured.generation, groups = new Map();
      let raw = 0, count = 0, unsupported;
      // Trusted captured entries are a fixed, bounded array of private records.
      if (captured.entries.length > MAX_EDITS) throw new Error('Workspace edit count exceeds 4096');
      for (const entry of captured.entries) {
        if (entry.type !== 'text') { unsupported ||= `${entry.type} workspace edits require a native backend`; continue; }
        if (entry.metadata !== undefined) { unsupported ||= 'Workspace edit annotations require a native backend'; continue; }
        const key = uriKey(entry.uri), target = mirrors.get(key);
        if(!target){
          const parsed=Uri.parse(key,true);
          if(!model.unopenedEdits||parsed.scheme!=='file'||parsed.query||parsed.fragment||parsed.authority&&parsed.authority!=='localhost'){unsupported ||= 'Workspace edits require a mirrored document or negotiated local file backend';continue;}
        }
        const edit = entry.edit;
        if (!edit || typeof edit !== 'object') throw new TypeError('Invalid workspace text edit');
        const range = edit.range, newText = edit.newText, eol = edit.newEol,
          snippet = edit.snippet, annotation = edit.annotationId;
        if (eol !== undefined || snippet !== undefined || annotation !== undefined) {
          unsupported ||= 'Snippet, end-of-line and annotated edits require a native backend'; continue;
        }
        if (typeof newText !== 'string') throw new TypeError('Workspace edit text must be a string');
        if (newText.length > MAX_RAW - raw) throw new Error('Workspace edit text exceeds 1 MiB');
        if (/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u.test(newText)) {
          throw new TypeError('Native workspace edit text requires valid Unicode scalars');
        }
        raw += Buffer.byteLength(newText);
        if (raw > MAX_RAW || ++count > MAX_EDITS) throw new Error('Workspace edit text or count exceeds budget');
        const normalized = { range: coordinates.range(range, 'Workspace edit'), newText };
        let group = groups.get(key);
        if (!group) {
          if (groups.size >= MAX_TARGETS) throw new Error('Workspace edit targets exceed 128');
          group = { ...(target?{document:target.document,version:target.version}:{}), uri: key, edits: [] };
          groups.set(key, group);
        }
        group.edits.push(normalized);
      }
      // Pinned ExtHostBulkEdits reads optional metadata after conversion without
      // an object/type guard. Its downstream autosave condition is JS truthiness.
      // The native typed flag expresses that effect without user valueOf/toJSON.
      const isRefactoring = !!(metadata?.isRefactoring ?? false);
      // Pinned bulk-edit autosave runs only for multiple affected resources.
      // A one-resource refactoring uses the ordinary unsaved text transaction.
      if (isRefactoring && policy.autoSave === true && groups.size > 1) unsupported ||= 'Refactoring automatic save requires a native backend; disable files.refactoring.autoSave to apply text edits';
      const args = { entries: [...groups.values()], metadata: { isRefactoring }, ...(model.unopenedEdits?{targetReceipts:true}:{}) };
      const expectedTargets=model.unopenedEdits?new Map([...groups.values()].map(group=>[group.uri,group.document])):undefined;
      // Measurement sees only our primitive DTO, never user toJSON/getters.
      const serialized = JSON.stringify(args);
      if (serialized.length > MAX_WIRE || Buffer.byteLength(serialized) > MAX_WIRE) throw new Error('Workspace edit wire payload exceeds 2 MiB');
      function assertCurrent() {
        guard();
        if (captured.current() !== expected) throw new Error('Workspace edit changed during normalization');
        if (model.refactoringPolicy().identity !== policy.identity) throw new Error('Workspace edit configuration changed during capture');
        for (const group of groups.values()) {
          const original = mirrors.get(group.uri);
          if(!original){if([...model.snapshots()].some(value=>value.uri===group.uri))throw new Error('Workspace edit unopened target became mirrored during capture');continue;}
          const current = model.snapshot(group.document);
          if (!current || current.document !== original.document || current.model !== original.model || current.uri !== original.uri
              || !Number.isSafeInteger(current.version) || current.version <= 0
              || current.version !== original.version) {
            throw new Error('Workspace edit document identity or version changed');
          }
        }
      }
      assertCurrent();
      if (!args.entries.length && captured.entries.length === 0) return false;
      if (unsupported) {
        options.notice?.(owner, unsupported);
        guard();
        return false;
      }
      if (!args.entries.length) return false;
      const result = await call(owner, 'nativeWorkspaceApplyEdit', args);
      // Original caller retirement rejects without reacquiring a renewed lease.
      guard();
      if (!isApplyEditResult(result,expectedTargets)) throw new Error('Invalid native workspace edit acknowledgement');
      // Do not require the edit object/config to stay immutable during an async
      // request. The submitted immutable DTO is authoritative after dispatch.
      // The exact mirror model/URI must survive; own native edits may advance it.
      if (result.applied) for (const group of groups.values()) {
        const id=expectedTargets?result.targets.find(target=>target.uri===group.uri).document:group.document;
        const original=mirrors.get(group.uri)??[...mirrors.values()].find(value=>value.document===id);
        const current=model.snapshot(id);
        if(!current||current.document!==id||!current.model||!Number.isSafeInteger(current.version)||current.version<=0||current.isClosed)throw new Error('Native workspace edit mirrors were not published before acknowledgement');
        if(original){
          if(current.document!==original.document||current.model!==original.model||current.uri!==original.uri||current.version<original.version)throw new Error('Native workspace edit mirror identity changed before acknowledgement');
        }else{
          const parsed=Uri.parse(current.uri,true);
          if(parsed.scheme!=='file'||parsed.query||parsed.fragment||current.isReadonly)throw new Error('Native unopened target mirror is not a writable local file');
        }
      }
      if (result.reason) options.notice?.(owner, result.reason);
      guard();
      return result.applied;
    }
    return Object.freeze({ applyEdit });
  }
  return Object.freeze({ forOwner });
}
module.exports = { createWorkspaceEdits, isApplyEditResult };
