'use strict';
const { isDeepStrictEqual } = require('node:util');
const { EventEmitter, Uri } = require('./api-types.cjs');
const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const empty = () => Object.create(null);
function merge(first, second) {
  if (!record(first) || !record(second)) return structuredClone(second);
  const result = empty();
  for (const [key, value] of Object.entries(first)) result[key] = structuredClone(value);
  for (const [key, value] of Object.entries(second)) result[key] = merge(Object.hasOwn(result, key) ? result[key] : undefined, value);
  return result;
}
function lookup(value, key) {
  if (!key) return value;
  for (const part of key.split('.')) {
    if (!record(value) || !Object.hasOwn(value, part)) return undefined;
    value = value[part];
  }
  return value;
}
function tree(values) {
  const result = empty();
  for (const [key, value] of Object.entries(values)) {
    const parts = key.split('.');
    let parent = result;
    for (const part of parts.slice(0, -1)) {
      if (!Object.hasOwn(parent, part)) parent[part] = empty();
      if (!record(parent[part])) { parent = undefined; break; }
      parent = parent[part];
    }
    if (parent) Object.defineProperty(parent, parts.at(-1), { value: structuredClone(value), enumerable: true, writable: true, configurable: true });
  }
  return result;
}
function identifiers(key) {
  if (!/^(\[[^\]]+\])+$/.test(key)) return [];
  return [...new Set([...key.matchAll(/\[([^\]]+)\]/g)].map(match => match[1].trim()).filter(Boolean))];
}
const formatUri = Uri.prototype.toString;
function scopeOverride(scope) {
  if (scope instanceof Uri) return {resource:Reflect.apply(formatUri,scope,[])};
  if (scope === null) return {resource:null};
  if (!scope || (typeof scope !== 'object' && typeof scope !== 'function')) return undefined;
  const uri=scope.uri, languageId=scope.languageId;
  if (uri && !(uri instanceof Uri)) return undefined;
  if (typeof languageId === 'string' && languageId) return {languageId,resource:uri ? Reflect.apply(formatUri,uri,[]) : null};
  if (uri instanceof Uri) return {resource:Reflect.apply(formatUri,uri,[])};
  return undefined;
}
function updateValue(value) {
  let count=0;const depths=new WeakMap();
  const encoded=JSON.stringify(value,function(key,item) {
    const depth=typeof this==='object' && this ? (depths.get(this) ?? -1)+1 : 0;
    if (++count>4096 || depth>=64) throw new Error('Configuration value exceeds node/depth budget');
    if (Buffer.byteLength(key)>65536 || (typeof item==='string' && Buffer.byteLength(item)>65536)) throw new Error('Configuration value exceeds 64 KiB');
    if (item && typeof item==='object') {
      if (Array.isArray(item) && item.length>4096) throw new Error('Configuration array exceeds 4096 entries');
      depths.set(item,depth);
    }
    return item;
  });
  if (encoded===undefined || Buffer.byteLength(encoded)>65536) throw new Error('Configuration value exceeds JSON / 64 KiB bounds');
  return JSON.parse(encoded);
}

function freeze(value) {
  if (value && typeof value === 'object') {
    for (const child of Object.values(value)) freeze(child);
    Object.freeze(value);
  }
  return value;
}
function defaultFor(type) {
  switch (Array.isArray(type) ? type[0] : type) {
    case 'boolean': return false;
    case 'number': case 'integer': return 0;
    case 'string': return '';
    case 'array': return [];
    case 'object': return {};
    default: return null;
  }
}
function createConfiguration(options = {}) {
  const changed = new EventEmitter();
  let properties = empty(), defaults = empty();
  function model(source, filterWorkspace = true) {
    let contents = empty();
    const groups = new Map();
    for (let i = 0; i < source.length; i++) {
      const filtered = values => Object.fromEntries(Object.entries(values).filter(([key]) =>
        !filterWorkspace || i !== source.length - 1 || !['application', 'machine'].includes(properties[key]?.scope)));
      const general = empty();
      for (const [key, value] of Object.entries(source[i])) {
        const ids = identifiers(key);
        if (!ids.length) { general[key] = value; continue; }
        if (!record(value)) continue;
        const name = JSON.stringify(ids), previous = groups.get(name);
        const values = filtered(value);
        // Merge identical identifier groups in place. Their first occurrence
        // determines precedence relative to other combined-language groups.
        groups.set(name, { ids, keys: [...new Set([...(previous?.keys || []), ...Object.keys(values)])],
          contents: merge(previous?.contents || empty(), tree(values)) });
        general[key] = values;
      }
      contents = merge(contents, tree(filtered(general)));
    }
    const overrides = [...groups.values()];
    const cache = new Map();
    function override(id) {
      let value = empty();
      if (id) for (const single of [false, true]) for (const group of overrides) {
        if ((group.ids.length === 1) === single && group.ids.includes(id)) value = merge(value, group.contents);
      }
      return value;
    }
    return { contents, overrides, override,
      effective(id) {
        if (!cache.has(id)) cache.set(id, merge(merge(defaults, contents), override(id)));
        return cache.get(id);
      },
    };
  }
  let layers = [], current = model([]), signature = '[]';
  function affected(previous, next) {
    const keys = new Set();
    for (let i = 0; i < Math.max(previous.length, next.length); i++) {
      const before = previous[i] || {}, after = next[i] || {};
      for (const key of new Set([...Object.keys(before), ...Object.keys(after)])) {
        if (!isDeepStrictEqual(before[key], after[key])) keys.add(key);
      }
      const a = model([before], false), b = model([after], false);
      const groups = [...a.overrides, ...b.overrides];
      for (const id of new Set(groups.flatMap(group => group.ids))) {
        for (const key of new Set(groups.filter(group => group.ids.includes(id)).flatMap(group => group.keys))) {
          if (!isDeepStrictEqual(lookup(a.override(id), key), lookup(b.override(id), key))) keys.add(key);
        }
      }
    }
    return keys;
  }
  function update(next, fire = true) {
    if (!Array.isArray(next) || next.some(layer => !record(layer))) throw new TypeError('Invalid configuration layers');
    const nextSignature = JSON.stringify(next);
    if (signature === nextSignature) return;
    const previous = current, changedKeys = affected(layers, next);
    layers = structuredClone(next); signature = nextSignature;
    const updated = model(layers);
    current = updated;
    if (fire && changedKeys.size) changed.fire(Object.freeze({
      affectsConfiguration(section, scope) {
        if (![...changedKeys].some(key => key === section || key.startsWith(`${section}.`))) return false;
        const overrides = scopeOverride(scope);
        return overrides === undefined || !isDeepStrictEqual(
          lookup(previous.effective(overrides.languageId), section), lookup(updated.effective(overrides.languageId), section));
      },
    }));
  }
  function inspect(key, id) {
    const global = model(layers.slice(0, -1), false), workspace = model(layers.slice(-1));
    const ids = [...new Set(current.overrides.flatMap(group => group.ids))]
      .filter(id => lookup(current.override(id), key) !== undefined);
    return { key, defaultValue: structuredClone(lookup(defaults, key)),
      globalValue: structuredClone(lookup(global.contents, key)),
      workspaceValue: structuredClone(lookup(workspace.contents, key)), workspaceFolderValue: undefined,
      defaultLanguageValue: undefined,
      globalLanguageValue: id ? structuredClone(lookup(global.override(id), key)) : undefined,
      workspaceLanguageValue: id ? structuredClone(lookup(workspace.override(id), key)) : undefined,
      workspaceFolderLanguageValue: undefined, languageIds: ids.length ? ids : undefined };
  }
  return {
    initialize(configuration, source = []) {
      properties = Object.assign(empty(), {
        'editor.tabSize': { default: 4, scope: 'language-overridable' },
        'editor.insertSpaces': { default: true, scope: 'language-overridable' },
        'editor.lineNumbers': { default: 'on', scope: 'language-overridable' },
        'editor.quickSuggestions': { default: true, scope: 'language-overridable' },
        'editor.quickSuggestionsDelay': { default: 120, scope: 'language-overridable' },
        'editor.suggestOnTriggerCharacters': { default: true, scope: 'language-overridable' },
        'editor.acceptSuggestionOnEnter': { default: 'on', scope: 'language-overridable' },
      });
      for (const group of Array.isArray(configuration) ? configuration : [configuration]) {
        for (const [key, property] of Object.entries(group?.properties || {})) {
          if (!record(property)) throw new TypeError(`Invalid configuration property: ${key}`);
          if (!Object.hasOwn(properties, key)) properties[key] = { ...property, scope: property.scope || group.scope || 'window' };
        }
      }
      const values = empty();
      for (const [key, property] of Object.entries(properties)) values[key] = Object.hasOwn(property, 'default') ? property.default : defaultFor(property.type);
      defaults = tree(values); signature = undefined;
      update(source, false);
    },
    update, onDidChange: changed.event,
    get(section = '', scope, owner) {
      const guard=owner && options.captureCaller ? options.captureCaller(owner) : undefined;
      const overrides=scopeOverride(scope) || {},languageId=overrides.languageId;
      const snapshot = lookup(current.effective(languageId), section);
      const result = {
        get(key, fallback) {
          const value = key ? lookup(snapshot, key) : undefined;
          return value === undefined ? fallback : structuredClone(value);
        },
        has(key) { return Boolean(key) && lookup(snapshot, key) !== undefined; },
        inspect(key) { return inspect(section ? `${section}.${key}` : key, languageId); },
        async update(key,value,target,overrideInLanguage) {
          if (options.enabled !== true) throw new Error('VSCLI configuration writes are not implemented by this native host');
          if (!owner || !guard) throw new Error('Configuration writes require an extension owner');
          guard();
          if (typeof key !== 'string' || !key || key.includes('\0')) throw new TypeError('Invalid configuration key');
          const fullKey=section ? `${section}.${key}` : key;
          if (Buffer.byteLength(fullKey)>1024) throw new Error('Configuration key exceeds 1 KiB');
          const destination=target===true || target===1 ? 1 : target===3 ? 3 : 2;
          let language=null;
          if (languageId && overrideInLanguage !== false) {
            const current=inspect(fullKey,languageId);
            const prior=destination===1 ? current.globalLanguageValue : current.workspaceLanguageValue;
            if (overrideInLanguage===true || prior!==undefined) language=languageId;
          }
          const remove=value===undefined;
          const snapshot=remove ? null : updateValue(value);
          guard();
          const result=await options.write(owner,{key:fullKey,value:snapshot,remove,target:destination,language,resource:overrides.resource ?? null});
          guard();
          if (result!==null) throw new Error('Invalid native configuration update acknowledgement');
        },
      };
      if (record(snapshot)) for (const [key, value] of Object.entries(snapshot)) {
        if (!Object.hasOwn(result, key)) Object.defineProperty(result, key, { value: freeze(structuredClone(value)), enumerable: true });
      }
      return Object.freeze(result);
    },
  };
}
module.exports = { createConfiguration };
