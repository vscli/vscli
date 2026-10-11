'use strict';
// Runtime-private framing lifecycle; never exported through the vscode facade.
// A timed-out save may have committed. Retain its occupied request until the
// exact native terminal response, while rejecting its Promise as outcome unknown.
function createNativeRequests(send, origin, timers = globalThis) {
  let nextId = 0;
  const pending = new Map();
  function request(method, params, lifecycle) {
    if (pending.size >= 64) return Promise.reject(new Error('Extension request limit exceeded'));
    if (!Number.isSafeInteger(nextId + 1)) return Promise.reject(new Error('Extension request identity exhausted'));
    const save = method === 'nativeDocumentSave';
    const context = origin();
    if (method === 'prompt' || ['nativeDocumentOpen', 'nativeDocumentShow', 'nativeDocumentSave', 'nativeCommand', 'nativeStateWrite'].includes(method)) {
      if (context?.active && Number.isSafeInteger(context.id)) params = { ...params, command: context.id, commandOwner: context.owner };
    }
    const id = `host-${++nextId}`;
    // Only internal document-services supplies this exact callback object.
    if (save && lifecycle) lifecycle.retained = true;
    return new Promise((resolve, reject) => {
      const call = { resolve, reject, save, terminal: save ? lifecycle?.onTerminal : undefined, timer: undefined };
      function release() {
        if (pending.get(id) !== call) return;
        pending.delete(id); timers.clearTimeout(call.timer);
        call.terminal?.();
      }
      call.release = release;
      pending.set(id, call);
      try {
        call.timer = timers.setTimeout(() => {
          if (pending.get(id) !== call) return;
          if (!save) release();
          reject(new Error(save ? 'Native document save acknowledgement timed out; save outcome is unknown' : `Native request timed out: ${method}`));
        }, save || method === 'prompt' ? 360000 : 5000);
        send({ id, method, params });
      }
      catch (error) { release(); reject(error); }
    });
  }
  function settle(message) {
    const call = pending.get(message.id);
    if (!call) return false;
    const hasResult = Object.hasOwn(message, 'result'), hasError = Object.hasOwn(message, 'error');
    // Malformed frames cannot free an actual occupied save or restart its timer.
    if (call.save && (hasResult === hasError || (hasResult && typeof message.result !== 'boolean') ||
        (hasError && (!message.error || typeof message.error.message !== 'string')))) return false;
    call.release();
    if (hasError) call.reject(new Error(message.error.message)); else call.resolve(message.result);
    return true;
  }
  function shutdown() {
    for (const call of [...pending.values()]) {
      call.release(); call.reject(new Error('Extension host stopped; any authorized native save outcome is unknown'));
    }
  }
  return { request, settle, shutdown };
}
module.exports = { createNativeRequests };
