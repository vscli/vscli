'use strict';
const { isApplyEditResult } = require('./workspace-edits.cjs');
const { isFindFilesResult } = require('./workspace-files.cjs');
// Runtime-private framing lifecycle; never exported through the vscode facade.
// A timed-out save or edit may have committed. Retain its occupied request until
// the exact native terminal response, rejecting its Promise as outcome unknown.
function createNativeRequests(send, origin, timers = globalThis) {
  let nextId = 0, serviceBytes = 0;
  const pending = new Map();
  function request(method, params, lifecycle) {
    if (pending.size >= 64) return Promise.reject(new Error('Extension request limit exceeded'));
    if (!Number.isSafeInteger(nextId + 1)) return Promise.reject(new Error('Extension request identity exhausted'));
    const save = method === 'nativeDocumentSave', applyEdit = method === 'nativeWorkspaceApplyEdit';
    const findFiles = method === 'nativeFindFiles';
    const refactoringSave=applyEdit&&params.args?.refactoringSave===true&&params.args?.metadata?.isRefactoring===true;
    const applyTargets=applyEdit&&params.args?.targetReceipts===true?new Map(params.args.entries.map(entry=>[entry.uri,entry.document])):undefined;
    const tabsClose = method === 'nativeTabsClose';
    const virtual = method === 'nativeVirtualDocumentOpen' || method === 'nativeVirtualDocumentChange' || method === 'nativeUntitledDocumentOpen' || method === 'nativeDocumentLanguage' || method === 'nativeEditorSelections';
    const editorOperation = method === 'nativeEditorSelections' || method === 'nativeDocumentShow' || method === 'nativeEditorOptions' || method === 'nativeEditorReveal' || method === 'nativeEditorDecorations' || method === 'nativeEditorInsertSnippet';
    const load = method === 'nativeDocumentLoad';
    const retained = load || save || applyEdit || findFiles || virtual || editorOperation || tabsClose;
    const expectedTabKind = tabsClose ? params.args?.kind : undefined;
    const expectedTabIds = tabsClose && Array.isArray(params.args?.ids) ? params.args.ids.slice() : undefined;
    const expectedEditor = method !== 'nativeDocumentShow' && editorOperation ? params.args?.editor : undefined;
    const decoration = method === 'nativeEditorDecorations', snippetOperation = method === 'nativeEditorInsertSnippet';
    const expectedKey = decoration ? params.args?.key : undefined, expectedDocument = decoration || snippetOperation ? params.args?.document : undefined;
    const context = origin();
    if (method === 'prompt' || ['nativeDocumentLoad', 'nativeDocumentOpen', 'nativeDocumentShow', 'nativeDocumentSave', 'nativeWorkspaceApplyEdit', 'nativeFindFiles', 'nativeVirtualDocumentOpen', 'nativeVirtualDocumentChange', 'nativeUntitledDocumentOpen', 'nativeDocumentLanguage', 'nativeEditorSelections', 'nativeEditorOptions', 'nativeEditorReveal', 'nativeEditorDecorations', 'nativeEditorInsertSnippet', 'nativeTabsClose', 'nativeCommand', 'nativeStateWrite'].includes(method)) {
      if (context?.active && Number.isSafeInteger(context.id)) params = { ...params, command: context.id, commandOwner: context.owner };
    }
    const id = `host-${++nextId}`;
    const frame = { id, method, params };
    let bytes = 0;
    if (['nativeDocumentLoad', 'nativeDocumentOpen', 'nativeDocumentShow', 'nativeDocumentSave', 'nativeWorkspaceApplyEdit', 'nativeFindFiles', 'nativeVirtualDocumentOpen', 'nativeVirtualDocumentChange', 'nativeUntitledDocumentOpen', 'nativeDocumentLanguage', 'nativeEditorSelections', 'nativeEditorOptions', 'nativeEditorReveal', 'nativeEditorDecorations', 'nativeEditorInsertSnippet', 'nativeTabsClose', 'nativeCommand'].includes(method)) {
      try {
        const encoded = JSON.stringify(frame);
        if (encoded.length > 2 * 1024 * 1024) throw new Error('Native document service wire payload limit reached');
        bytes = Buffer.byteLength(encoded);
        if (bytes > 2 * 1024 * 1024 || bytes > 8 * 1024 * 1024 - serviceBytes) throw new Error('Native document service wire payload limit reached');
      } catch (error) { return Promise.reject(error); }
    }
    // Only internal document-services supplies this exact callback object.
    if (retained && lifecycle) lifecycle.retained = true;
    return new Promise((resolve, reject) => {
      const call = { resolve, reject, save, applyEdit, applyTargets, refactoringSave, findFiles, virtual, load, editorOperation, expectedEditor, expectedKey, expectedDocument, decoration, snippetOperation, tabsClose, expectedTabKind, expectedTabIds, bytes, terminal: retained ? lifecycle?.onTerminal : undefined, timer: undefined };
      function release() {
        if (pending.get(id) !== call) return;
        pending.delete(id); serviceBytes -= call.bytes; timers.clearTimeout(call.timer);
        call.terminal?.();
      }
      call.release = release;
      pending.set(id, call); serviceBytes += bytes;
      try {
        call.timer = timers.setTimeout(() => {
          if (pending.get(id) !== call) return;
          if (!retained) release();
          reject(new Error(tabsClose ? 'Native tab close timed out; outcome is unknown' : save ? 'Native document save acknowledgement timed out; save outcome is unknown' : applyEdit ? 'Native workspace edit acknowledgement timed out; edit outcome is unknown' : findFiles ? 'Native findFiles acknowledgement timed out; search outcome is unknown' : load || virtual || editorOperation ? 'Native generated document acknowledgement timed out; publication outcome is unknown' : `Native request timed out: ${method}`));
        }, save || refactoringSave || tabsClose || method === 'prompt' ? 360000 : 5000);
        send(frame);
      }
      catch (error) { release(); reject(error); return; }
      // Accepted work cannot be unoccupied merely because its post-send
      // cancellation hook failed. Only the exact terminal reply releases it.
      try { lifecycle?.onRequest?.(id); }
      catch (error) { if (!retained) release(); reject(error); }
    });
  }
  function settle(message) {
    const call = pending.get(message.id);
    if (!call) return false;
    const hasResult = Object.hasOwn(message, 'result'), hasError = Object.hasOwn(message, 'error');
    if (call.tabsClose && (hasResult === hasError || (hasResult && (!message.result ||
        Array.isArray(message.result) || Object.keys(message.result).length !== 3 ||
        typeof message.result.closed !== 'boolean' || message.result.kind !== call.expectedTabKind ||
        !Array.isArray(message.result.ids) || !call.expectedTabIds ||
        message.result.ids.length !== call.expectedTabIds.length ||
        message.result.ids.some((id,index)=>id !== call.expectedTabIds[index]))) ||
        (hasError && (!message.error || typeof message.error.message !== 'string' ||
          message.error.message.length > 4096 || Buffer.byteLength(message.error.message) > 4096)))) return false;
    // Malformed frames cannot free occupied Save/ApplyEdit work or restart its timer.
    if (call.save && (hasResult === hasError || (hasResult && typeof message.result !== 'boolean') ||
        (hasError && (!message.error || typeof message.error.message !== 'string')))) return false;
    if (call.applyEdit && (hasResult === hasError || (hasResult && !isApplyEditResult(message.result,call.applyTargets,call.refactoringSave)) ||
        (hasError && (!message.error || typeof message.error.message !== 'string'
          || message.error.message.length > 4096 || Buffer.byteLength(message.error.message) > 4096)))) return false;
    if (call.findFiles && (hasResult === hasError || (hasResult && !isFindFilesResult(message.result)) ||
        (hasError && (!message.error || typeof message.error.message !== 'string'
          || message.error.message.length > 4096 || Buffer.byteLength(message.error.message) > 4096)))) return false;
    if (call.snippetOperation && (hasResult === hasError || (hasResult && (!message.result ||
        Object.keys(message.result).length !== 3 || message.result.document !== call.expectedDocument ||
        message.result.editor !== call.expectedEditor || typeof message.result.applied !== 'boolean')) ||
        (hasError && (!message.error || typeof message.error.message !== 'string' ||
          message.error.message.length > 4096 || Buffer.byteLength(message.error.message) > 4096)))) return false;
    if (!call.snippetOperation && (call.load || call.virtual || call.editorOperation) && (hasResult === hasError || (hasResult && (!message.result ||
        !Number.isSafeInteger(message.result.document) || message.result.document <= 0 || (call.decoration && message.result.document !== call.expectedDocument) ||
        (Object.hasOwn(message.result,'editor') ? (!call.editorOperation || Object.keys(message.result).length !== (call.decoration ? 3 : 2) || (call.decoration && (typeof message.result.key !== 'string' || !message.result.key || message.result.key !== call.expectedKey)) || typeof message.result.editor !== 'string' || !message.result.editor || message.result.editor.length > 128 || Buffer.byteLength(message.result.editor) > 128 || (call.expectedEditor !== undefined && message.result.editor !== call.expectedEditor)) : (Object.keys(message.result).length !== 1 || call.expectedEditor !== undefined)))) ||
        (hasError && (!message.error || typeof message.error.message !== 'string'
          || message.error.message.length > 4096 || Buffer.byteLength(message.error.message) > 4096)))) return false;
    call.release();
    if (hasError) call.reject(new Error(message.error.message)); else call.resolve(message.result);
    return true;
  }
  function shutdown() {
    for (const call of [...pending.values()]) {
      call.release(); call.reject(new Error('Extension host stopped; any authorized native save or workspace edit outcome is unknown'));
    }
  }
  return { request, settle, shutdown };
}
module.exports = { createNativeRequests };
