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
    const virtual = method === 'nativeVirtualDocumentOpen' || method === 'nativeVirtualDocumentChange';
    const retained = save || applyEdit || findFiles || virtual;
    const context = origin();
    if (method === 'prompt' || ['nativeDocumentOpen', 'nativeDocumentShow', 'nativeDocumentSave', 'nativeWorkspaceApplyEdit', 'nativeFindFiles', 'nativeVirtualDocumentOpen', 'nativeVirtualDocumentChange', 'nativeCommand', 'nativeStateWrite'].includes(method)) {
      if (context?.active && Number.isSafeInteger(context.id)) params = { ...params, command: context.id, commandOwner: context.owner };
    }
    const id = `host-${++nextId}`;
    const frame = { id, method, params };
    let bytes = 0;
    if (['nativeDocumentOpen', 'nativeDocumentShow', 'nativeDocumentSave', 'nativeWorkspaceApplyEdit', 'nativeFindFiles', 'nativeVirtualDocumentOpen', 'nativeVirtualDocumentChange', 'nativeCommand'].includes(method)) {
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
      const call = { resolve, reject, save, applyEdit, findFiles, virtual, bytes, terminal: retained ? lifecycle?.onTerminal : undefined, timer: undefined };
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
          reject(new Error(save ? 'Native document save acknowledgement timed out; save outcome is unknown' : applyEdit ? 'Native workspace edit acknowledgement timed out; edit outcome is unknown' : findFiles ? 'Native findFiles acknowledgement timed out; search outcome is unknown' : virtual ? 'Native virtual document acknowledgement timed out; publication outcome is unknown' : `Native request timed out: ${method}`));
        }, save || method === 'prompt' ? 360000 : 5000);
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
    // Malformed frames cannot free occupied Save/ApplyEdit work or restart its timer.
    if (call.save && (hasResult === hasError || (hasResult && typeof message.result !== 'boolean') ||
        (hasError && (!message.error || typeof message.error.message !== 'string')))) return false;
    if (call.applyEdit && (hasResult === hasError || (hasResult && !isApplyEditResult(message.result)) ||
        (hasError && (!message.error || typeof message.error.message !== 'string'
          || message.error.message.length > 4096 || Buffer.byteLength(message.error.message) > 4096)))) return false;
    if (call.findFiles && (hasResult === hasError || (hasResult && !isFindFilesResult(message.result)) ||
        (hasError && (!message.error || typeof message.error.message !== 'string'
          || message.error.message.length > 4096 || Buffer.byteLength(message.error.message) > 4096)))) return false;
    if (call.virtual && (hasResult === hasError || (hasResult && (!message.result ||
        Object.keys(message.result).length !== 1 || !Number.isSafeInteger(message.result.document) || message.result.document <= 0)) ||
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
