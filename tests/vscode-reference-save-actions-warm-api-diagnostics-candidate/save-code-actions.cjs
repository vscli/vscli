'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { configurationReadiness, matches, reader, setupBudget, loadCases, boundedSource } = require('./save-configuration-ready.cjs');
const cases = loadCases();
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const TEXT = 'fix=0\r\nimports=0\r\nchild=0\r\nother=0\r\n猫🙂\r\n';
exports.text = TEXT;
const READINESS_RESOURCE = 'readiness.ini';
const READINESS_TEXT = 'readiness=0\r\n';
exports.readinessText = READINESS_TEXT;
// This positive setup witness uses an actual document Save. Executing the
// formatter API directly would not prove save-participant installation.
exports.saveParticipantReadiness = async (vscode, workspace, clock = {}) => {
  const now = clock.now || Date.now, wait = clock.pause || pause;
  const limits = { maxAttempts: 16, deadlineMs: 5000, retryIntervalMs: 250 };
  const start = now();
  let registration, timer, abortListener;
  const partial = { protocol: 'save-participant-readiness-v2', status: 'starting',
    resource: READINESS_RESOURCE, input: READINESS_TEXT, saveReason: 'explicit-auxiliary',
    targetAutoSave: clock.expectedAutoSave || 'off', limits, attempts: [], callbacks: [] };
  clock.report?.(partial);
  const timeout = new Promise((_, reject) => {
    timer = (clock.setTimeout || setTimeout)(() => reject(new Error(
      'Save-participant readiness deadline exceeded')), limits.deadlineMs);
  });
  const aborted = new Promise((_, reject) => {
    abortListener = () => reject(new Error('Auxiliary readiness canceled by setup deadline'));
    if (clock.signal?.aborted) abortListener();
    else clock.signal?.addEventListener('abort', abortListener, { once: true });
  });
  timeout.catch(() => {}); aborted.catch(() => {});
  const bounded = work => Promise.race([work, timeout, aborted]);
  try {
    const file = path.join(workspace, READINESS_RESOURCE);
    const input = fs.readFileSync(file, 'utf8');
    assert.equal(input, READINESS_TEXT, 'Readiness auxiliary input differs');
    const document = await bounded(vscode.workspace.openTextDocument(vscode.Uri.file(file)));
    assert.equal(document.languageId, 'ini', 'Readiness language differs');
    assert.equal(document.isDirty, false, 'Readiness document starts dirty');
    assert.equal(document.getText(), input, 'Readiness model differs from declared input');
    const editorConfig = vscode.workspace.getConfiguration('editor', document);
    const effective = { formatOnSave: editorConfig.get('formatOnSave'),
      formatOnSaveMode: editorConfig.get('formatOnSaveMode'),
      autoSave: vscode.workspace.getConfiguration('files', document).get('autoSave') };
    assert.equal(effective.formatOnSave, true, 'Readiness format-on-save is disabled');
    assert.equal(effective.formatOnSaveMode, 'file', 'Readiness formatting mode differs');
    assert.ok(['off', 'afterDelay'].includes(partial.targetAutoSave), 'Unknown target save reason');
    assert.equal(effective.autoSave, partial.targetAutoSave, 'Readiness target autosave setting differs');
    const state = () => ({ resource: READINESS_RESOURCE, uri: document.uri.toString(), version: document.version,
      text: document.getText(), dirty: document.isDirty, disk: fs.readFileSync(file, 'utf8') });
    const initial = state(), attempts = partial.attempts, callbacks = partial.callbacks;
    Object.assign(partial, { status: 'running', input, effective, initial });
    let saving;
    registration = vscode.languages.registerDocumentFormattingEditProvider(
      { scheme: 'file', language: 'ini', pattern: new vscode.RelativePattern(workspace, READINESS_RESOURCE) },
      { provideDocumentFormattingEdits(doc, _options, token) {
        assert.ok(callbacks.length < limits.maxAttempts, 'Readiness callback budget exceeded');
        const uri = doc.uri.toString();
        assert.ok(typeof uri === 'string' && uri.length <= 4096, 'Readiness callback URI exceeds budget');
        const exact = !!saving && doc === document && uri === document.uri.toString()
          && doc.version === saving.version && doc.getText() === saving.text
          && !token.isCancellationRequested;
        callbacks.push({ attempt: saving?.attempt ?? null, uri, version: doc.version,
          text: doc === document ? doc.getText() : null,
          resource: doc === document ? READINESS_RESOURCE : null,
          exactDocument: doc === document, duringSave: !!saving,
          cancelled: token.isCancellationRequested, matched: exact });
        return null;
      } });
    for (let index = 1; index <= limits.maxAttempts; index += 1) {
      assert.ok(now() - start < limits.deadlineMs, 'Save-participant readiness deadline exceeded');
      const before = state();
      const edit = new vscode.WorkspaceEdit();
      edit.insert(document.uri, document.positionAt(document.getText().length), 'x');
      assert.equal(await bounded(vscode.workspace.applyEdit(edit)), true, 'Readiness edit failed');
      assert.equal(document.isDirty, true, 'Readiness edit did not dirty auxiliary document');
      const prepared = state(), firstCallback = callbacks.length;
      partial.pendingAttempt = { index, before, prepared, callbackStart: firstCallback };
      saving = { attempt: index, version: document.version, text: document.getText() };
      let saved;
      try { saved = await bounded(document.save()); } finally { saving = undefined; }
      const after = state();
      assert.equal(saved, true, 'Readiness auxiliary save failed');
      assert.equal(after.dirty, false, 'Readiness save did not settle');
      assert.equal(after.text, prepared.text, 'Null readiness formatter changed auxiliary text');
      assert.equal(after.disk, after.text, 'Readiness save precedes auxiliary disk commit');
      const matched = callbacks.slice(firstCallback).filter(callback => callback.matched);
      assert.ok(matched.length <= 1, 'Duplicate readiness formatter invocation');
      partial.pendingAttempt = undefined;
      attempts.push({ index, before, prepared, after, callbackStart: firstCallback,
        callbackCount: callbacks.length - firstCallback, matched: matched.length === 1,
        elapsedMs: now() - start });
      assert.ok(now() - start < limits.deadlineMs, 'Save-participant readiness deadline exceeded');
      if (matched.length === 1) {
        Object.assign(partial, { status: 'observed', final: after,
          scope: 'Separate explicit public auxiliary Save and exact null-formatter callback establish save-contribution installation for every target reason; no target output predicate or target command retry' });
        return partial;
      }
      if (index < limits.maxAttempts) {
        const remaining = limits.deadlineMs - (now() - start);
        assert.ok(remaining > 0, 'Save-participant readiness deadline exceeded');
        await bounded(wait(Math.min(limits.retryIntervalMs, remaining)));
      }
    }
    assert.fail('Save-participant readiness attempt budget exceeded');
  } catch (error) {
    partial.status = 'failed'; partial.error = String(error.message || error).slice(0, 1024);
    error.saveParticipantReadiness = partial;
    throw error;
  } finally {
    clock.signal?.removeEventListener('abort', abortListener);
    (clock.clearTimeout || clearTimeout)(timer);
    registration?.dispose();
  }
};
// Shared two-barrier orchestration, also exercised with controlled public API doubles.
exports.stableSaveSetup = async (vscode, workspace, fixture, budget, progress = {}, clock = {}) => {
  const saveParticipantReadiness = await budget.bound(() => exports.saveParticipantReadiness(vscode, workspace,
    { ...clock, expectedAutoSave: fixture.autosave, signal: budget.signal,
      report: evidence => { progress.saveParticipantReadiness = evidence; } }));
  const configurationProof = await budget.bound(() => configurationReadiness(vscode,
    vscode.Uri.file(path.join(workspace, 'main.txt')), fixture,
    { ...clock, signal: budget.signal, report: evidence => { progress.configurationReadiness = evidence; } }));
  progress.configurationReadiness = configurationProof;
  return { saveParticipantReadiness, configurationProof };
};
// Diagnostic-only public configuration evidence. The target fixture and its
// projected observations are unchanged; this recorder never writes settings,
// invokes a target command, retries a target, or waits for preferred output.
exports.configurationDiagnostics = (vscode, targetUri, phase, now = Date.now) => {
  const limits = Object.freeze({ maxRecords: 128, maxValueNodes: 1024,
    maxValueDepth: 8, maxValueBytes: 32768, maxTotalBytes: 524288 });
  const keys = ['editor.codeActionsOnSave', 'editor.formatOnSave', 'files.autoSave', 'files.autoSaveDelay'];
  const uri = targetUri.toString();
  assert.ok(targetUri.scheme === 'file' && typeof uri === 'string' && Buffer.byteLength(uri) <= 4096
    && path.basename(targetUri.fsPath) === 'main.txt',
    'Diagnostic target URI differs');
  const scope = { uri: targetUri, languageId: 'plaintext' };
  const start = now(), records = [];
  let boundDocument, totalBytes = 0, lastElapsed = 0, failure, disposed = false;
  // Clone public values immediately so subsequent migration cannot rewrite old
  // evidence through a shared object. Undefined inspect fields keep the public
  // JSON omission semantics; wholly absent accessor results are explicit null.
  const value = input => {
    let nodes = 0, bytes = 0;
    const seen = new Set();
    const visit = (item, depth) => {
      nodes += 1;
      assert.ok(nodes <= limits.maxValueNodes && depth <= limits.maxValueDepth,
        'Diagnostic configuration value exceeds bounds');
      if (item === null || typeof item === 'boolean') { bytes += 8; }
      else if (typeof item === 'number') { assert.ok(Number.isFinite(item), 'Invalid diagnostic value'); bytes += 32; }
      else if (typeof item === 'string') { bytes += Buffer.byteLength(item); }
      else {
        assert.ok(item && typeof item === 'object' && !seen.has(item), 'Invalid diagnostic value');
        seen.add(item);
        let output;
        if (Array.isArray(item)) {
          assert.ok(item.length <= limits.maxValueNodes, 'Diagnostic configuration value exceeds bounds');
          output = item.map(child => visit(child, depth + 1));
        } else {
          const names = Object.keys(item);
          assert.ok(names.length <= limits.maxValueNodes, 'Diagnostic configuration value exceeds bounds');
          output = Object.create(null);
          for (const key of names) {
            bytes += Buffer.byteLength(key);
            assert.ok(bytes <= limits.maxValueBytes, 'Diagnostic configuration value exceeds bounds');
            if (item[key] !== undefined) output[key] = visit(item[key], depth + 1);
          }
        }
        seen.delete(item);
        assert.ok(bytes <= limits.maxValueBytes, 'Diagnostic configuration value exceeds bounds');
        return output;
      }
      assert.ok(bytes <= limits.maxValueBytes, 'Diagnostic configuration value exceeds bounds');
      return item;
    };
    return visit(input === undefined ? null : input, 0);
  };
  const checked = action => {
    if (failure) throw failure;
    try { return action(); } catch (error) { failure = error; throw error; }
  };
  const snapshot = document => {
    if (document !== undefined) {
      assert.equal(document.uri.toString(), uri, 'Diagnostic document resource differs');
      assert.equal(document.languageId, 'plaintext', 'Diagnostic document language differs');
      assert.ok(Number.isInteger(document.version) && document.version > 0 && document.version <= 2147483647,
        'Diagnostic document version exceeds bounds');
      if (boundDocument) assert.equal(document, boundDocument, 'Diagnostic document object differs');
    }
    const editor = vscode.workspace.getConfiguration('editor', scope);
    const files = vscode.workspace.getConfiguration('files', scope);
    return { resource: 'main.txt', uri, languageId: 'plaintext', open: !!document,
      version: document?.version ?? null, exactDocument: boundDocument && document ? document === boundDocument : null,
      effective: value({ codeActionsOnSave: editor.get('codeActionsOnSave'), formatOnSave: editor.get('formatOnSave'),
        autoSave: files.get('autoSave'), autoSaveDelay: files.get('autoSaveDelay') }),
      inspect: value({ codeActionsOnSave: editor.inspect('codeActionsOnSave'), formatOnSave: editor.inspect('formatOnSave'),
        autoSave: files.inspect('autoSave'), autoSaveDelay: files.inspect('autoSaveDelay') }) };
  };
  const retain = (origin, document, context) => checked(() => {
    assert.ok(!disposed && records.length < limits.maxRecords, 'Diagnostic record budget exceeded');
    const elapsedMs = now() - start;
    assert.ok(Number.isInteger(elapsedMs) && elapsedMs >= lastElapsed && elapsedMs <= 300000,
      'Diagnostic clock exceeds bounded worker lifetime');
    lastElapsed = elapsedMs;
    const currentPhase = phase();
    assert.ok(currentPhase === 'setup' || currentPhase === 'target', 'Diagnostic phase differs');
    const entry = { sequence: records.length + 1, origin, phase: currentPhase, elapsedMs,
      snapshot: snapshot(document), context: value(context) };
    const encoded = JSON.stringify(entry);
    totalBytes += Buffer.byteLength(encoded);
    assert.ok(totalBytes <= limits.maxTotalBytes, 'Diagnostic total byte budget exceeded');
    records.push(entry);
    return entry;
  });
  const initial = checked(() => snapshot(undefined));
  const listener = vscode.workspace.onDidChangeConfiguration(event => {
    try {
      if (failure || disposed) return;
      const affected = keys.filter(key => event.affectsConfiguration(key, scope));
      if (affected.length) retain('configuration-change', boundDocument, { affected });
    } catch (error) {
      // Event dispatchers may swallow listener errors. Latch them so finish
      // rejects the capture instead of returning silently truncated evidence.
      failure = error;
    }
  });
  return {
    bind(document) { return checked(() => {
      assert.ok(!boundDocument, 'Diagnostic document rebound');
      snapshot(document); boundDocument = document;
    }); },
    provider(document, context) { return retain('source-provider', document, context); },
    saved(document) { return retain('did-save', document, null); },
    finish() { return checked(() => {
      const result = { protocol: 'save-configuration-diagnostics-v1', limits, initial,
      final: snapshot(boundDocument), records, scope: 'Public target resource/language configuration at source callbacks, didSave and all relevant configuration events; independent diagnostic metadata, no migration forcing, preferred target predicate or target retry' };
      assert.ok(Buffer.byteLength(JSON.stringify(result)) <= limits.maxTotalBytes,
        'Diagnostic total byte budget exceeded');
      return result;
    }); },
    partial() { return { protocol: 'save-configuration-diagnostics-v1', limits, initial,
      final: records.at(-1)?.snapshot || initial, records, status: 'partial-failure',
      error: failure ? String(failure.message || failure).slice(0, 1024) : null,
      scope: 'Exact retained diagnostic prefix on failed setup; not a successful target capture' }; },
    dispose() { if (!disposed) { disposed = true; listener.dispose(); } }
  };
};

// Diagnostic-only setup API receipts. Recording errors are latched separately:
// they never replace the original strict readiness assertion or provider result.
exports.sourceApiDiagnostics = (now = Date.now) => {
  const limits = Object.freeze({ maxCallbacks: 256, maxEvents: 512,
    maxActions: 128, maxTitleBytes: 1024, maxKindBytes: 128, maxBytes: 65536 });
  const receipt = { protocol: 'source-api-readiness-diagnostics-v1', limits,
    status: 'not-issued', callbacks: [], cancellationEvents: [],
    scope: 'One unchanged public setup API call; retained result/cancellation boundaries are diagnostic and do not authorize targets' };
  const subscriptions = [];
  let active = false, sealed = false, start;
  const fail = error => { receipt.recordingError ||= String(error.message || error).slice(0, 1024); };
  const safe = work => { try { work(); } catch (error) { fail(error); } };
  const elapsed = () => {
    const value = now() - start;
    assert.ok(Number.isSafeInteger(value) && value >= 0, 'Invalid source API diagnostic clock');
    return value;
  };
  const boundedString = (value, maximum) => {
    assert.ok(typeof value === 'string' && Buffer.byteLength(value) <= maximum,
      'Source API diagnostic string exceeds bounds');
    return value;
  };
  const append = (list, value, maximum) => {
    assert.ok(list.length < maximum, 'Source API diagnostic record budget exceeded');
    list.push(value);
    if (Buffer.byteLength(JSON.stringify(receipt)) > limits.maxBytes - 16384) {
      list.pop(); throw new Error('Source API diagnostic byte budget exceeded');
    }
  };
  const stop = () => {
    active = false;
    for (const subscription of subscriptions) {
      safe(() => { subscription.record.cancelledAtEnd = subscription.token.isCancellationRequested;
        assert.equal(typeof subscription.record.cancelledAtEnd, 'boolean'); });
      safe(() => subscription.listener?.dispose());
    }
    subscriptions.length = 0;
  };
  return {
    begin(input) {
      safe(() => {
        assert.equal(receipt.status, 'not-issued', 'Setup API issued more than once');
        start = now(); assert.ok(Number.isSafeInteger(start));
        // Caller supplies only the fixed public command/range/URI arguments.
        assert.ok(Buffer.byteLength(JSON.stringify(input)) <= 8192, 'Setup API input exceeds bounds');
        receipt.input = JSON.parse(JSON.stringify(input)); receipt.status = 'pending'; active = true;
      });
    },
    callback(token) {
      if (!active) return;
      safe(() => {
        const record = { sequence: receipt.callbacks.length + 1, elapsedMs: elapsed(),
          cancelledAtEntry: token.isCancellationRequested };
        assert.equal(typeof record.cancelledAtEntry, 'boolean');
        append(receipt.callbacks, record, limits.maxCallbacks);
        const subscription = { record, token, listener: undefined };
        // Retain cleanup ownership before subscribing: a public token may invoke
        // the listener synchronously when it was already canceled.
        subscriptions.push(subscription);
        subscription.listener = token.onCancellationRequested(() => safe(() => {
          append(receipt.cancellationEvents, { callback: record.sequence, elapsedMs: elapsed() }, limits.maxEvents);
        }));
      });
    },
    resolved(value) {
      if (sealed) return;
      safe(() => {
        receipt.status = 'resolved'; receipt.elapsedMs = elapsed();
        receipt.result = { type: value === null ? 'null' : Array.isArray(value) ? 'array' : typeof value };
        if (Array.isArray(value)) {
          receipt.result.count = value.length;
          assert.ok(value.length <= limits.maxActions, 'Source API action summary exceeds bounds');
          const actions = value.map(action => ({
            title: boundedString(action.title, limits.maxTitleBytes),
            kind: action.kind == null ? null : boundedString(action.kind.value, limits.maxKindBytes),
            disabled: action.disabled == null ? null : boundedString(action.disabled.reason, limits.maxTitleBytes)
          }));
          assert.ok(Buffer.byteLength(JSON.stringify(receipt)) + Buffer.byteLength(JSON.stringify(actions))
            <= limits.maxBytes - 16384, 'Source API action summary byte budget exceeded');
          receipt.result.actions = actions;
        }
      });
      stop();
    },
    rejected(error) {
      if (sealed) return;
      safe(() => { receipt.status = 'rejected'; receipt.elapsedMs = elapsed();
        receipt.error = { name: String(error.name || 'Error').slice(0, 128),
          message: String(error.message || error).slice(0, 1024) }; });
      stop();
    },
    interrupted(error) {
      safe(() => {
        receipt.boundaryError = { name: String(error.name || 'Error').slice(0, 128),
          message: String(error.message || error).slice(0, 1024) };
        if (receipt.status === 'pending') {
          receipt.status = 'interrupted'; receipt.elapsedMs = elapsed();
          receipt.apiSettlement = 'unobserved';
        }
      });
      stop(); sealed = true;
    },
    current(snapshot) {
      safe(() => {
        const value = snapshot();
        assert.ok(Buffer.byteLength(JSON.stringify(value)) <= 8192, 'Setup API current proof exceeds bounds');
        receipt.after = JSON.parse(JSON.stringify(value));
      });
    },
    proof() {
      // Defensive cap also covers errors added during cleanup. Never substitute
      // a diagnostic failure for the caller's real assertion/API rejection.
      if (Buffer.byteLength(JSON.stringify(receipt)) > limits.maxBytes) {
        return { protocol: receipt.protocol, limits, status: receipt.status,
          recordingError: 'Source API diagnostic envelope exceeds bounds', scope: receipt.scope };
      }
      return JSON.parse(JSON.stringify(receipt));
    },
    dispose() { stop(); sealed = true; }
  };
};

exports.saveCodeActionsTrace = async (vscode, name, workspace) => {
  const fixture = cases.find(value => value.name === name);
  assert.ok(fixture, 'Unknown save code actions fixture');
  const file = path.join(workspace, 'main.txt');
  assert.equal(fs.readFileSync(file, 'utf8'), TEXT);
  const required = ['type', 'workbench.action.files.save', 'undo', 'redo'];
  const budget = setupBudget();
  const progress = { saveParticipantReadiness: null, configurationReadiness: null };
  const sourceApiDiagnostic = exports.sourceApiDiagnostics();
  const targetProgress = { protocol: 'stable-save-target-prefix-v1', callbacks: [], saved: [], changes: [],
    observations: [], steps: [], issued: [], pending: null,
    limits: { maxRecordsPerList: 256, maxPrefixBytes: 524288, maxCurrentSnapshotBytes: 163840, maxCurrentTextBytes: 65536, maxCurrentSelections: 256 },
    scope: 'Actual issued/pending gesture and completed public target prefix on failure; no successful target qualification' };
  let targetFailure;
  const targetGuard = () => { if (targetFailure) throw targetFailure; };
  const retainTarget = (list, value) => {
    targetGuard();
    try {
      assert.ok(list.length < targetProgress.limits.maxRecordsPerList, 'Target prefix record budget exceeded');
      list.push(value);
      try { assert.ok(Buffer.byteLength(JSON.stringify(targetProgress)) <= targetProgress.limits.maxPrefixBytes - targetProgress.limits.maxCurrentSnapshotBytes - 4096,
        'Target prefix byte budget exceeded'); } catch (error) { list.pop(); throw error; }
    } catch (error) {
      targetFailure = error; targetProgress.recordingError = String(error.message || error).slice(0, 1024); throw error;
    }
  };
  const beginTarget = (action, commandIssued) => {
    const record = { sequence: targetProgress.issued.length + 1, action, commandIssued, completed: false };
    retainTarget(targetProgress.issued, record); targetProgress.pending = record;
  };
  const completeTarget = () => { targetGuard(); targetProgress.pending.completed = true; targetProgress.pending = null; };

  let phase = 'setup', document, editor, commands, setupAuthorization, targetCommandsIssued = 0;
  let registration, saveListener, changeListener;
  const diagnostic = exports.configurationDiagnostics(vscode, vscode.Uri.file(file), () => phase);
  try {
    commands = new Set(await budget.bound(() => vscode.commands.getCommands(true)));
    for (const command of required) assert.ok(commands.has(command), `Original command missing: ${command}`);
    const targetBeforeReadiness = fs.readFileSync(file, 'utf8');
    const { saveParticipantReadiness, configurationProof } = await exports.stableSaveSetup(
      vscode, workspace, fixture, budget, progress);
    assert.equal(fs.readFileSync(file, 'utf8'), targetBeforeReadiness,
      'Readiness changed original target disk');
    const { callbacks, saved, changes } = targetProgress;
    const push = retainTarget;
    const definitions = [
      { title: 'Synthetic imports', kind: 'source.organizeImports', line: 1, text: 'imports=1' },
      { title: 'Synthetic child fix', kind: 'source.fixAll.child', line: 2, text: 'child=1' },
      { title: 'Synthetic root fix', kind: 'source.fixAll', line: 0, text: 'fix=1' },
      { title: 'Synthetic unrelated prefix', kind: 'source.fixAllX', line: 3, text: 'other=1' },
    ];
    const selector = { scheme: 'file', language: 'plaintext', pattern: new vscode.RelativePattern(workspace, 'main.txt') };
    registration = vscode.languages.registerCodeActionsProvider(selector, {
      provideCodeActions(doc, range, context, token) {
        sourceApiDiagnostic.callback(token);
        diagnostic.provider(doc, { only: context.only?.value ?? null,
          triggerKind: context.triggerKind, cancelled: token.isCancellationRequested });
        // Deliberately return every kind: the editor must filter on hierarchy,
        // exclusions and the requested family, rather than trusting this provider.
        const actions = definitions.map(definition => {
          const action = new vscode.CodeAction(definition.title, new vscode.CodeActionKind(definition.kind));
          action.edit = new vscode.WorkspaceEdit();
          action.edit.replace(doc.uri, doc.lineAt(definition.line).range, definition.text);
          return action;
        });
        push(callbacks, { phase, only: context.only?.value ?? null, triggerKind: context.triggerKind,
          version: doc.version, text: doc.getText(), cancelled: token.isCancellationRequested,
          range: [range.start.line, range.start.character, range.end.line, range.end.character],
          returned: definitions.map(value => ({ title: value.title, kind: value.kind })) });
        return actions;
      }
    }, { providedCodeActionKinds: definitions.map(value => new vscode.CodeActionKind(value.kind)) });
    document = await budget.bound(() => vscode.workspace.openTextDocument(vscode.Uri.file(file)));
    diagnostic.bind(document);
    editor = await budget.bound(() => vscode.window.showTextDocument(document, { preview: false }));
    const end = document.positionAt(document.getText().length);
    editor.selection = new vscode.Selection(end, end);
    const scalar = position => [...document.getText().slice(0, document.offsetAt(position))].length;
    const proof = () => ({ text: document.getText(), version: document.version, dirty: document.isDirty,
      selections: editor.selections.map(selection => [selection.anchor.line, selection.anchor.character,
        selection.active.line, selection.active.character]), disk: fs.readFileSync(file, 'utf8') });
    const state = action => ({ action, resource: 'main.txt', text: document.getText(), dirty: document.isDirty,
      primary: { anchor: scalar(editor.selection.anchor), cursor: scalar(editor.selection.active) },
      disk: fs.readFileSync(file, 'utf8') });
    saveListener = vscode.workspace.onDidSaveTextDocument(doc => {
      if (doc === document) {
        try { diagnostic.saved(doc); } catch { /* failure latched; preserve original acknowledgement */ }
        push(saved, { phase, version: doc.version, text: doc.getText(), dirty: doc.isDirty,
        disk: fs.readFileSync(file, 'utf8') });
      }
    });
    changeListener = vscode.workspace.onDidChangeTextDocument(event => {
      if (event.document === document) push(changes, { phase, version: document.version,
        text: document.getText(), dirty: document.isDirty, reason: event.reason ?? null,
        changes: event.contentChanges.map(change => ({ rangeOffset: change.rangeOffset,
          rangeLength: change.rangeLength, text: change.text })) });
    });
    async function stable() {
      const start = Date.now(); let changed = start, previous;
      for (;;) {
        targetGuard();
        const key = JSON.stringify(proof());
        if (key !== previous) { previous = key; changed = Date.now(); }
        if (Date.now() - start >= 100 && Date.now() - changed >= 50) return {
          elapsedMs: Date.now() - start,
          scope: 'Actual command/didSave acknowledgement followed by minimum100ms/50ms stable public state; no preferred target-output predicate or retry' };
        assert.ok(Date.now() - start < 3000, 'Public state did not settle');
        await pause(10);
      }
    }
    async function didSave(after) {
      const start = Date.now();
      while (saved.length === after) {
        targetGuard();
        assert.ok(Date.now() - start < 8000, 'Actual didSave acknowledgement absent');
        await pause(10);
      }
      assert.equal(saved.length, after + 1, 'Unexpected duplicate target save');
      assert.equal(saved.at(-1).disk, saved.at(-1).text, 'didSave precedes disk commit');
      return { elapsedMs: Date.now() - start, event: saved.at(-1) };
    }
    try {
      assert.equal(document.eol, vscode.EndOfLine.CRLF);
      const configuration = vscode.workspace.getConfiguration('editor', document);
      const effective = { codeActionsOnSave: configuration.get('codeActionsOnSave'),
        formatOnSave: configuration.get('formatOnSave'),
        autoSave: vscode.workspace.getConfiguration('files', document).get('autoSave'),
        autoSaveDelay: vscode.workspace.getConfiguration('files', document).get('autoSaveDelay') };
      assert.equal(effective.autoSave, fixture.autosave); assert.equal(effective.formatOnSave, false);
      const before = proof();
      let warm;
      try {
        warm = await budget.bound(() => {
          sourceApiDiagnostic.begin({ command: 'vscode.executeCodeActionProvider', resource: 'main.txt',
            uri: document.uri.toString(), range: [0, 0, end.line, end.character], kind: 'source', itemResolveCount: 100, before });
          let request;
          try {
            request = vscode.commands.executeCommand('vscode.executeCodeActionProvider', document.uri,
              new vscode.Range(new vscode.Position(0, 0), end), 'source', 100);
          } catch (error) { sourceApiDiagnostic.rejected(error); throw error; }
          return Promise.resolve(request).then(value => {
            sourceApiDiagnostic.resolved(value); return value;
          }, error => { sourceApiDiagnostic.rejected(error); throw error; });
        });
      } catch (error) { sourceApiDiagnostic.interrupted(error); sourceApiDiagnostic.current(proof); throw error; }
      sourceApiDiagnostic.current(proof);
      assert.ok(Array.isArray(warm) && warm.length === definitions.length, 'Positive source-action API readiness failed');
      assert.equal(callbacks.length, 1, 'Readiness callback count differs');
      assert.deepEqual(proof(), before, 'Readiness changed target text/version/selection/disk');
      assert.ok(!sourceApiDiagnostic.proof().recordingError, 'Setup API diagnostic recording failed');
      const checkpoint = reader(vscode, document.uri)();
      assert.ok(matches(checkpoint, configurationProof.expected), 'Canonical configuration changed before target');
      setupAuthorization = budget.authorizeTargets();
      const setup = { saveParticipantReadiness, configurationReadiness: configurationProof, configurationCheckpoint: checkpoint, setupAuthorization, effective, effectiveKeys: Array.isArray(effective.codeActionsOnSave) ? null : Object.keys(effective.codeActionsOnSave),
        configurationInspect: configuration.inspect('codeActionsOnSave'), before, after: proof(),
        warmKinds: warm.map(action => action.kind?.value ?? null),
        scope: 'One public execute-source-provider call; no edits applied, exact unchanged proof and no readiness retry' };
      phase = 'target';
      const { observations, steps } = targetProgress;
      retainTarget(observations, state('initial'));
      const count = saved.length;
      beginTarget('type', true); targetCommandsIssued += 1;
      await vscode.commands.executeCommand('type', { text: 'λ🙂' });
      retainTarget(steps, { action: 'type', settlement: await stable() }); retainTarget(observations, state('type')); completeTarget();
      const saveAction = fixture.autosave === 'afterDelay' ? 'files.autoSave.afterDelay' : 'workbench.action.files.save';
      beginTarget(saveAction, fixture.autosave !== 'afterDelay');
      if (fixture.autosave !== 'afterDelay') {
        targetCommandsIssued += 1; await vscode.commands.executeCommand(saveAction);
      }
      retainTarget(steps, { action: saveAction, acknowledgement: await didSave(count), settlement: await stable() });
      retainTarget(observations, state(saveAction)); completeTarget();
      if (fixture.autosave === 'off') {
        // Five steps cover the four possible mutations plus typed input. Fixed
        // counts retain no-ops/unexpected grouping rather than retrying to a goal.
        for (const command of [...Array(5).fill('undo'), ...Array(5).fill('redo')]) {
          beginTarget(command, true); targetCommandsIssued += 1;
          await vscode.commands.executeCommand(command);
          retainTarget(steps, { action: command, settlement: await stable() }); retainTarget(observations, state(command)); completeTarget();
        }
      }
      targetGuard();
      const configurationDiagnostics = diagnostic.finish();
      const targetChecks = configurationDiagnostics.records.filter(record => record.phase === 'target')
        .map(record => ({ sequence: record.sequence, origin: record.origin, matched: matches(record.snapshot, configurationProof.expected) }));
      const finalCheckpoint = reader(vscode, document.uri)();
      const stableConfiguration = { valid: targetChecks.every(record => record.matched) && matches(finalCheckpoint, configurationProof.expected),
        targetChecks, finalCheckpoint, scope: 'Canonical configuration checks only; all original target outputs and events remain retained even on mismatch' };
      return { name, observerProtocol: 'stable-save-source-api-diagnostics-v1',
        sourceApiReadinessDiagnostics: sourceApiDiagnostic.proof(),
        setup, observations, steps, callbacks, saved, changes, stableConfiguration, targetCommandsIssued,
        configurationDiagnostics,
        commandInventory: required.sort(), executeApiListed: commands.has('vscode.executeCodeActionProvider'),
        scope: 'Pinned synthetic sourceActions, layered/language settings and original Save/Undo/Redo; exact callback order and text/disk/selection. Not native LSP, unchanged extension or full save-lifecycle parity' };
    } finally {
      saveListener?.dispose(); changeListener?.dispose(); registration?.dispose();
      saveListener = changeListener = registration = undefined;
    }
  } catch (error) {
    // A secondary snapshot/IO failure must never replace the actual target error.
    let disk = null, diskError = null;
    try { disk = boundedSource(file, targetProgress.limits.maxCurrentTextBytes).toString('utf8'); }
    catch (snapshotError) { diskError = String(snapshotError.message || snapshotError).slice(0, 1024); }
    if (document && editor) {
      try {
        const text = document.getText();
        assert.ok(Buffer.byteLength(text) <= targetProgress.limits.maxCurrentTextBytes
          && editor.selections.length <= targetProgress.limits.maxCurrentSelections, 'Current target snapshot exceeds bounds');
        const current = { status: 'observed', resource: 'main.txt', uri: document.uri.toString(),
          languageId: document.languageId, version: document.version, text, dirty: document.isDirty,
          selections: editor.selections.map(selection => [selection.anchor.line, selection.anchor.character,
            selection.active.line, selection.active.character]), disk, diskError };
        assert.ok(Buffer.byteLength(JSON.stringify(current)) <= targetProgress.limits.maxCurrentSnapshotBytes,
          'Current target snapshot exceeds byte bounds');
        targetProgress.current = current;
      } catch (snapshotError) { targetProgress.current = { status: 'unavailable',
        error: String(snapshotError.message || snapshotError).slice(0, 1024), disk, diskError }; }
    } else targetProgress.current = { status: 'unavailable', error: 'Target document/editor not bound', disk, diskError };
    error.saveReferenceFailure = { protocol: 'stable-save-setup-failure-v1',
      observerProtocol: 'stable-save-source-api-diagnostics-v1',
      sourceApiReadinessDiagnostics: sourceApiDiagnostic.proof(), name, phase,
      targetStarted: targetCommandsIssued > 0, targetCommandsIssued, targetProgress, setupAuthorization: setupAuthorization || null,
      combinedBudget: budget.proof(), progress,
      originalTarget: { input: TEXT, disk, diskError },
      configurationDiagnostics: diagnostic.partial(),
      error: { name: String(error.name || 'Error').slice(0, 128), message: String(error.message || error).slice(0, 1024) },
      scope: 'Actual bounded failed setup prefix; target commands were not started unless targetStarted is true; no outcome retry' };
    throw error;
  } finally {
    budget.retire(); diagnostic.dispose(); sourceApiDiagnostic.dispose();
    saveListener?.dispose(); changeListener?.dispose(); registration?.dispose();
  }
};
