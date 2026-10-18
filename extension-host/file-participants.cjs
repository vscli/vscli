'use strict';
// Copyright (c) Microsoft Corporation. All rights reserved.
// Source-guided from Microsoft VS Code 1.141.0 (MIT, license in edit-types.cjs):
// extHostFileSystemEventService._fireWillEvent and event.AsyncEmitter.fireAsync,
// immutable commit 2a59476c9bfcb90b3ddc372c36762471b7dfad1c.
// Native adaptations: original caller leases, bounded queues/DTOs and separate
// logical response vs actual completion. This module performs no native edits.
const { Disposable } = require('./api-types.cjs');
const { CancellationToken } = require('./cancellation.cjs');
const { WorkspaceEdit } = require('./edit-values.cjs');
const { snapshotWorkspaceEdit } = require('./workspace-edit-values.cjs');
const { reviveFileOperation } = require('./file-operations.cjs');
const MAX_LISTENERS = 1024, MAX_WAITS = 4096, MAX_QUEUE_BYTES = 1024 * 1024;
const MAX_EDITS = 4096, MAX_RESOURCES = 128, MAX_RAW = 1024 * 1024, MAX_WIRE = 2 * MAX_RAW;
const methods = {create:'onWillCreateFiles',rename:'onWillRenameFiles',delete:'onWillDeleteFiles'};

function createFileParticipants(options) {
  const listeners = {create:new Set(),rename:new Set(),delete:new Set()};
  // AsyncEmitter retains undelivered entries after cancellation. Do not silently
  // discard that source behavior, or substitute a new data object for an entry.
  const queues = {create:[],rename:[],delete:[]};
  let registrations = 0, queuedBytes = 0, sequence = 0, active;
  const report = options.report || (error => console.error(error));
  function diagnostic(error) { try { report(error); } catch { /* Logging cannot change delivery. */ } }
  function forOwner(owner) {
    const api = {};
    for (const [kind, name] of Object.entries(methods)) api[name] = (listener, thisArg, disposables) => {
      const guard = options.captureCaller(owner); guard();
      if (typeof listener !== 'function') throw new TypeError('File participant listener requires a function');
      if (registrations >= MAX_LISTENERS) throw new Error('File participant listener limit reached');
      // Capture once, including the scope used to assimilate extension thenables.
      const invoke = options.bind(owner, callback => callback());
      const record = {owner,guard,invoke,listener,thisArg};
      listeners[kind].add(record); registrations++;
      const disposable = new Disposable(() => {
        if (listeners[kind].delete(record)) registrations--;
      });
      options.track?.(owner, disposable);
      if (Array.isArray(disposables)) disposables.push(disposable);
      return disposable;
    };
    return Object.freeze(api);
  }
  function release(run) {
    if (run.deliveryDone && run.actual.size === 0 && active === run) {
      active = undefined; run.finish();
    }
  }
  function track(run, promise) {
    run.actual.add(promise);
    promise.then(() => {run.actual.delete(promise);release(run);}, () => {run.actual.delete(promise);release(run);});
    return promise;
  }
  function combine(edits) {
    const result = {edits:[]}, resources = new Set();
    let raw = 0, wireBytes = 0;
    for (const [record, value] of edits) {
      // Conversion happens AFTER listener delivery and all awaited contributions.
      // Each edit gets its own create-resource version suppression set.
      record.guard();
      const dto = record.invoke(() => snapshotWorkspaceEdit(value, record.guard, options.versionInfo()));
      wireBytes += Buffer.byteLength(JSON.stringify(dto));
      if (wireBytes > MAX_WIRE) throw new RangeError('Combined file participant wire exceeds 2 MiB');
      for (const edit of dto.edits) {
        for (const resource of [edit.resource, edit.oldResource, edit.newResource]) if (resource != null) resources.add(resource);
        if (edit.textEdit) raw += Buffer.byteLength(edit.textEdit.text);
        if (edit.options?.contents?.type === 'base64') raw += Buffer.byteLength(edit.options.contents.value, 'base64');
        for (const cell of edit.cellEdit?.cells || []) {
          raw += Buffer.byteLength(cell.source);
          for (const output of cell.outputs) for (const item of output.items) raw += item.valueBytes.byteLength;
        }
        if (raw > MAX_RAW || resources.size > MAX_RESOURCES || result.edits.length >= MAX_EDITS) throw new RangeError('Combined file participant edits exceed native budget');
        result.edits.push(edit);
      }
      record.guard();
    }
    const encoded = JSON.stringify(result);
    if (encoded.length > MAX_WIRE || Buffer.byteLength(encoded) > MAX_WIRE) throw new RangeError('Combined file participant wire exceeds 2 MiB');
    return result;
  }
  function fire(frame, timeout, token = CancellationToken.None) {
    // Validate the whole frame and admission before consuming sequence or slots.
    const files = reviveFileOperation(frame, options.session);
    if (active) throw new Error('File participant invocation is actually occupied');
    if (frame.sequence <= sequence || !Number.isSafeInteger(timeout) || timeout < 0 || !CancellationToken.isCancellationToken(token)) throw new Error('Invalid file participant admission');
    const cohort = [...listeners[frame.kind]], queue = queues[frame.kind];
    const charge = Buffer.byteLength(JSON.stringify(frame));
    const queueCount = Object.values(queues).reduce((count, entries) => count + entries.length, 0);
    if (queueCount + cohort.length > MAX_LISTENERS || queuedBytes + charge * cohort.length > MAX_QUEUE_BYTES) throw new Error('File participant delivery queue exceeds native budget');
    const data = {files};
    for (const record of cohort) queue.push({record,data,charge});
    queuedBytes += charge * cohort.length; sequence = frame.sequence;
    let finish;
    const settled = new Promise(resolve => {finish = resolve;});
    const run = {actual:new Set(),deliveryDone:false,finish,settled,waits:0}; active = run;
    const edits = [], names = new Set();
    async function deliver() {
      while (queue.length && !token.isCancellationRequested) {
        const {record,data,charge} = queue.shift(); queuedBytes -= charge;
        const thenables = [];
        let threw = false;
        const event = {...data,token,waitUntil(value) {
          if (Object.isFrozen(thenables)) throw new Error('waitUntil can NOT be called asynchronous');
          record.guard();
          if (active !== run || ++run.waits > MAX_WAITS) throw new Error('File participant wait limit or lifetime exceeded');
          const started = Date.now();
          const joined = record.invoke(() => Promise.resolve(value).then(result => {
            record.guard();
            if (result instanceof WorkspaceEdit) {
              const name = options.extensionName?.(record.owner) ?? record.owner;
              if (typeof name !== 'string' || name.length > 1024) throw new TypeError('Invalid file participant extension name');
              edits.push([record,result]);
              names.add(name);
            }
            if (Date.now() - started > timeout) diagnostic(new Error(`SLOW file-participant ${record.owner}`));
          }));
          track(run, joined); thenables.push(joined);
          joined.catch(error => {if (threw) diagnostic(error);});
        }};
        try { record.invoke(() => Reflect.apply(record.listener, record.thisArg, [event])); }
        catch (error) {
          threw = true;
          diagnostic(error);
          // Source skips freezing/awaiting on a synchronous throw. Keep any
          // promises already admitted alive, and observe failures independently.
          continue;
        }
        Object.freeze(thenables);
        const results = await Promise.allSettled(thenables);
        for (const result of results) if (result.status === 'rejected') diagnostic(result.reason);
      }
      if (token.isCancellationRequested || edits.length === 0) return undefined;
      const edit=combine(edits), owners=[...new Set(edits.map(([record])=>record.owner))];
      return {edit,extensionNames:[...names],...(options.nativeSource?{source:options.nativeSource(edit,owners)}:{})};
    }
    const result = deliver().finally(() => {run.deliveryDone = true;release(run);});
    return Object.freeze({result,settled});
  }
  return Object.freeze({forOwner,fire,busy:()=>!!active});
}
module.exports = {createFileParticipants};
