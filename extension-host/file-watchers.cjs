'use strict';
const path = require('node:path');
const { Uri, Disposable, EventEmitter } = require('./api-types.cjs');
const { compileGlob } = require('./watcher-glob.cjs');
// Keep worker_threads and the OS backend cold until an extension starts a watch.
const watchLocal = (...args) => require('./local-file-watcher.cjs').watchLocal(...args);
const DEFAULT_EXCLUDES = {
  '**/.git/objects/**':true, '**/.git/subtree-cache/**':true,
  '**/node_modules/*/**':true, '**/.hg/store/**':true,
};
function boundedPattern(value) {
  if (typeof value !== 'string' || Buffer.byteLength(value) > 4096 || value.includes('\0'))
    throw new TypeError('Invalid or oversized watcher glob');
  return value;
}
function snapshotUri(value) {
  if (!Uri.isUri(value)) throw new TypeError('Watcher base must be a URI');
  const fields = {};
  for (const key of ['scheme','authority','path','query','fragment']) {
    const component = value[key];
    if (typeof component !== 'string' || Buffer.byteLength(component) > 4096 || component.includes('\0'))
      throw new TypeError('Invalid or oversized watcher base URI');
    fields[key] = component;
  }
  if (fields.query || fields.fragment) throw new Error('Watcher base must not have query or fragment');
  return Uri.from(fields, true);
}
function createFileWatchers(options) {
  const watches = new Set();
  const sessions = new Map();
  const unowned = options.unowned || (callback => callback());
  const backend = options.localWatch || watchLocal;
  function subscribeLocal(base, settings, emit, error) {
    const key = JSON.stringify([base.toString(), settings]);
    let session = sessions.get(key);
    const listener = {emit,error};
    if (!session) {
      session = {listeners:new Set([listener]),child:undefined};
      sessions.set(key,session);
      try {
        session.child = backend(base,settings,
          event=>{if(!session.failed) for(const current of [...session.listeners]) current.emit(event);},
          failure=>{
            session.failed = failure;
            if(sessions.get(key) === session) sessions.delete(key);
            for(const current of [...session.listeners]) current.error(failure);
          });
      } catch (failure) {sessions.delete(key);throw failure;}
    } else session.listeners.add(listener);
    return new Disposable(()=>{
      session.listeners.delete(listener);
      if (!session.listeners.size) {
        if (sessions.get(key) === session) sessions.delete(key);
        session.child.dispose();
      }
    });
  }
  function excludes() {
    const configured = options.configuration().get('watcherExclude', {});
    if (!configured || typeof configured !== 'object' || Array.isArray(configured))
      throw new TypeError('files.watcherExclude must be an object');
    if (Object.keys(configured).length > 128) throw new Error('Watcher exclude limit exceeded');
    const merged = {...DEFAULT_EXCLUDES,...configured};
    return Object.entries(merged).filter(([,enabled]) => enabled === true).map(([key]) => boundedPattern(key));
  }
  function forOwner(owner) {
    return {
      createFileSystemWatcher(globPattern, ignoreCreateEvents = false, ignoreChangeEvents = false, ignoreDeleteEvents = false) {
        const guard = options.captureCaller(owner); guard();
        if (watches.size >= 64) throw new Error('Extension watcher limit exceeded');
        const string = typeof globPattern === 'string';
        let pattern, explicitBase;
        if (string) pattern = boundedPattern(globPattern);
        else {
          if (!globPattern || typeof globPattern !== 'object') throw new TypeError('Invalid watcher glob');
          const baseUri = globPattern.baseUri;
          const base = baseUri == null ? globPattern.base : undefined;
          pattern = boundedPattern(globPattern.pattern);
          explicitBase = snapshotUri(baseUri ?? (typeof base === 'string' ? Uri.file(base) : undefined));
        }
        guard();
        const ignores = [!!ignoreChangeEvents,!!ignoreCreateEvents,!!ignoreDeleteEvents];
        const emitters = [new EventEmitter(),new EventEmitter(),new EventEmitter()];
        let closed = false, child, signature, incarnation = 0;
        const recursive = string || pattern.includes('**') || pattern.includes('/');
        function stop() { incarnation++; child?.dispose(); child = undefined; }
        function start() {
          if (closed || ignores.every(Boolean)) return;
          guard();
          const root = options.workspaceRoot();
          const base = explicitBase || (root ? Uri.file(root) : undefined);
          const filtered = recursive ? excludes() : [];
          const next = JSON.stringify([base?.toString(),filtered]);
          if (signature === next) return;
          stop(); signature = next;
          if (!base) return;
          const epoch = incarnation;
          const insensitive = options.fileSystem.pathCaseSensitive(base.scheme) === false;
          const match = compileGlob(insensitive ? pattern.toLowerCase() : pattern);
          const excluded = filtered.map(glob => compileGlob(insensitive ? glob.toLowerCase() : glob));
          const basePath = base.scheme === 'file' ? base.fsPath.replaceAll(path.sep,'/') : base.path;
          const baseName = insensitive ? basePath.toLowerCase() : basePath;
          const accepts = uri => {
            if (uri.scheme !== base.scheme || uri.authority.toLowerCase() !== base.authority.toLowerCase()) return false;
            const actual = uri.scheme === 'file' ? uri.fsPath.replaceAll(path.sep,'/') : uri.path;
            const compared = insensitive ? actual.toLowerCase() : actual;
            if (compared !== baseName && !compared.startsWith(baseName.replace(/\/$/,'')+'/')) return false;
            if (string) return match(compared) && !excluded.some(check=>check(compared));
            const relative = compared.slice(baseName.replace(/\/$/,'').length+1);
            // Upstream permits watching a file itself with RelativePattern(file,'*').
            return match(compared === baseName ? '' : relative)
              && !excluded.some(check=>check(compared));
          };
          const emit = event => unowned(() => {
            if (closed || epoch !== incarnation || ![1,2,3].includes(event.type) || ignores[event.type-1]) return;
            try { guard(); } catch { stop(); return; }
            if (accepts(event.uri)) emitters[event.type-1].fire(event.uri);
          });
          const failed = error => unowned(() => {
            if (!closed && epoch === incarnation) options.diagnostic?.(owner,error);
          });
          child = unowned(() => base.scheme === 'file'
            ? subscribeLocal(base,{recursive,excludes:filtered},emit,failed)
            : options.fileSystem.watchProvider(owner,base,{recursive,excludes:filtered},emit));
          guard();
        }
        const entry = {owner,start,dispose() {
          if (!closed) {
            closed = true; watches.delete(entry);
            for (const emitter of emitters) emitter.dispose();
          }
          stop();
        }};
        watches.add(entry);
        let tracked;
        try {
          start(); guard(); tracked = options.track(owner,new Disposable(entry.dispose));
          return Object.freeze({
            get ignoreCreateEvents() {return ignores[1];},
            get ignoreChangeEvents() {return ignores[0];},
            get ignoreDeleteEvents() {return ignores[2];},
            onDidCreate:options.event(owner,emitters[1].event),
            onDidChange:options.event(owner,emitters[0].event),
            onDidDelete:options.event(owner,emitters[2].event),
            dispose:()=>tracked.dispose(),
          });
        } catch (error) { entry.dispose(); throw error; }
      },
    };
  }
  return {forOwner,refresh() {
    for (const watch of [...watches]) {
      try {watch.start();} catch(error) {options.diagnostic?.(watch.owner,error);watch.dispose();}
    }
  },disposeOwner(owner) {
    const errors = [];
    for (const watch of [...watches]) if (watch.owner === owner) {
      try { watch.dispose(); } catch (error) { errors.push(error); }
    }
    if (errors.length) throw new AggregateError(errors, 'Filesystem watcher cleanup failed');
  }};
}
module.exports = { createFileWatchers };
