'use strict';
const { Uri, Position, Disposable, EventEmitter } = require('./api-types.cjs');
const MAX_BYTES = 2 * 1024 * 1024, MAX_DIAGNOSTICS = 5000, MAX_RESOURCES = 128, MAX_COLLECTIONS = 64;
function text(value, limit = 4096) {
  if (typeof value !== 'string' || Buffer.byteLength(value) > limit) throw new TypeError('Diagnostic text exceeds its budget or has invalid type');
  return value;
}
function uriKey(uri) {
  if (!(uri instanceof Uri)) throw new TypeError('Diagnostics require a URI');
  return text(uri.toString(), 4096);
}
function position(value, document) {
  const pos = new Position(value?.line, value?.character);
  if (pos.line > 0x7fffffff || pos.character > 0x7fffffff) throw new Error('Diagnostic coordinates exceed LSP uinteger bounds');
  if (document) {
    const line = document.lineAt(pos.line).text;
    if (pos.character > line.length || (pos.character > 0 && pos.character < line.length && /[\uD800-\uDBFF]/.test(line[pos.character - 1]) && /[\uDC00-\uDFFF]/.test(line[pos.character]))) throw new Error('Diagnostic position is outside the document or splits UTF-16');
  }
  return { line: pos.line, character: pos.character };
}
function range(value, document) {
  const start = position(value?.start, document), end = position(value?.end, document);
  if (start.line > end.line || start.line === end.line && start.character > end.character) throw new Error('Diagnostic range is reversed');
  return { start, end };
}
function code(value) {
  if (typeof value === 'string') return text(value);
  if (typeof value === 'number' && Number.isSafeInteger(value)) return value;
  throw new TypeError('Diagnostic code requires bounded text or a safe integer');
}
// User-owned arrays can run getters that grow their length. Every loop uses
// one validated length and copies into an ordinary array before further work.
function copyArray(value, limit, description) {
  if (!Array.isArray(value)) throw new Error(`Invalid ${description}`);
  const length = value.length;
  if (!Number.isInteger(length) || length < 0 || length > limit) throw new Error(`${description} exceeds ${limit} entries`);
  const result = [];
  for (let index = 0; index < length; index++) result.push(value[index]);
  if (value.length !== length) throw new Error(`${description} changed during normalization`);
  return result;
}
function normalize(value, document) {
  if (!value || typeof value !== 'object') throw new TypeError('Invalid diagnostic');
  const severity = value.severity;
  if (!Number.isInteger(severity) || severity < 0 || severity > 3) throw new TypeError('Invalid diagnostic severity');
  const result = { range: range(value.range, document), message: text(value.message), severity: severity + 1 };
  const source = value.source, identifier = value.code, tags = value.tags, relatedInformation = value.relatedInformation;
  if (source !== undefined) result.source = text(source, 256);
  if (identifier !== undefined) {
    if (typeof identifier === 'object' && identifier !== null) {
      result.code = code(identifier.value);
      const target = identifier.target;
      if (!(target instanceof Uri) || !['file', 'http', 'https'].includes(target.scheme)) throw new Error('Unsupported diagnostic information URI');
      result.codeDescription = { href: uriKey(target) };
    } else result.code = code(identifier);
  }
  if (tags !== undefined) {
    const values = copyArray(tags, 2, 'Diagnostic tags');
    if (values.some(tag => tag !== 1 && tag !== 2)) throw new Error('Invalid diagnostic tags');
    result.tags = values;
  }
  if (relatedInformation !== undefined) {
    result.relatedInformation = copyArray(relatedInformation, 32, 'Diagnostic related information').map(related => {
      const location = related.location;
      return { message: text(related.message), location: { uri: uriKey(location?.uri), range: range(location?.range) } };
    });
  }
  return result;
}
function createDiagnostics(options) {
  const collections = new Map(), ownerGenerations = new Map(), pendingOwners = new Set(), events = new EventEmitter();
  const changedUris = new Map(); let nextId = 0, publishQueued = false, eventTimer;
  function alive(collection) { if (collection.disposed) throw new Error('illegal state - object is disposed'); options.assertOwner(collection.owner); }
  function allEntries(replace, next) {
    return [...collections.values()].flatMap(collection => [...(collection === replace ? next : collection.entries).values()]);
  }
  function checkBudget(collection, next) {
    const entries = allEntries(collection, next);
    if (new Set(entries.map(entry => entry.key)).size > MAX_RESOURCES || entries.reduce((count, entry) => count + entry.original.length, 0) > MAX_DIAGNOSTICS) throw new Error('Diagnostic resource or count budget exceeded');
    let bytes = 0;
    for (const entry of entries) bytes += Buffer.byteLength(JSON.stringify(entry.normalized)) + Buffer.byteLength(entry.key) + 64;
    for (const col of collections.values()) bytes += Buffer.byteLength(col.name) + 128;
    if (bytes > MAX_BYTES) throw new Error('Diagnostic collections exceed 2 MiB');
  }
  function changed(uris) {
    // Keys were normalized before mutation; publication never invokes URI getters.
    for (const { key, uri } of uris) changedUris.set(key, uri);
    if (changedUris.size > 2 * MAX_RESOURCES) throw new Error('Diagnostic change event exceeds 128 resources');
    if (changedUris.size && !eventTimer) eventTimer = setTimeout(() => {
      eventTimer = undefined;
      const uris = Object.freeze([...changedUris.values()]); changedUris.clear(); events.fire({ uris });
    }, 50);
  }
  function queue(owner) {
    ownerGenerations.set(owner, (ownerGenerations.get(owner) || 0) + 1); pendingOwners.add(owner);
    if (publishQueued) return;
    publishQueued = true;
    queueMicrotask(() => {
      publishQueued = false;
      for (const current of [...pendingOwners]) {
        pendingOwners.delete(current);
        const owned = [...collections.values()].filter(collection => collection.owner === current);
        const wire = owned.map(collection => ({ id: collection.id, name: collection.name, entries: [...collection.entries.values()]
          .filter(entry => entry.document && !entry.document.isClosed && options.document(entry.key) === entry.document && entry.document.version === entry.version)
          .map(entry => ({ document: entry.document._snapshot.id, version: entry.version, diagnostics: entry.normalized })) }));
        options.notify('diagnosticCollections', { session: options.session, owner: current, generation: ownerGenerations.get(current), collections: wire });
      }
    });
  }
  function prepare(uri, diagnostics) {
    const key = uriKey(uri), document = options.document(key);
    if (!['file', 'untitled'].includes(uri.scheme)) throw new Error('Diagnostics require a file or untitled URI');
    const version = document?.version, original = copyArray(diagnostics, MAX_DIAGNOSTICS, 'Diagnostic list');
    const normalized = []; let bytes = 0;
    for (const value of original) {
      const item = normalize(value, document); bytes += Buffer.byteLength(JSON.stringify(item));
      if (bytes > MAX_BYTES) throw new Error('Diagnostic collections exceed 2 MiB');
      normalized.push(item);
    }
    if (document && (document.isClosed || document.version !== version || options.document(key) !== document)) throw new Error('Diagnostic document changed during normalization');
    return { key, uri, document, version, original, normalized };
  }
  function create(owner, name = '') {
    options.assertOwner(owner);
    if (collections.size >= MAX_COLLECTIONS || nextId >= Number.MAX_SAFE_INTEGER) throw new Error('Diagnostic collection limit reached (64)');
    text(name, 128);
    const collection = { id: ++nextId, owner, name, disposed: false, generation: 0, entries: new Map() };
    let tracked;
    const facade = {
      get name() { alive(collection); return name; },
      set(first, diagnostics) {
        alive(collection); const expected = collection.generation, next = new Map(collection.entries), touched = new Map(); let uris = [];
        if (!first) { this.clear(); return; }
        if (first instanceof Uri) {
          if (!diagnostics) { this.delete(first); return; }
          const entry = prepare(first, diagnostics); next.set(entry.key, entry); touched.set(entry.key, entry); uris = [entry];
        } else {
          const tuples = copyArray(first, MAX_RESOURCES, 'Diagnostic bulk update').map(tuple => {
            if (!Array.isArray(tuple) || tuple.length !== 2) throw new Error('Invalid diagnostic bulk tuple');
            const uri = tuple[0], key = uriKey(uri), values = tuple[1];
            return { uri, key, values };
          }).sort((a, b) => a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
          let previous, group = [], inputCount = 0;
          function finish(final) {
            if (!previous) return;
            if (!final && group.length === 0) { next.delete(previous.key); return; }
            const entry = prepare(previous.uri, group); next.set(previous.key, entry); touched.set(previous.key, entry);
          }
          for (const tuple of tuples) {
            if (previous?.key !== tuple.key) { finish(false); previous = tuple; group = []; uris.push(tuple); }
            if (tuple.values === undefined || tuple.values === null) group = [];
            else {
              const values = copyArray(tuple.values, MAX_DIAGNOSTICS - inputCount, 'Diagnostic bulk input'); inputCount += values.length;
              for (const value of values) group.push(value);
            }
          }
          finish(true);
        }
        checkBudget(collection, next); alive(collection);
        for (const entry of touched.values()) if (entry.document && (entry.document.isClosed || entry.document.version !== entry.version || options.document(entry.key) !== entry.document)) throw new Error('Diagnostic document changed during normalization');
        if (collection.generation !== expected) throw new Error('Diagnostic collection changed during normalization');
        if (new Set([...changedUris.keys(), ...uris.map(entry => entry.key)]).size > MAX_RESOURCES) throw new Error('Diagnostic change event exceeds 128 resources');
        collection.entries = next; collection.generation++; changed(uris); queue(owner);
      },
      delete(uri) {
        alive(collection); const expected = collection.generation, key = uriKey(uri);
        alive(collection); if (collection.generation !== expected) throw new Error('Diagnostic collection changed during URI normalization');
        if (!changedUris.has(key) && changedUris.size >= MAX_RESOURCES) throw new Error('Diagnostic change event exceeds 128 resources');
        collection.entries.delete(key); collection.generation++; changed([{ key, uri }]); queue(owner);
      },
      clear() { alive(collection); const uris = [...collection.entries.values()]; if (new Set([...changedUris.keys(), ...uris.map(entry => entry.key)]).size > MAX_RESOURCES) throw new Error('Diagnostic change event exceeds 128 resources'); collection.entries.clear(); collection.generation++; changed(uris); queue(owner); },
      get(uri) { alive(collection); const entry = collection.entries.get(uriKey(uri)); return entry ? Object.freeze(entry.original.slice()) : []; },
      has(uri) { alive(collection); return collection.entries.has(uriKey(uri)); },
      forEach(callback, thisArg) { alive(collection); for (const [uri, diagnostics] of this) callback.call(thisArg, uri, diagnostics, this); },
      *[Symbol.iterator]() { alive(collection); for (const entry of collection.entries.values()) { alive(collection); yield [entry.uri, Object.freeze(entry.original.slice())]; } },
      dispose() { if (!collection.disposed) tracked.dispose(); },
    };
    tracked = options.track(owner, new Disposable(() => {
      if (collection.disposed) return;
      const uris = [...collection.entries.values()]; collection.disposed = true; collection.generation++; collections.delete(collection.id); collection.entries.clear(); try { changed(uris); } finally { queue(owner); }
    }));
    collection.dispose = () => tracked.dispose(); collections.set(collection.id, collection); queue(owner); return facade;
  }
  function getDiagnostics(uri) {
    if (uri !== undefined) {
      const key = uriKey(uri), result = [];
      for (const collection of collections.values()) { const entry = collection.entries.get(key); if (entry) result.push(...entry.original); }
      return result;
    }
    const result = new Map();
    for (const collection of collections.values()) for (const entry of collection.entries.values()) {
      if (!result.has(entry.key)) result.set(entry.key, [entry.uri, []]);
      result.get(entry.key)[1].push(...entry.original);
    }
    return [...result.values()];
  }
  let versions = new Map();
  function documentChanged() {
    const affected = new Set(), next = new Map(), changedKeys = new Set();
    for (const collection of collections.values()) for (const entry of collection.entries.values()) {
      const current = options.document(entry.key), stamp = current && !current.isClosed ? `${current._snapshot.id}:${current.version}` : undefined;
      next.set(entry.key, stamp);
      if (versions.get(entry.key) !== stamp) changedKeys.add(entry.key);
    }
    for (const collection of collections.values()) for (const entry of collection.entries.values()) if (changedKeys.has(entry.key)) affected.add(collection.owner);
    versions = next;
    for (const owner of affected) queue(owner);
  }
  function disposeOwner(owner) { for (const collection of [...collections.values()]) if (collection.owner === owner) collection.dispose(); queue(owner); }
  function currentDiagnostics(document, requested) {
    const result = [], anchors = [];
    for (const collection of collections.values()) {
      const entry = collection.entries.get(document._snapshot.uri);
      if (!entry || entry.document !== document || entry.version !== document.version || document.isClosed) continue;
      anchors.push({collection,generation:collection.generation,entry});
      for (const original of entry.original) {
        const diagnosticRange = range(original.range,document);
        const intersects = (diagnosticRange.start.line < requested.end.line || diagnosticRange.start.line === requested.end.line && diagnosticRange.start.character <= requested.end.character)
          && (diagnosticRange.end.line > requested.start.line || diagnosticRange.end.line === requested.start.line && diagnosticRange.end.character >= requested.start.character);
        if (intersects) { if (result.length >= 128) throw new Error('More than128 diagnostics intersect selection'); result.push(original); }
      }
    }
    for (const {collection,generation,entry} of anchors) if (collection.disposed || collection.generation !== generation || entry.document !== document || entry.version !== document.version || document.isClosed) throw new Error('Diagnostic context changed during normalization');
    return result;
  }
  return { forOwner: owner => ({ createDiagnosticCollection: name => create(owner, name), getDiagnostics, onDidChangeDiagnostics: (listener, thisArg, disposables) => {
    options.assertOwner(owner); const disposable = options.track(owner, events.event(listener, thisArg)); if (disposables) disposables.push(disposable); return disposable;
  } }), currentDiagnostics, documentChanged, disposeOwner };
}
module.exports = { createDiagnostics, normalize };
