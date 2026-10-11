'use strict';
// OS notifications and bounded metadata diffs run only in the optional-host
// worker. No file contents, editor models or native saves are touched.
const { Worker, isMainThread, parentPort, workerData } = require('node:worker_threads');
const path = require('node:path');
const MAX_WORKERS = 32, MAX_PATH = 4096, MAX_ENTRIES = 10000, MAX_DIRECTORIES = 256;
const MAX_PATH_BYTES = 4 * 1024 * 1024, MAX_EVENTS = 4096, MAX_EVENT_BYTES = 2 * 1024 * 1024;
const BATCH_EVENTS = 64, BATCH_BYTES = 64 * 1024, MAX_INFLIGHT = 4;
const actual = new Set();

function watchLocal(baseUri, options = {}, emit, error) {
  if (Number(process.versions.node.split('.')[0]) < 22 ||
      !['linux', 'darwin', 'win32'].includes(process.platform)) {
    throw new Error('Local filesystem watching requires Node >=22 on Linux, macOS or Windows');
  }
  const { Uri } = require('./api-types.cjs');
  if (!baseUri || baseUri.scheme !== 'file' || typeof emit !== 'function' || typeof error !== 'function') {
    throw new TypeError('Invalid local filesystem watcher');
  }
  const uri = Uri.parse(baseUri.toString());
  if (uri.query || uri.fragment || (process.platform !== 'win32' && uri.authority && uri.authority !== 'localhost')) {
    throw new Error('Local watcher requires an ordinary file URI');
  }
  const base = uri.fsPath;
  if (!path.isAbsolute(base) || Buffer.byteLength(base) > MAX_PATH || base.includes('\0')) {
    throw new Error('Local watcher path exceeds 4096-byte budget');
  }
  const excludes = options.recursive ? options.excludes ?? [] : [];
  if (!Array.isArray(excludes) || excludes.length > 128 ||
      excludes.some(item => typeof item !== 'string' || Buffer.byteLength(item) > MAX_PATH || item.includes('\0')) ||
      excludes.reduce((sum, item) => sum + Buffer.byteLength(item), 0) > 32 * 1024) {
    throw new Error('Local watcher exclude budget exceeded');
  }
  if (actual.size >= MAX_WORKERS) throw new Error('Local filesystem watcher actual worker limit reached');
  let disposed = false, exited = false, reported = false, initialized = false;
  let resolveReady, rejectReady, resolveClosed;
  const ready = new Promise((resolve, reject) => { resolveReady = resolve; rejectReady = reject; });
  ready.catch(() => {});
  const closed = new Promise(resolve => { resolveClosed = resolve; });
  const worker = new Worker(__filename, { workerData: {
    localWatcher: true, base, recursive: !!options.recursive, excludes: [...excludes],
  } });
  actual.add(worker);
  function report(value) {
    if (reported || disposed) return;
    reported = true;
    const reason = value instanceof Error ? value : new Error(String(value).slice(0, 4096));
    if (!initialized) rejectReady(reason);
    try { error(reason); } catch { /* diagnostics cannot own worker capacity */ }
  }
  function dispose() {
    if (disposed) return;
    disposed = true;
    if (!initialized) rejectReady(new Error('Local watcher disposed before initialization'));
    // Retain the original actual Worker until exit, not terminate() invocation.
    worker.terminate().catch(() => {});
  }
  worker.on('message', message => {
    if (message?.kind === 'ready') {
      initialized = true;
      if (!disposed) resolveReady();
    } else if (message?.kind === 'events') {
      if (!disposed && Array.isArray(message.events) && message.events.length <= BATCH_EVENTS) {
        for (const event of message.events) {
          if (disposed) break;
          try { emit({ type: event.type, uri: Uri.file(event.path) }); }
          catch (reason) { report(reason); dispose(); break; }
        }
      }
      if (!exited) worker.postMessage({ kind: 'ack', id: message.id });
    } else if (message?.kind === 'error') report(message.message);
  });
  worker.on('error', report);
  worker.on('exit', code => {
    exited = true;
    actual.delete(worker);
    if (!disposed && !reported) report(new Error('Local watcher stopped unexpectedly (' + code + ')'));
    if (!initialized) rejectReady(new Error('Local watcher exited before initialization'));
    resolveClosed();
  });
  return { dispose, ready, closed };
}

async function runWorker() {
  const fs = require('node:fs'), disk = require('node:fs/promises');
  const { base, recursive, excludes } = workerData;
  const compile = excludes.length ? require('./watcher-glob.cjs').compileGlob : undefined;
  const matchers = excludes.flatMap(value => {
    const variants = value.endsWith('/**') ? [value, value.slice(0, -3)] : [value];
    return variants.map(pattern => compile(pattern, { ignoreCase: process.platform !== 'linux' }));
  });
  const watches = new Map(), pending = [], inflight = new Map(), hints = new Set();
  let stopped = false, running = false, dirty = false, initialized = false;
  let timer, recovery, sequence = 0, queuedBytes = 0, hintBytes = 0, index = new Map();
  function fail(reason) {
    if (stopped) return;
    stopped = true;
    clearTimeout(timer); clearInterval(recovery);
    for (const entry of watches.values()) entry.watcher.close();
    watches.clear();
    parentPort.postMessage({ kind: 'error', message: String(reason?.message ?? reason).slice(0, 4096) });
    parentPort.close();
  }
  function schedule() {
    if (stopped) return;
    dirty = true;
    if (running || timer) return;
    timer = setTimeout(() => { timer = undefined; refresh().catch(fail); }, 10);
  }
  function hint(root, name, eventType) {
    if (stopped) return;
    if (eventType === 'change' && name != null) {
      const target = path.join(root, String(name));
      if (!hints.has(target)) {
        const charge = Buffer.byteLength(target) + 32;
        if (hints.size >= MAX_EVENTS || hintBytes + charge > MAX_EVENT_BYTES) {
          fail(new Error('Local watcher change-hint overflow')); return;
        }
        hints.add(target); hintBytes += charge;
      }
    }
    schedule();
  }
  function ensureWatch(target, directory, needed, stat) {
    needed.add(target);
    const identity = String(stat.dev) + ':' + String(stat.ino);
    const old = watches.get(target);
    if (old?.identity === identity) return true;
    if (old) { old.watcher.close(); watches.delete(target); }
    if (watches.size >= MAX_DIRECTORIES + 2) throw new Error('Local watcher directory handle budget exceeded');
    let watcher;
    try {
      watcher = fs.watch(target, { persistent: true, recursive: false }, (type, name) => {
        hint(directory ? target : path.dirname(target), name ?? (directory ? undefined : path.basename(target)), type);
      });
    } catch (reason) {
      if (reason.code === 'ENOENT' || reason.code === 'ENOTDIR') { schedule(); return false; }
      throw reason;
    }
    const entry = { watcher, identity };
    watcher.on('error', reason => {
      if (watches.get(target) !== entry) return;
      if (reason.code === 'ENOENT' || reason.code === 'ENOTDIR') {
        watches.delete(target); watcher.close(); schedule();
      } else fail(reason);
    });
    watches.set(target, entry);
    return true;
  }
  async function maybeStat(target, follow = false) {
    try { return await (follow ? disk.stat(target, { bigint: true }) : disk.lstat(target, { bigint: true })); }
    catch (reason) { if (['ENOENT', 'ENOTDIR'].includes(reason.code)) return undefined; throw reason; }
  }
  function stamp(stat) {
    return { kind: stat.isDirectory() ? 'directory' : stat.isSymbolicLink() ? 'link' : 'file',
      dev: stat.dev, ino: stat.ino, size: stat.size, mtime: stat.mtimeNs, ctime: stat.ctimeNs };
  }
  async function snapshot() {
    const end = Date.now() + 10000, result = new Map(), needed = new Set();
    let entries = 0, bytes = 0, directories = 0;
    function admit(target) {
      if (stopped) throw new Error('Local watcher retired');
      if (Date.now() >= end) throw new Error('Local watcher metadata scan deadline exceeded');
      if (++entries > MAX_ENTRIES || Buffer.byteLength(target) > MAX_PATH) throw new Error('Local watcher entry/path budget exceeded');
      bytes += Buffer.byteLength(target);
      if (bytes > MAX_PATH_BYTES) throw new Error('Local watcher metadata path budget exceeded');
    }
    function excluded(target) { const value = target.split(path.sep).join('/'); return matchers.some(check => check(value)); }
    // Watch nearest existing parent before reading a missing/recreated base.
    let anchor = path.dirname(base), anchorStat;
    for (let depth = 0; ; depth++) {
      if (depth > 128) throw new Error('Local watcher ancestor budget exceeded');
      const stat = await maybeStat(anchor, true);
      if (stat?.isDirectory()) { anchorStat = stat; break; }
      const next = path.dirname(anchor);
      if (next === anchor) throw new Error('Local watcher has no existing directory ancestor');
      anchor = next;
    }
    ensureWatch(anchor, true, needed, anchorStat);
    const raw = await maybeStat(base);
    if (raw) {
      const root = raw.isSymbolicLink() ? await maybeStat(base, true) : raw;
      if (root) {
        admit(base); result.set(base, stamp(root));
        if (root.isDirectory()) await visit(base, 0, root);
        else ensureWatch(base, false, needed, root);
      }
    }
    async function visit(directory, depth, stat) {
      if (depth > 64 || ++directories > MAX_DIRECTORIES) throw new Error('Local watcher recursive directory budget exceeded');
      if (!ensureWatch(directory, true, needed, stat)) return;
      let listing;
      try { listing = await disk.opendir(directory); }
      catch (reason) { if (['ENOENT', 'ENOTDIR'].includes(reason.code)) return; throw reason; }
      for await (const entry of listing) {
        const target = path.join(directory, entry.name);
        admit(target);
        if (excluded(target)) continue;
        const stat = await maybeStat(target);
        if (!stat) continue;
        result.set(target, stamp(stat));
        if (recursive && stat.isDirectory()) await visit(target, depth + 1, stat);
        // Child symlinks remain indexed links; they are never followed.
      }
    }
    for (const [target, entry] of watches) if (!needed.has(target)) {
      entry.watcher.close(); watches.delete(target);
    }
    return result;
  }
  function send() {
    while (!stopped && pending.length && inflight.size < MAX_INFLIGHT) {
      const events = [];
      let bytes = 0;
      while (pending.length && events.length < BATCH_EVENTS) {
        const candidate = pending[0], charge = Buffer.byteLength(candidate.path) + 32;
        if (events.length && bytes + charge > BATCH_BYTES) break;
        pending.shift(); queuedBytes -= charge; events.push(candidate); bytes += charge;
      }
      if (sequence >= Number.MAX_SAFE_INTEGER) throw new Error('Local watcher sequence exhausted');
      const id = ++sequence;
      inflight.set(id, bytes); parentPort.postMessage({ kind: 'events', id, events });
    }
  }
  function changes(next, observedHints) {
    const events = [], add = (type, target) => {
      const charge = Buffer.byteLength(target) + 32;
      if (events.length + pending.length >= MAX_EVENTS || queuedBytes + charge > MAX_EVENT_BYTES) {
        throw new Error('Local watcher event overflow; resubscribe after reducing scope');
      }
      events.push({ type, path: target }); queuedBytes += charge;
    };
    for (const [target] of index) if (!next.has(target)) add(3, target);
    for (const [target, after] of next) {
      const before = index.get(target);
      if (!before) add(2, target);
      else if (before.kind !== after.kind || before.dev !== after.dev || before.ino !== after.ino ||
          (after.kind !== 'directory' && (before.size !== after.size || before.mtime !== after.mtime ||
            before.ctime !== after.ctime || observedHints.has(target)))) add(1, target);
    }
    pending.push(...events); send();
  }
  async function refresh() {
    if (running || stopped) return;
    running = true; dirty = false;
    const observed = new Set(hints); hints.clear(); hintBytes = 0;
    try {
      const next = await snapshot();
      if (stopped) return;
      if (initialized) changes(next, observed);
      index = next;
      if (!initialized) { initialized = true; parentPort.postMessage({ kind: 'ready' }); }
      clearInterval(recovery);
      recovery = index.has(base) ? undefined : setInterval(schedule, 250);
    } finally {
      running = false;
      if (dirty && !stopped) schedule();
    }
  }
  parentPort.on('message', message => {
    if (message?.kind === 'ack') {
      if (!inflight.delete(message.id)) { fail(new Error('Local watcher invalid batch acknowledgement')); return; }
      try { send(); } catch (reason) { fail(reason); }
    }
  });
  try { await refresh(); } catch (reason) { fail(reason); }
}

if (!isMainThread && workerData?.localWatcher === true) runWorker().catch(reason => {
  parentPort.postMessage({ kind: 'error', message: String(reason?.message ?? reason).slice(0, 4096) });
  parentPort.close();
});
module.exports = { watchLocal, _testing: { actualWorkers: () => actual.size } };
