'use strict';
const assert = require('node:assert/strict');
const path = require('node:path');
let caseInventory;
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const KEYS = ['editor.codeActionsOnSave', 'editor.formatOnSave', 'files.autoSave', 'files.autoSaveDelay'];
const LIMITS = Object.freeze({ deadlineMs: 10000, sampleIntervalMs: 100, quietMs: 250,
  maxSamples: 128, maxEvents: 128, maxValueNodes: 1024, maxValueDepth: 8,
  maxValueBytes: 32768, maxTotalBytes: 524288 });
const PROTOCOL = 'save-configuration-readiness-v1';
const SCOPE = 'Public source-defined canonical resource/language configuration before original target commands; no settings update, private migration forcing, target-output predicate or target retry';
function copy(input) {
  let nodes = 0, bytes = 0;
  const seen = new Set();
  function visit(value, depth) {
    nodes += 1;
    assert.ok(nodes <= LIMITS.maxValueNodes && depth <= LIMITS.maxValueDepth,
      'Configuration readiness value exceeds bounds');
    if (value === null || typeof value === 'boolean') bytes += 8;
    else if (typeof value === 'number') { assert.ok(Number.isFinite(value), 'Invalid readiness value'); bytes += 32; }
    else if (typeof value === 'string') bytes += Buffer.byteLength(value);
    else {
      assert.ok(value && typeof value === 'object' && !seen.has(value), 'Invalid readiness value');
      seen.add(value);
      let result;
      if (Array.isArray(value)) {
        assert.ok(value.length <= LIMITS.maxValueNodes, 'Configuration readiness value exceeds bounds');
        result = value.map(child => visit(child, depth + 1));
      } else {
        const names = Object.keys(value);
        assert.ok(names.length <= LIMITS.maxValueNodes, 'Configuration readiness value exceeds bounds');
        result = Object.create(null);
        for (const key of names) {
          bytes += Buffer.byteLength(key);
          assert.ok(bytes <= LIMITS.maxValueBytes, 'Configuration readiness value exceeds bounds');
          if (value[key] !== undefined) result[key] = visit(value[key], depth + 1);
        }
      }
      seen.delete(value);
      assert.ok(bytes <= LIMITS.maxValueBytes, 'Configuration readiness value exceeds bounds');
      return result;
    }
    assert.ok(bytes <= LIMITS.maxValueBytes, 'Configuration readiness value exceeds bounds');
    return value;
  }
  return visit(input === undefined ? null : input, 0);
}
// This is solely the pinned migration's entry transform, not a settings merger.
function canonical(value) {
  if (Array.isArray(value)) return value.slice();
  assert.ok(value && typeof value === 'object', 'Invalid declared code-action input');
  const result = Object.create(null);
  for (const [key, item] of Object.entries(value)) result[key] = typeof item === 'boolean'
    ? (item ? 'explicit' : 'never') : item;
  return result;
}
function canonicalContract(fixture) {
  const original = loadCases().find(value => value.name === fixture.name);
  assert.ok(original && JSON.stringify(original) === JSON.stringify(fixture), 'Unknown or changed declared fixture');
  // Explicit effective declarations: do not import/reimplement a native merger.
  const effectiveActions = {
    'object-fixall-first': { 'source.organizeImports': 'explicit', 'source.fixAll': 'explicit' },
    'array-imports-first': ['source.organizeImports', 'source.fixAll'],
    'array-fixall-first': ['source.fixAll', 'source.organizeImports'],
    'ancestor-false-child': { 'source.fixAll': 'explicit', 'source.fixAll.child': 'never' },
    'ancestor-never-child': { 'source.fixAll': 'explicit', 'source.fixAll.child': 'never' },
    'source-ancestor-false-family': { source: 'explicit', 'source.fixAll': 'never' },
    'source-ancestor-never-family': { source: 'explicit', 'source.fixAll': 'never' },
    'after-delay-always-skipped': { 'source.fixAll': 'always', 'source.organizeImports': 'always' },
    'after-delay-array-skipped': ['source.organizeImports', 'source.fixAll'],
    'user-workspace-object-merge': { 'source.fixAll': 'explicit', 'source.organizeImports': 'explicit' },
    'language-composite-single-merge': { 'source.organizeImports': 'explicit', 'source.fixAll': 'explicit', 'source.fixAll.child': 'never' },
    'workspace-array-replaces-object': ['source.organizeImports'],
    'workspace-empty-object-retains-user': { 'source.fixAll': 'explicit' },
    'child-only-dot-boundary': { 'source.fixAll.child': 'explicit' }
  };
  const actions = { key: 'editor.codeActionsOnSave', defaultValue: {},
    globalValue: canonical(fixture.user['editor.codeActionsOnSave']) };
  if (Object.hasOwn(fixture.workspace, 'editor.codeActionsOnSave')) {
    actions.workspaceValue = canonical(fixture.workspace['editor.codeActionsOnSave']);
    actions.workspaceFolderValue = canonical(fixture.workspace['editor.codeActionsOnSave']);
  }
  if (fixture.name === 'language-composite-single-merge') {
    actions.globalLanguageValue = { 'source.fixAll': 'explicit' };
    actions.workspaceLanguageValue = { 'source.fixAll.child': 'never' };
    actions.workspaceFolderLanguageValue = { 'source.fixAll.child': 'never' };
    actions.languageIds = ['plaintext', 'cpp'];
  }
  return copy({ effective: { codeActionsOnSave: effectiveActions[fixture.name], formatOnSave: false,
    autoSave: fixture.autosave, autoSaveDelay: 500 }, inspect: { codeActionsOnSave: actions,
    formatOnSave: { key: 'editor.formatOnSave', defaultValue: false, globalValue: false, languageIds: ['ini'] },
    autoSave: { key: 'files.autoSave', defaultValue: 'off', globalValue: fixture.autosave },
    autoSaveDelay: { key: 'files.autoSaveDelay', defaultValue: 1000, globalValue: 500 } } });
}
function orderedEqual(actual, expected) {
  return JSON.stringify(actual) === JSON.stringify(expected);
}
// Inspect property ordering itself is not semantics. Nested action entry order,
// arrays and languageIds ARE preserved and compared independently.
function matches(snapshot, expected) {
  if (!orderedEqual(snapshot.effective, expected.effective)) return false;
  if (JSON.stringify(Object.keys(snapshot.inspect).sort()) !== JSON.stringify(Object.keys(expected.inspect).sort())) return false;
  for (const key of Object.keys(expected.inspect)) {
    const actual = snapshot.inspect[key], wanted = expected.inspect[key];
    if (!actual || JSON.stringify(Object.keys(actual).sort()) !== JSON.stringify(Object.keys(wanted).sort())) return false;
    for (const field of Object.keys(wanted)) if (!orderedEqual(actual[field], wanted[field])) return false;
  }
  return true;
}
function reader(vscode, targetUri) {
  const uri = targetUri.toString();
  assert.ok(targetUri.scheme === 'file' && Buffer.byteLength(uri) <= 4096
    && path.basename(targetUri.fsPath) === 'main.txt', 'Configuration readiness resource differs');
  const scope = { uri: targetUri, languageId: 'plaintext' };
  return () => {
    // Reacquire every object: a previously returned API object is a snapshot.
    const editor = vscode.workspace.getConfiguration('editor', scope);
    const files = vscode.workspace.getConfiguration('files', scope);
    return { resource: 'main.txt', uri, languageId: 'plaintext',
      effective: copy({ codeActionsOnSave: editor.get('codeActionsOnSave'), formatOnSave: editor.get('formatOnSave'),
        autoSave: files.get('autoSave'), autoSaveDelay: files.get('autoSaveDelay') }),
      inspect: copy({ codeActionsOnSave: editor.inspect('codeActionsOnSave'), formatOnSave: editor.inspect('formatOnSave'),
        autoSave: files.inspect('autoSave'), autoSaveDelay: files.inspect('autoSaveDelay') }) };
  };
}
async function configurationReadiness(vscode, targetUri, fixture, clock = {}) {
  const now = clock.now || Date.now, wait = clock.pause || pause;
  const expected = canonicalContract(fixture), read = reader(vscode, targetUri);
  const start = now(), proof = { protocol: PROTOCOL, status: 'running', name: fixture.name,
    resource: 'main.txt', uri: targetUri.toString(), languageId: 'plaintext', expected, limits: LIMITS,
    samples: [], events: [], scope: SCOPE };
  let quietSince, previous = -1, failure, listener, timer, abortListener, disposed = false;
  clock.report?.(proof);
  function stamp() {
    const elapsedMs = now() - start;
    assert.ok(Number.isInteger(elapsedMs) && elapsedMs >= 0 && elapsedMs >= previous && elapsedMs <= LIMITS.deadlineMs,
      'Configuration readiness clock exceeds bounds');
    previous = elapsedMs; return elapsedMs;
  }
  function bound() {
    assert.ok(Buffer.byteLength(JSON.stringify(proof)) <= LIMITS.maxTotalBytes - 4096,
      'Configuration readiness metadata exceeds bounds');
  }
  function retain(list, record, maximum) {
    assert.ok(list.length < maximum, 'Configuration readiness record budget exceeded');
    list.push(record);
    try { bound(); } catch (error) { list.pop(); throw error; }
  }
  function checked(action) {
    if (failure) throw failure;
    try { return action(); } catch (error) { failure = error; throw error; }
  }
  const expire = () => {
    const error = new Error('Configuration readiness deadline exceeded');
    failure ||= error; return failure;
  };
  const timeout = new Promise((_, reject) => {
    timer = (clock.setTimeout || setTimeout)(() => reject(expire()), LIMITS.deadlineMs);
  });
  const aborted = new Promise((_, reject) => {
    abortListener = () => { failure ||= new Error('Configuration readiness canceled by setup deadline'); reject(failure); };
    if (clock.signal?.aborted) abortListener();
    else clock.signal?.addEventListener('abort', abortListener, { once: true });
  });
  timeout.catch(() => {}); aborted.catch(() => {});
  const bounded = async work => { const result = await Promise.race([work, timeout, aborted]); if (failure) throw failure; return result; };
  try {
    listener = vscode.workspace.onDidChangeConfiguration(event => {
      try {
        if (disposed || failure) return;
        const affected = KEYS.filter(key => event.affectsConfiguration(key, { uri: targetUri, languageId: 'plaintext' }));
        if (!affected.length) return;
        checked(() => retain(proof.events, { sequence: proof.events.length + 1,
          elapsedMs: stamp(), affected, snapshot: read() }, LIMITS.maxEvents));
        quietSince = undefined;
      } catch (error) { failure ||= error; }
    });
    for (;;) {
      checked(() => {
        const elapsedMs = stamp();
        assert.ok(elapsedMs < LIMITS.deadlineMs, 'Configuration readiness deadline exceeded');
        const snapshot = read(), matched = matches(snapshot, expected);
        retain(proof.samples, { sequence: proof.samples.length + 1, elapsedMs, snapshot, matched }, LIMITS.maxSamples);
        if (!matched) quietSince = undefined;
        else if (quietSince === undefined) quietSince = elapsedMs;
        if (matched && elapsedMs - quietSince >= LIMITS.quietMs) {
          proof.status = 'ready'; proof.elapsedMs = elapsedMs; proof.quietSinceMs = quietSince;
          proof.finalSampleSequence = proof.samples.at(-1).sequence; bound();
        }
      });
      if (proof.status === 'ready') return proof;
      await bounded(wait(Math.min(LIMITS.sampleIntervalMs, LIMITS.deadlineMs - (now() - start))));
    }
  } catch (error) {
    proof.status = 'failed'; proof.error = String(error.message || error).slice(0, 1024);
    error.configurationReadiness = proof;
    throw error;
  } finally {
    disposed = true; listener?.dispose(); clock.signal?.removeEventListener('abort', abortListener);
    (clock.clearTimeout || clearTimeout)(timer);
  }
}
function setupBudget(clock = {}) {
  const now = clock.now || Date.now, start = now(), controller = new AbortController();
  let closed = false, authorized = false, timer;
  const limit = 15000;
  const timeout = new Promise((_, reject) => {
    timer = (clock.setTimeout || setTimeout)(() => {
      closed = true; controller.abort(); reject(new Error('Combined save setup deadline exceeded'));
    }, limit);
  });
  timeout.catch(() => {});
  const guard = () => {
    const elapsedMs = now() - start;
    assert.ok(!closed && !authorized && Number.isInteger(elapsedMs) && elapsedMs >= 0 && elapsedMs < limit,
      'Combined save setup deadline exceeded');
  };
  return { signal: controller.signal,
    async bound(work) { guard(); assert.equal(typeof work, 'function');
      const value = await Promise.race([work(), timeout]); guard(); return value; },
    authorizeTargets() { guard(); authorized = true; (clock.clearTimeout || clearTimeout)(timer);
      return { protocol: 'stable-save-setup-v1', elapsedMs: now() - start, deadlineMs: limit, targetAttempts: 0 }; },
    retire() { closed = true; controller.abort(); (clock.clearTimeout || clearTimeout)(timer); },
    proof() { return { deadlineMs: limit, elapsedMs: now() - start, authorized }; } };
}
const SOURCE_INPUTS = Object.freeze(['save-code-actions.cjs', 'save-configuration-ready.cjs',
  'save-code-actions-cases.json', 'save-code-actions-suite.cjs', 'save-code-actions-run.cjs',
  'save-code-actions-worker.cjs', 'supervisor.cjs', 'package-lock.json', 'package.json', 'extension.cjs']);
function boundedSource(file, maximum, fileSystem = require('node:fs')) {
  const flags = fileSystem.constants.O_RDONLY | (fileSystem.constants.O_NONBLOCK || 0);
  const descriptor = fileSystem.openSync(file, flags);
  try {
    const initial = fileSystem.fstatSync(descriptor);
    assert.ok(initial.isFile() && Number.isSafeInteger(initial.size) && initial.size >= 0
      && initial.size <= maximum, 'Source file exceeds bounds or is not regular');
    const bytes = Buffer.alloc(maximum + 1);
    let count = 0;
    while (count <= maximum) {
      const received = fileSystem.readSync(descriptor, bytes, count, bytes.length - count, null);
      assert.ok(Number.isInteger(received) && received >= 0 && received <= bytes.length - count,
        'Invalid source read');
      if (received === 0) break;
      count += received;
    }
    assert.ok(count <= maximum, 'Source file exceeds bounds while reading');
    const final = fileSystem.fstatSync(descriptor);
    assert.ok(final.isFile() && final.size === initial.size && final.mtimeMs === initial.mtimeMs
      && count === initial.size, 'Source file changed or ended before its declared size');
    return bytes.subarray(0, count);
  } finally { fileSystem.closeSync(descriptor); }
}
function sourceIdentity(directory, fileSystem = require('node:fs')) {
  const { createHash } = require('node:crypto');
  const manifestBytes = boundedSource(path.join(directory, 'source-sha256.json'), 16384, fileSystem);
  const manifest = JSON.parse(manifestBytes);
  assert.ok(manifest && typeof manifest === 'object' && !Array.isArray(manifest), 'Invalid source manifest');
  assert.deepEqual(Object.keys(manifest).sort(), [...SOURCE_INPUTS].sort(), 'Source inventory differs');
  for (const file of SOURCE_INPUTS) assert.match(manifest[file], /^[a-f0-9]{64}$/, 'Invalid source digest');
  let total = 0;
  const actual = Object.create(null);
  for (const file of SOURCE_INPUTS) {
    const bytes = boundedSource(path.join(directory, file), Math.min(1048576, 4194304 - total), fileSystem);
    total += bytes.length;
    actual[file] = createHash('sha256').update(bytes).digest('hex');
    assert.equal(actual[file], manifest[file], `Source bytes differ: ${file}`);
  }
  return actual;
}
function loadCases() {
  if (!caseInventory) {
    const actual = sourceIdentity(__dirname);
    assert.equal(actual['save-code-actions-cases.json'],
      'a96e7078cd7e710cb33fc0c3d5e4e40e9cd0283e884035ff552dfd37b1a9be9c', 'Original case bytes differ');
    const bytes = boundedSource(path.join(__dirname, 'save-code-actions-cases.json'), 1048576);
    const { createHash } = require('node:crypto');
    assert.equal(createHash('sha256').update(bytes).digest('hex'), actual['save-code-actions-cases.json'],
      'Case bytes changed after source preflight');
    const parsed = JSON.parse(bytes);
    assert.ok(Array.isArray(parsed) && parsed.length === 14, 'Original case inventory differs');
    caseInventory = parsed;
  }
  return caseInventory;
}
function archiveFailure(output, name, failure, metadata, fileSystem = require('node:fs')) {
  assert.ok(loadCases().some(fixture => fixture.name === name), 'Unknown failure fixture');
  assert.equal(failure.protocol, 'stable-save-setup-failure-v1');
  assert.equal(failure.name, name, 'Failure fixture identity differs');
  // Compact encoding preserves every retained value while bounding failure
  // envelopes independently of pretty-print indentation expansion.
  const bytes = JSON.stringify(failure) + '\n';
  assert.ok(Buffer.byteLength(bytes) <= 2 * 1024 * 1024, 'Failed setup evidence exceeds bounds');
  const { createHash } = require('node:crypto');
  const proof = { ...metadata, name, failureSha256: createHash('sha256').update(bytes).digest('hex'),
    scope: 'Actual failed setup/target prefix preserved before rethrow; no successful target qualification claimed' };
  const proofBytes = JSON.stringify(proof, null, 2) + '\n';
  assert.ok(Buffer.byteLength(proofBytes) <= 32768, 'Failed setup provenance exceeds bounds');
  fileSystem.writeFileSync(path.join(output, `${name}-failure.json`), bytes);
  fileSystem.writeFileSync(path.join(output, `${name}-failure-provenance.json`), proofBytes);
}
module.exports = { configurationReadiness, canonicalContract, canonical, matches,
  reader, copy, setupBudget, archiveFailure, sourceIdentity, boundedSource, loadCases, SOURCE_INPUTS, LIMITS, PROTOCOL, SCOPE };
