'use strict';
const { Position, Uri, Disposable } = require('./api-types.cjs');
const { CancellationTokenSource } = require('./provider-types.cjs');
const MAX_PROVIDERS = 128, MAX_PENDING = 8, MAX_RESULT_BYTES = 1024 * 1024;
const METHODS = Object.freeze({
  completion: ['registerCompletionItemProvider', 'provideCompletionItems'],
  hover: ['registerHoverProvider', 'provideHover'],
  definition: ['registerDefinitionProvider', 'provideDefinition'],
  references: ['registerReferenceProvider', 'provideReferences'],
  formatting: ['registerDocumentFormattingEditProvider', 'provideDocumentFormattingEdits'],
  symbols: ['registerDocumentSymbolProvider', 'provideDocumentSymbols'],
  signature: ['registerSignatureHelpProvider', 'provideSignatureHelp'],
});
function text(value, max = 4096) {
  if (typeof value !== 'string' || Buffer.byteLength(value) > max) throw new Error('Provider string exceeds its budget or has invalid type');
  return value;
}
function selector(value) {
  const entries = Array.isArray(value) ? value : [value];
  if (entries.length > 32) throw new Error('Provider selector exceeds 32 entries');
  return entries.map(entry => {
    if (typeof entry === 'string') return { language: text(entry, 128) };
    if (!entry || typeof entry !== 'object' || Object.keys(entry).some(key => !['language', 'scheme'].includes(key))) {
      throw new Error('VSCLI provider selectors currently support language and scheme only');
    }
    const result = {};
    for (const key of ['language','scheme']) if (entry[key] !== undefined) result[key] = text(entry[key], 128);
    return result;
  });
}
function score(entries, document) {
  return Math.max(0, ...entries.map(entry => {
    let score = 0;
    for (const [key, actual] of [['language', document.languageId], ['scheme', document.uri.scheme]]) {
      if (entry[key] === undefined) continue;
      if (entry[key] === actual) score = 10;
      else if (entry[key] === '*') score = Math.max(score, 5);
      else return 0;
    }
    return score;
  }));
}
function position(value, document) {
  const result = new Position(value?.line, value?.character);
  if (document) {
    const line = document.lineAt(result.line).text;
    if (result.character > line.length || (result.character > 0 && result.character < line.length && /[\uD800-\uDBFF]/.test(line[result.character - 1]) && /[\uDC00-\uDFFF]/.test(line[result.character]))) {
      throw new Error('Provider position is outside the document or splits UTF-16');
    }
  }
  return result;
}
function range(value, document) {
  const start = position(value?.start, document), end = position(value?.end, document);
  if (start.isAfter(end)) throw new Error('Provider range is reversed');
  return { start, end };
}
function list(value, max) {
  if (!Array.isArray(value) || value.length > max) throw new Error(`Provider result must be an array of at most ${max} entries`);
  return value;
}
function kind(value, maximum) {
  if (!Number.isInteger(value) || value < 0 || value > maximum) throw new Error('Invalid provider result kind');
  return value + 1; // VS Code API kinds are zero based; native menus consume LSP kinds.
}
function documentation(value) {
  if (typeof value === 'string') return text(value, 65536);
  if (!value || typeof value !== 'object' || value.isTrusted || value.supportHtml) throw new Error('Trusted or HTML provider markdown is not supported');
  if (typeof value.language === 'string') return { language: text(value.language, 128), value: text(value.value, 65536) };
  return { kind: 'markdown', value: text(value.value, 65536) };
}
function uri(value) {
  if (!(value instanceof Uri) || value.scheme !== 'file' || value.query || value.fragment || value.authority && value.authority !== 'localhost') throw new Error('Provider locations require a local file URI');
  return text(value.toString(), 4096);
}
function edit(value, document) { return { range: range(value.range, document), newText: text(value.newText, 4 * 1024 * 1024) }; }
function normalize(type, value, document) {
  if (value === undefined || value === null) return null;
  switch (type) {
    case 'completion': {
      const items = list(Array.isArray(value) ? value : value.items, 300).map(item => {
        if (item.command !== undefined) throw new Error('Completion commands are not implemented');
        const result = { label: text(typeof item.label === 'string' ? item.label : item.label?.label, 1024) };
        if (item.kind !== undefined) result.kind = kind(item.kind, 24);
        for (const key of ['detail','sortText','filterText']) if (item[key] !== undefined) result[key] = text(item[key]);
        if (item.documentation !== undefined) result.documentation = documentation(item.documentation);
        const insertion = item.textEdit !== undefined ? item.textEdit.newText : item.insertText === undefined ? result.label : typeof item.insertText === 'string' ? item.insertText : item.insertText.value;
        result.insertText = text(insertion, 65536);
        if (item.textEdit === undefined && item.insertText && typeof item.insertText === 'object') result.insertTextFormat = 2;
        if (item.textEdit !== undefined) result.textEdit = edit(item.textEdit, document);
        else if (item.range !== undefined) result.textEdit = { range: range(item.range, document), newText: result.insertText };
        if (item.additionalTextEdits !== undefined) result.additionalTextEdits = list(item.additionalTextEdits, 4096).map(value => edit(value, document));
        return result;
      });
      return { items, isIncomplete: !!value.isIncomplete };
    }
    case 'hover': {
      const contents = list(Array.isArray(value.contents) ? value.contents : [value.contents], 32).map(documentation);
      return value.range === undefined ? { contents } : { contents, range: range(value.range, document) };
    }
    case 'definition': case 'references': return list(Array.isArray(value) ? value : [value], 512).map(value => {
      if (value.targetUri !== undefined) return { targetUri: uri(value.targetUri), targetRange: range(value.targetRange), targetSelectionRange: range(value.targetSelectionRange), ...(value.originSelectionRange ? { originSelectionRange: range(value.originSelectionRange, document) } : {}) };
      return { uri: uri(value.uri), range: range(value.range) };
    });
    case 'formatting': return list(value, 4096).map(value => edit(value, document));
    case 'symbols': {
      let count = 0;
      const convert = (value, depth) => {
        if (++count > 512 || depth > 16) throw new Error('Provider symbol count or depth limit exceeded');
        const result = { name: text(value.name, 4096), kind: kind(value.kind, 25) };
        if (value.location !== undefined) return { ...result, containerName: text(value.containerName || '', 4096), location: { uri: uri(value.location.uri), range: range(value.location.range) } };
        return { ...result, detail: text(value.detail || '', 4096), range: range(value.range, document), selectionRange: range(value.selectionRange, document), children: list(value.children || [], 512).map(child => convert(child, depth + 1)) };
      };
      return list(value, 512).map(value => convert(value, 0));
    }
    case 'signature': return {
      signatures: list(value.signatures, 128).map(signature => ({
        label: text(signature.label, 65536), ...(signature.documentation !== undefined ? { documentation: documentation(signature.documentation) } : {}),
        parameters: list(signature.parameters || [], 128).map(parameter => {
          const label = typeof parameter.label === 'string' ? text(parameter.label) : list(parameter.label, 2);
          if (Array.isArray(label) && (label.length !== 2 || !label.every(Number.isSafeInteger) || label[0] < 0 || label[1] < label[0] || label[1] > signature.label.length)) throw new Error('Invalid signature parameter offsets');
          return { label, ...(parameter.documentation !== undefined ? { documentation: documentation(parameter.documentation) } : {}) };
        }),
      })), activeSignature: value.activeSignature, activeParameter: value.activeParameter,
    };
    default: throw new Error('Unknown provider type');
  }
}
function wireBudget(result) {
  const pending = [result]; let bytes = 0, nodes = 0;
  while (pending.length) {
    const value = pending.pop();
    if (++nodes > 100000) throw new Error('Language provider result exceeds its node budget');
    if (typeof value === 'string') bytes += Buffer.byteLength(value);
    else if (value && typeof value === 'object') {
      const entries = Object.entries(value);
      for (const [key, child] of entries) { bytes += Buffer.byteLength(key) + 4; pending.push(child); }
    } else bytes += 16;
    if (bytes > MAX_RESULT_BYTES) throw new Error('Language provider result exceeds 1 MiB');
  }
  if (Buffer.byteLength(JSON.stringify(result)) > MAX_RESULT_BYTES) throw new Error('Language provider result exceeds 1 MiB');
}
function createProviders(options) {
  const entries = new Map(), calls = new Map();
  let nextId = 0;
  function snapshot() { return [...entries.values()].map(({ id, owner, type, selector, triggers }) => ({ id, owner, type, selector, triggers })); }
  function publish() { options.notify('languageProviders', { session: options.session, providers: snapshot() }); }
  function forOwner(owner) {
    return Object.fromEntries(Object.entries(METHODS).map(([type, [registration, method]]) => [registration, (documentSelector, provider, ...triggers) => {
      if (entries.size >= MAX_PROVIDERS) throw new Error('Extension language provider limit reached');
      if (!provider || typeof provider[method] !== 'function') throw new TypeError(`Provider requires ${method}`);
      if (type === 'signature' && triggers.length === 1 && triggers[0] && typeof triggers[0] === 'object') {
        const metadata = triggers[0];
        if (Object.keys(metadata).some(key => !['triggerCharacters', 'retriggerCharacters'].includes(key))) throw new Error('Unsupported signature provider metadata');
        triggers = [...list(metadata.triggerCharacters || [], 16), ...list(metadata.retriggerCharacters || [], 16)];
      }
      if (triggers.length > 16 || triggers.some(value => typeof value !== 'string' || [...value].length !== 1 || Buffer.byteLength(value) > 4)) throw new Error('Provider trigger characters exceed their budget');
      const id = ++nextId;
      const entry = { id, owner, type, selector: selector(documentSelector), triggers, provider, method };
      entries.set(id, entry);
      const disposable = new Disposable(() => { entries.delete(id); for (const call of calls.values()) if (call.entry === entry) call.source.cancel(); publish(); });
      try { const result = options.track(owner, disposable); publish(); return result; }
      catch (error) { disposable.dispose(); throw error; }
    }]));
  }
  async function provide(params) {
    const entry = entries.get(params.provider);
    if (params.session !== options.session || !entry || entry.owner !== params.owner) throw new Error('Stale or invalid provider owner/session');
    const document = options.document(params.document);
    if (!document || document.isClosed || document.version !== params.version || !score(entry.selector, document)) throw new Error('Provider document changed or does not match its selector');
    if (calls.size >= MAX_PENDING) throw new Error('Language provider invocation limit reached');
    const callId = params.request === undefined ? Symbol() : params.request;
    if (typeof callId !== 'symbol' && (!Number.isSafeInteger(callId) || callId < 1 || calls.has(callId))) throw new Error('Invalid or duplicate provider request ID');
    const source = new CancellationTokenSource(), call = { source, entry, document, version: document.version };
    calls.set(callId, call);
    let args;
    try {
      switch (entry.type) {
        case 'completion': args = [document, position(params.position, document), source.token, { triggerKind: 0 }]; break;
        case 'signature': args = [document, position(params.position, document), source.token, { triggerKind: 1, isRetrigger: false }]; break;
        case 'hover': case 'definition': args = [document, position(params.position, document), source.token]; break;
        case 'references': args = [document, position(params.position, document), { includeDeclaration: !!params.includeDeclaration }, source.token]; break;
        case 'formatting':
          if (!Number.isInteger(params.options?.tabSize) || params.options.tabSize < 1 || params.options.tabSize > 32 || typeof params.options.insertSpaces !== 'boolean') throw new Error('Invalid native formatting options');
          args = [document, { tabSize: params.options.tabSize, insertSpaces: params.options.insertSpaces }, source.token]; break;
        case 'symbols': args = [document, source.token]; break;
      }
    } catch (error) { calls.delete(callId); source.dispose(); throw error; }
    let timer;
    const work = Promise.resolve().then(() => entry.provider[entry.method](...args)).then(value => {
      if (source.token.isCancellationRequested || entries.get(entry.id) !== entry || document.isClosed || document.version !== call.version) throw new Error('Language provider result became stale or canceled');
      const result = normalize(entry.type, value, document);
      wireBudget(result);
      return result;
    }).finally(() => { calls.delete(callId); source.dispose(); clearTimeout(timer); });
    // A timed-out callback keeps its slot until it settles, bounding ignored cancellation.
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => { reject(new Error('Language provider deadline exceeded')); source.cancel(); }, options.timeoutMs || 5000); });
    let cancellationListener;
    const canceled = new Promise((_, reject) => { cancellationListener = source.token.onCancellationRequested(() => reject(new Error('Language provider invocation canceled'))); });
    return Promise.race([work, timeout, canceled]).finally(() => { clearTimeout(timer); cancellationListener.dispose(); });
  }
  function cancel(params) {
    const call = calls.get(params.request);
    if (params.session !== options.session || !call || call.entry.owner !== params.owner) return false;
    call.source.cancel(); return true;
  }
  function documentChanged() { for (const { source, document, version } of calls.values()) if (document.isClosed || document.version !== version) source.cancel(); }
  function disposeOwner(owner) { for (const [id, entry] of entries) if (entry.owner === owner) { entries.delete(id); for (const call of calls.values()) if (call.entry === entry) call.source.cancel(); } publish(); }
  return { forOwner, provide, cancel, snapshot, documentChanged, disposeOwner, pendingCount: () => calls.size };
}
module.exports = { createProviders, score, selector, normalize };
