'use strict';
const { Position, Uri, Disposable } = require('./api-types.cjs');
const { createActions } = require('./code-actions.cjs');
const { CodeActionKind } = require('./code-action-types.cjs');
const signatureTypes = require('./signatures.cjs');
const { CancellationTokenSource, SnippetString } = require('./provider-types.cjs');
const MAX_PROVIDERS = 128, MAX_PENDING = 8, MAX_RESULT_BYTES = 1024 * 1024;
const METHODS = Object.freeze({
  codeaction: ['registerCodeActionsProvider', 'provideCodeActions'],
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
        if (!item || typeof item !== 'object') throw new Error('Invalid completion item');
        if (item.command !== undefined) throw new Error('Completion commands are not implemented');
        if (item.commitCharacters !== undefined || item.keepWhitespace !== undefined) throw new Error('Completion commit characters and whitespace controls are not implemented');
        const result = { label: text(typeof item.label === 'string' ? item.label : item.label?.label, 1024) };
        if (item.preselect !== undefined) { if (typeof item.preselect !== 'boolean') throw new Error('Invalid completion preselect'); result.preselect = item.preselect; }
        if (item.kind !== undefined) result.kind = kind(item.kind, 24);
        for (const key of ['detail','sortText','filterText']) if (item[key] !== undefined) result[key] = text(item[key]);
        if (item.documentation !== undefined) result.documentation = documentation(item.documentation);
        if (item.textEdit === undefined && item.insertText !== undefined && typeof item.insertText !== 'string' && !(item.insertText instanceof SnippetString)) throw new Error('Completion insertText requires a string or SnippetString');
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
    case 'signature': return signatureTypes.normalize(value);
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
  const entries = new Map(), calls = new Map(), completions = new Map();
  let nextId = 0, nextHandle = 0, registryEpoch = 0;
  const actions = createActions({ ...options, text, range, position, current: item => {
    if (entries.get(item.entry.id) !== item.entry || item.epoch !== registryEpoch || item.document.isClosed || item.document.version !== item.version) return false;
    try { actions.assertWorkspace(item.workspace, options); return true; } catch { return false; }
  } });
  const signatures = signatureTypes.createSignatures({ ...options, position, assertCurrent,
    registered: (entry, epoch) => entries.get(entry.id) === entry && registryEpoch === epoch });
  function snapshot() { return [...entries.values()].map(({ id, owner, type, selector, triggers, retriggers, resolves, actionKinds }) => ({ id, owner, type, selector, triggers, resolves, ...(type === 'signature' ? {retriggers} : {}), ...(actionKinds ? {actionKinds} : {}) })); }
  function publish() { registryEpoch++; completions.clear(); actions.clear(); signatures.clear(); for (const call of calls.values()) if (call.completionOrigin !== undefined || call.actionOrigin !== undefined || call.entry.type === 'signature') call.source.cancel(); options.notify('languageProviders', { session: options.session, providers: snapshot() }); }
  function forOwner(owner) {
    return Object.fromEntries(Object.entries(METHODS).map(([type, [registration, method]]) => [registration, (documentSelector, provider, ...triggers) => {
      if (entries.size >= MAX_PROVIDERS) throw new Error('Extension language provider limit reached');
      if (!provider || typeof provider[method] !== 'function') throw new TypeError(`Provider requires ${method}`);
      let actionKinds, retriggers = [];
      if (type === 'codeaction') {
        if (triggers.length > 1) throw new Error('Invalid code action metadata');
        const metadata = triggers[0]; triggers = [];
        if (metadata !== undefined) {
          if (!metadata || typeof metadata !== 'object' || Object.keys(metadata).some(key => !['providedCodeActionKinds','documentation'].includes(key))) throw new Error('Unsupported code action metadata');
          if (metadata.documentation !== undefined) throw new Error('Code action provider documentation is unsupported');
          if (metadata.providedCodeActionKinds !== undefined) actionKinds = require('./code-actions.cjs').array(metadata.providedCodeActionKinds,32).map(kind => { if (!(kind instanceof CodeActionKind)) throw new Error('Code action metadata requires CodeActionKind'); return kind.value; });
        }
      }
      if (type === 'signature' && triggers.length === 1 && triggers[0] && typeof triggers[0] === 'object') {
        const metadata = triggers[0];
        if (Object.keys(metadata).some(key => !['triggerCharacters', 'retriggerCharacters'].includes(key))) throw new Error('Unsupported signature provider metadata');
        const triggerCharacters = metadata.triggerCharacters, retriggerCharacters = metadata.retriggerCharacters;
        triggers = signatureTypes.characters(triggerCharacters === undefined ? [] : triggerCharacters);
        retriggers = signatureTypes.characters(retriggerCharacters === undefined ? [] : retriggerCharacters);
      }
      if (type === 'signature') triggers = signatureTypes.characters(triggers);
      if (triggers.length > 16 || triggers.some(value => typeof value !== 'string' || [...value].length !== 1 || Buffer.byteLength(value) > 4)) throw new Error('Provider trigger characters exceed their budget');
      const id = ++nextId;
      const entry = { id, owner, type, selector: selector(documentSelector), triggers, retriggers, provider, method, actionKinds, resolves: type === 'completion' && typeof provider.resolveCompletionItem === 'function' || type === 'codeaction' && typeof provider.resolveCodeAction === 'function' };
      entries.set(id, entry);
      const disposable = new Disposable(() => { entries.delete(id); for (const call of calls.values()) if (call.entry === entry) call.source.cancel(); publish(); });
      try { const result = options.track(owner, disposable); publish(); return result; }
      catch (error) { disposable.dispose(); throw error; }
    }]));
  }
  function assertCurrent(call) {
    if (call.source.token.isCancellationRequested || entries.get(call.entry.id) !== call.entry || (['completion','codeaction','signature'].includes(call.entry.type) && registryEpoch !== call.epoch) || call.document.isClosed || call.document.version !== call.version) throw new Error('Language provider result became stale or canceled');
    if (call.entry.type === 'signature' && (options.document(call.documentId) !== call.document || call.document.uri.toString() !== call.uri)) throw new Error('Signature document identity changed');
    if (call.workspace) actions.assertWorkspace(call.workspace, options);
  }
  function purgeCompletions() {
    for (const [id, cached] of completions) if (Date.now() - cached.started >= 6000 || cached.document.isClosed || cached.document.version !== cached.version || entries.get(cached.entry.id) !== cached.entry) completions.delete(id);
  }
  function retainCompletions(entry, call, originals, result, request) {
    purgeCompletions();
    // One bounded cohort per provider; new lists retire its preceding item handles.
    for (const [id, cached] of completions) if (cached.entry === entry) completions.delete(id);
    if (completions.size + result.items.length > 300) throw new Error('Completion resolve handle budget exceeded (300)');
    let bytes = Buffer.byteLength(JSON.stringify(result));
    for (const cached of completions.values()) bytes += cached.bytes;
    if (bytes > 2 * MAX_RESULT_BYTES) throw new Error('Completion resolve cache exceeds 2 MiB');
    const staged = result.items.map((item, index) => {
      if (nextHandle >= Number.MAX_SAFE_INTEGER) throw new Error('Completion handle exhausted');
      const handle = ++nextHandle;
      return [handle, { entry, document: call.document, version: call.version, request,
        original: originals[index], snapshot: JSON.stringify(item), bytes: Buffer.byteLength(JSON.stringify(item)), started: Date.now() }];
    });
    for (let index = 0; index < staged.length; index++) { completions.set(...staged[index]); result.items[index]._vscliCompletionHandle = staged[index][0]; }
  }
  async function provide(params) {
    const signature = params.signatureRequest === true || entries.get(params.provider)?.type === 'signature';
    let accepted;
    try { return await performProvide(params, call => { accepted = call; }); }
    finally { if (signature && !accepted?.signatureWork) signatures.released(params, accepted); }
  }
  async function performProvide(params, accepted) {
    const entry = entries.get(params.provider);
    if (params.session !== options.session || !entry || entry.owner !== params.owner) throw new Error('Stale or invalid provider owner/session');
    const document = options.document(params.document);
    if (!document || document.isClosed || document.version !== params.version || !score(entry.selector, document)) throw new Error('Provider document changed or does not match its selector');
    if (calls.size >= MAX_PENDING) throw new Error('Language provider invocation limit reached');
    const callId = params.request === undefined ? Symbol() : params.request;
    if (typeof callId !== 'symbol' && (!Number.isSafeInteger(callId) || callId < 1 || calls.has(callId))) throw new Error('Invalid or duplicate provider request ID');
    const source = new CancellationTokenSource(), call = { source, entry, document, documentId: params.document, uri: document.uri.toString(), version: document.version, epoch: registryEpoch };
    calls.set(callId, call); accepted(call);
    let args;
    try {
      switch (entry.type) {
        case 'codeaction': args = actions.args(params,call); assertCurrent(call); break;
        case 'completion': {
          const context = params.completionContext;
          if (context !== undefined && context !== null && (!Number.isInteger(context.triggerKind) || context.triggerKind < 1 || context.triggerKind > 3 || Object.keys(context).some(key => !['triggerKind', 'triggerCharacter'].includes(key)))) throw new TypeError('Unsupported completion context');
          const kind = context?.triggerKind ?? 1;
          if (kind === 2 && (typeof context.triggerCharacter !== 'string' || [...context.triggerCharacter].length !== 1 || Buffer.byteLength(context.triggerCharacter) > 4)) throw new TypeError('Invalid completion trigger character');
          args = [document, position(params.position, document), source.token, { triggerKind: kind - 1, ...(kind === 2 ? { triggerCharacter: context.triggerCharacter } : {}) }]; break;
        }
        case 'signature': signatures.reserve(call, callId); args = signatures.args(params, call); break;
        case 'hover': case 'definition': args = [document, position(params.position, document), source.token]; break;
        case 'references': args = [document, position(params.position, document), { includeDeclaration: !!params.includeDeclaration }, source.token]; break;
        case 'formatting':
          if (!Number.isInteger(params.options?.tabSize) || params.options.tabSize < 1 || params.options.tabSize > 32 || typeof params.options.insertSpaces !== 'boolean') throw new Error('Invalid native formatting options');
          args = [document, { tabSize: params.options.tabSize, insertSpaces: params.options.insertSpaces }, source.token]; break;
        case 'symbols': args = [document, source.token]; break;
      }
    } catch (error) { calls.delete(callId); source.dispose(); throw error; }
    let timer;
    const work = Promise.resolve().then(() => { assertCurrent(call); const callback = entry.provider[entry.method]; if (entry.type === 'signature') assertCurrent(call); return callback.apply(entry.provider, args); }).then(value => {
      assertCurrent(call);
      let originals, input = value;
      if (entry.type === 'completion' && value !== undefined && value !== null) {
        const items = list(Array.isArray(value) ? value : value.items, 300), count = items.length;
        if (!Number.isSafeInteger(count) || count < 0 || count > 300) throw new Error('Invalid completion item count');
        originals = [];
        for (let index = 0; index < count; index++) originals.push(items[index]);
        input = { items: originals, isIncomplete: value.isIncomplete };
      }
      let result;
      if (entry.type === 'codeaction') { const converted = actions.normalize(value,call,args[2]); result = converted.result; originals = converted.originals; }
      else result = normalize(entry.type, input, document);
      wireBudget(result);
      assertCurrent(call);
      if (entry.type === 'codeaction') actions.retain(call,originals,result,callId);
      if (entry.type === 'completion' && entry.resolves && result) retainCompletions(entry, call, originals, result, callId);
      if (entry.type === 'signature') signatures.retain(call, value, result, callId);
      wireBudget(result);
      assertCurrent(call);
      return result;
    }).catch(error => {
      actions.retire(callId,entry.owner);
      for (const [id, cached] of completions) if (cached.request === callId) completions.delete(id);
      if (entry.type === 'signature') signatures.retire({session:options.session,owner:entry.owner,request:callId});
      throw error;
    }).finally(() => { calls.delete(callId); source.dispose(); clearTimeout(timer); if (entry.type === 'signature') signatures.released(params, call); });
    if (entry.type === 'signature') call.signatureWork = true;
    // A timed-out callback keeps its slot until it settles, bounding ignored cancellation.
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => { reject(new Error('Language provider deadline exceeded')); source.cancel(); }, options.timeoutMs || 5000); });
    let cancellationListener;
    const canceled = new Promise((_, reject) => { cancellationListener = source.token.onCancellationRequested(() => reject(new Error('Language provider invocation canceled'))); });
    return Promise.race([work, timeout, canceled]).finally(() => { clearTimeout(timer); cancellationListener.dispose(); });
  }
  async function resolveCompletion(params) {
    purgeCompletions();
    const cached = completions.get(params.handle), entry = entries.get(params.provider);
    if (params.session !== options.session || !cached || cached.entry !== entry || entry.owner !== params.owner || cached.request !== params.origin || cached.version !== params.version || options.document(params.document) !== cached.document) throw new Error('Stale completion resolve handle, owner, or document');
    if (calls.size >= MAX_PENDING) throw new Error('Language provider invocation limit reached');
    if (!Number.isSafeInteger(params.request) || params.request < 1 || calls.has(params.request)) throw new Error('Invalid or duplicate provider request ID');
    const source = new CancellationTokenSource(), call = { source, entry, document: cached.document, version: cached.version, epoch: registryEpoch, completionOrigin: cached.request };
    calls.set(params.request, call);
    let timer;
    const work = Promise.resolve().then(() => entry.provider.resolveCompletionItem(cached.original, source.token)).then(value => {
      assertCurrent(call);
      if (completions.get(params.handle) !== cached) throw new Error('Completion resolve became stale or canceled');
      const result = normalize('completion', [value === undefined || value === null ? cached.original : value], cached.document).items[0];
      const original = JSON.parse(cached.snapshot);
      for (const key of new Set([...Object.keys(original), ...Object.keys(result)])) {
        if (!['detail', 'documentation', 'additionalTextEdits'].includes(key) && JSON.stringify(original[key]) !== JSON.stringify(result[key])) throw new Error(`Completion resolve changed immutable field ${key}`);
      }
      result._vscliCompletionHandle = params.handle;
      wireBudget(result);
      assertCurrent(call);
      if (completions.get(params.handle) !== cached) throw new Error('Completion resolve became stale or canceled');
      return result;
    }).finally(() => { calls.delete(params.request); source.dispose(); clearTimeout(timer); });
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => { reject(new Error('Language provider deadline exceeded')); source.cancel(); }, options.timeoutMs || 5000); });
    let listener;
    const canceled = new Promise((_, reject) => { listener = source.token.onCancellationRequested(() => reject(new Error('Language provider invocation canceled'))); });
    return Promise.race([work, timeout, canceled]).finally(() => { clearTimeout(timer); listener.dispose(); });
  }
  async function resolveAction(params) {
    const entry = entries.get(params.provider);
    if (params.session !== options.session || !entry || entry.owner !== params.owner || entry.type !== 'codeaction' || !entry.resolves) throw new Error('Stale code action resolve owner/session');
    const cached = actions.lookup(params,entry);
    if (calls.size >= MAX_PENDING) throw new Error('Language provider invocation limit reached');
    if (!Number.isSafeInteger(params.request) || params.request < 1 || calls.has(params.request)) throw new Error('Invalid or duplicate provider request ID');
    const source = new CancellationTokenSource(), call = {source,entry,document:cached.document,version:cached.version,epoch:cached.epoch,workspace:cached.workspace,actionOrigin:cached.origin};
    assertCurrent(call); calls.set(params.request,call); let timer;
    const work = Promise.resolve().then(() => { assertCurrent(call); return entry.provider.resolveCodeAction(cached.original,source.token); }).then(value => {
      assertCurrent(call); if (!actions.has(params.handle,cached)) throw new Error('Code action resolve handle retired');
      const result = actions.resolved(value,cached,call,params.handle); wireBudget(result); assertCurrent(call);
      if (!actions.has(params.handle,cached)) throw new Error('Code action resolve handle retired'); return result;
    }).finally(() => {calls.delete(params.request);source.dispose();clearTimeout(timer);});
    const timeout = new Promise((_,reject) => {timer=setTimeout(() => {reject(new Error('Language provider deadline exceeded'));source.cancel();},options.timeoutMs||5000);});
    let listener; const canceled = new Promise((_,reject) => {listener=source.token.onCancellationRequested(() => reject(new Error('Language provider invocation canceled')));});
    return Promise.race([work,timeout,canceled]).finally(() => {clearTimeout(timer);listener.dispose();});
  }
  function cancel(params) {
    if (params.session !== options.session) return false;
    signatures.retire(params);
    if (params.releaseSignatureHelp === true) return true;
    actions.retire(params.request,params.owner);
    for (const [id, cached] of completions) if (cached.entry.owner === params.owner && cached.request === params.request) completions.delete(id);
    for (const pending of calls.values()) if (pending.entry.owner === params.owner && (pending.completionOrigin === params.request || pending.actionOrigin === params.request)) pending.source.cancel();
    const call = calls.get(params.request);
    if (!call || call.entry.owner !== params.owner) return false;
    call.source.cancel(); return true;
  }
  function documentChanged() { purgeCompletions(); actions.purge(); signatures.purge(); for (const { source, document, version } of calls.values()) if (document.isClosed || document.version !== version) source.cancel();
    for (const call of calls.values()) if (call.workspace) { try { actions.assertWorkspace(call.workspace, options); } catch { call.source.cancel(); } } }

  function disposeOwner(owner) { for (const [id, entry] of entries) if (entry.owner === owner) { entries.delete(id); for (const call of calls.values()) if (call.entry === entry) call.source.cancel(); } publish(); }
  return { forOwner, provide, resolveCompletion, resolveAction, retainedActionCount:actions.count, retainedSignatureCount:signatures.count, signaturePending:signatures.pending, cancel, snapshot, documentChanged, disposeOwner, pendingCount: () => calls.size, retainedCompletionCount: () => { purgeCompletions(); return completions.size; } };
}
module.exports = { createProviders, score, selector, normalize };
