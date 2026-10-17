'use strict';
const { Position, Uri, Disposable } = require('./api-types.cjs');
const coordinates = require('./coordinates.cjs');
const { snapshotTextEdit } = require('./edit-values.cjs');
const { createActions, kindValue } = require('./code-actions.cjs');
const signatureTypes = require('./signatures.cjs');
const symbolTypes = require('./symbols.cjs');
const {selector,score}=require('./language-selector.cjs');
const {normalizeNavigation}=require('./navigation-providers.cjs');
const {createProviderOrdering}=require('./provider-order.cjs');
const workspaceSymbolTypes=require('./workspace-symbols.cjs');
const renameTypes=require('./rename-providers.cjs');
const workspaceEditValues=require('./workspace-edit-values.cjs');
const NAVIGATION_TYPES=['definition','references','implementation','typeDefinition','declaration'];
const { CancellationTokenSource, SnippetString } = require('./provider-types.cjs');
const MAX_PROVIDERS = 128, MAX_PENDING = 8, MAX_RESULT_BYTES = 1024 * 1024;
const METHODS = Object.freeze({
  codeaction: ['registerCodeActionsProvider', 'provideCodeActions'],
  completion: ['registerCompletionItemProvider', 'provideCompletionItems'],
  hover: ['registerHoverProvider', 'provideHover'],
  definition: ['registerDefinitionProvider', 'provideDefinition'],
  references: ['registerReferenceProvider', 'provideReferences'],
  implementation: ['registerImplementationProvider', 'provideImplementation'],
  typeDefinition: ['registerTypeDefinitionProvider', 'provideTypeDefinition'],
  declaration: ['registerDeclarationProvider', 'provideDeclaration'],
  formatting: ['registerDocumentFormattingEditProvider', 'provideDocumentFormattingEdits'],
  symbols: ['registerDocumentSymbolProvider', 'provideDocumentSymbols'],
  signature: ['registerSignatureHelpProvider', 'provideSignatureHelp'],
  rename: ['registerRenameProvider', 'provideRenameEdits'],
});
function text(value, max = 4096) {
  if (typeof value !== 'string' || value.length > max || Buffer.byteLength(value) > max) throw new Error('Provider string exceeds its budget or has invalid type');
  return value;
}
function position(value, document) {
  const admitted = coordinates.position(value, 'Provider');
  const result = new Position(admitted.line, admitted.character);
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
function edit(value, document) {
  const captured = snapshotTextEdit(value);
  return { range: range(captured.range, document), newText: text(captured.newText, 4 * 1024 * 1024) };
}
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
        const suppliedEdit = item.textEdit;
        const capturedEdit = suppliedEdit === undefined ? undefined : edit(suppliedEdit, document);
        const insertText = capturedEdit === undefined ? item.insertText : undefined;
        if (capturedEdit === undefined && insertText !== undefined && typeof insertText !== 'string' && !(insertText instanceof SnippetString)) throw new Error('Completion insertText requires a string or SnippetString');
        const insertion = capturedEdit !== undefined ? capturedEdit.newText : insertText === undefined ? result.label : typeof insertText === 'string' ? insertText : insertText.value;
        result.insertText = text(insertion, 65536);
        if (capturedEdit === undefined && insertText && typeof insertText === 'object') result.insertTextFormat = 2;
        if (capturedEdit !== undefined) result.textEdit = capturedEdit;
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
    case 'definition': case 'references': case 'implementation': case 'typeDefinition': case 'declaration': return normalizeNavigation(type,value);
    case 'formatting': return list(value, 4096).map(value => edit(value, document));
    case 'symbols': return symbolTypes.normalize(value,document,{text,range,kind});
    case 'signature': return signatureTypes.normalize(value);
    default: throw new Error('Unknown provider type');
  }
}
function wireBudget(result, limit=MAX_RESULT_BYTES) {
  const pending = [result]; let bytes = 0, nodes = 0;
  while (pending.length) {
    const value = pending.pop();
    if (++nodes > 100000) throw new Error('Language provider result exceeds its node budget');
    if (typeof value === 'string') bytes += Buffer.byteLength(value);
    else if (value && typeof value === 'object') {
      const entries = Object.entries(value);
      for (const [key, child] of entries) { bytes += Buffer.byteLength(key) + 4; pending.push(child); }
    } else bytes += 16;
    if (bytes > limit) throw new Error('Language provider result exceeds 1 MiB');
  }
  if (Buffer.byteLength(JSON.stringify(result)) > limit) throw new Error('Language provider result exceeds 1 MiB');
}
function createProviders(options) {
  const languageEpoch = document => options.languageEpoch?.(document) ?? 0;
  function documentSource(document) {
    if(options.documentSource)return options.documentSource(document);
    return {model:document,document:document._snapshot?.id,version:document.version,languageEpoch:languageEpoch(document),languageId:document.languageId,uri:document.uri.toString(),isClosed:document.isClosed};
  }
  function providerPosition(value,document) {
    const source=documentSource(document),text=source?.canonicalText??source?.text;
    if(typeof text!=='string')return position(value,document);
    const admitted=coordinates.position(value,'Provider');let offset=0;
    for(let line=0;line<admitted.line;line++){const end=text.indexOf('\n',offset);if(end<0)throw new RangeError('Provider position is outside the document');offset=end+1;}
    const lf=text.indexOf('\n',offset),end=lf<0?text.length:lf>offset&&text[lf-1]==='\r'?lf-1:lf;
    const at=offset+admitted.character;
    if(at>end||at>offset&&at<end&&/[\uD800-\uDBFF]/.test(text[at-1])&&/[\uDC00-\uDFFF]/.test(text[at]))throw new RangeError('Provider position is outside the document or splits UTF-16');
    return new Position(admitted.line,admitted.character);
  }
  function released(params, call) {
    if(!params||typeof params!=='object'||params.session!==options.session||!Number.isSafeInteger(params.request)||params.request<1||!Number.isSafeInteger(params.provider)||params.provider<1||typeof params.owner!=='string'||!params.owner||params.owner.length>256)return false;
    // A refused duplicate never releases a still-running original callback.
    if(calls.has(params.request)&&calls.get(params.request)!==call)return false;
    options.notify('languageProviderReleased',{session:options.session,owner:params.owner,provider:params.provider,request:params.request});return true;
  }
  function releaseRefusal(params) {
    if(calls.has(params.request))return false;
    const sent=released(params);
    if(params.signatureRequest===true)signatures.released(params);
    if(params.symbolRequest===true)symbols.released(params);
    return sent;
  }
  const entries = new Map(), calls = new Map(), completions = new Map();
  const navigationOrdering=new Map([...NAVIGATION_TYPES,'rename'].map(type=>[type,createProviderOrdering()]));
  const workspaceIdentity=options.workspaceIdentity??(()=>null),unowned=options.unowned??(callback=>callback());
  const workspaceSymbols=workspaceSymbolTypes.createWorkspaceSymbols({registered:entry=>entries.get(entry.id)===entry,workspaceIdentity});
  const workspaceWaiting=[];let workspaceQueries=0;
  const now=performance.now.bind(performance);
  function deadlineTimer(deadline, expire) {
    let timer;
    function tick() {
      const remaining=deadline-now();
      // Node timers may run before a fractional monotonic deadline. Re-arm
      // against the ORIGINAL instant; never renew a query/callback's lease.
      if(remaining>0){timer=setTimeout(tick,Math.ceil(remaining));return;}
      expire();
    }
    tick();
    return ()=>clearTimeout(timer);
  }
  function pumpWorkspace(){
    while(calls.size<MAX_PENDING&&workspaceWaiting.length){
      const job=workspaceWaiting.shift();
      try{job.check();provideWorkspaceSymbols(job.params).then(job.resolve,job.reject);}catch(error){job.reject(error);}
    }
  }
  function capacityChanged(){queueMicrotask(()=>unowned(pumpWorkspace));}
  let nextId = 0, nextHandle = 0, registryEpoch = 0;
  const actions = createActions({ ...options, text, range, position, current: item => {
    if (entries.get(item.entry.id) !== item.entry || item.epoch !== registryEpoch || item.document.isClosed || item.document.version !== item.version || languageEpoch(item.document) !== item.languageEpoch) return false;
    try { actions.assertWorkspace(item.workspace, options); return true; } catch { return false; }
  } });
  const signatures = signatureTypes.createSignatures({ ...options, position, assertCurrent,
    registered: (entry, epoch) => entries.get(entry.id) === entry && registryEpoch === epoch });
  const symbols = symbolTypes.createSymbols(options);
  const resultCallback = (entry, callback) => options.bindProviderResult?.(entry.provider, callback) ?? callback;
  const invokeResult = (entry, callback) => options.invokeProviderResult ? options.invokeProviderResult(entry.provider, callback) : callback();
  function snapshot() { return [...entries.values()].map(({ id, owner, type, selector, triggers, retriggers, resolves, actionKinds }) => ({ id, owner, type, selector, triggers, resolves, ...(type === 'signature' ? {retriggers} : {}), ...(actionKinds ? {actionKinds} : {}) })); }
  function publish() { registryEpoch++; completions.clear(); actions.clear(); signatures.clear(); for (const call of calls.values()) if (call.completionOrigin !== undefined || call.actionOrigin !== undefined || ['signature','symbols','rename'].includes(call.entry.type)) call.source.cancel(); options.notify('languageProviders', { session: options.session, providers: snapshot() }); }
  function forOwner(owner) {
    const methods=Object.fromEntries(Object.entries(METHODS).map(([type, [registration, method]]) => [registration, (documentSelector, provider, ...triggers) => {
      const originalCaller=options.captureCaller?.(owner);originalCaller?.();
      if (entries.size >= MAX_PROVIDERS) throw new Error('Extension language provider limit reached');
      provider = options.bindProvider?.(owner, provider) ?? provider;
      if (!provider || typeof provider[method] !== 'function') throw new TypeError(`Provider requires ${method}`);
      let actionKinds, retriggers = [];
      if (type === 'codeaction') {
        if (triggers.length > 1) throw new Error('Invalid code action metadata');
        const metadata = triggers[0]; triggers = [];
        if (metadata !== undefined) {
          if (!metadata || typeof metadata !== 'object' || Object.keys(metadata).some(key => !['providedCodeActionKinds','documentation'].includes(key))) throw new Error('Unsupported code action metadata');
          if (metadata.documentation !== undefined) throw new Error('Code action provider documentation is unsupported');
          const providedKinds = metadata.providedCodeActionKinds;
          if (providedKinds !== undefined) actionKinds = require('./code-actions.cjs').array(providedKinds,32).map(kind => {
            // A public CodeActionKind is mutable and deliberately unvalidated.
            // Capture its value once; only bounded primitive strings reach Rust.
            return kindValue(kind);
          });
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
      const entry = { id, owner, type, selector: selector(documentSelector), triggers, retriggers, provider, method, actionKinds, resolves: type === 'completion' && typeof provider.resolveCompletionItem === 'function' || type === 'codeaction' && typeof provider.resolveCodeAction === 'function' || type === 'rename' && typeof provider.prepareRename === 'function' };
      originalCaller?.();
      navigationOrdering.get(type)?.register(entry,options.isBuiltinOwner?.(owner)===true);
      entries.set(id, entry);
      const disposable = new Disposable(() => { navigationOrdering.get(type)?.remove(entry); entries.delete(id); for (const call of calls.values()) if (call.entry === entry) call.source.cancel(); publish(); });
      try { const result = options.track(owner, disposable); publish(); return result; }
      catch (error) { disposable.dispose(); throw error; }
    }]));
    methods.registerWorkspaceSymbolProvider=provider=>{
      const guard=options.captureCaller?.(owner);guard?.();
      if(entries.size>=MAX_PROVIDERS||nextId>=Number.MAX_SAFE_INTEGER)throw new Error('Extension language provider limit or identity exhausted');
      provider=options.bindProvider?.(owner,provider)??provider;
      if(!provider||typeof provider.provideWorkspaceSymbols!=='function')throw new TypeError('Provider requires provideWorkspaceSymbols');
      const entry={id:++nextId,owner,type:'workspaceSymbols',selector:[],triggers:[],retriggers:[],provider,method:'provideWorkspaceSymbols',resolves:typeof provider.resolveWorkspaceSymbol==='function'};
      guard?.();entries.set(entry.id,entry);
      const disposable=new Disposable(()=>{entries.delete(entry.id);workspaceSymbols.clearEntry(entry);for(const call of calls.values())if(call.entry===entry)call.source.cancel();publish();});
      try{const result=options.track(owner,disposable);publish();return result;}catch(error){disposable.dispose();throw error;}
    };
    methods.match=(documentSelector,document)=>{
      const guard=options.captureCaller?.(owner);guard?.();const source=documentSource(document);
      if(!source||source.isClosed)throw new Error('Language selector document is not mirrored');
      const normalized=selector(documentSelector);guard?.();const live=documentSource(document);
      if(!live||live.isClosed||live.version!==source.version||live.languageEpoch!==source.languageEpoch||live.uri!==source.uri)throw new Error('Language selector document changed');
      return score(normalized,source);
    };
    return methods;
  }
  function assertCurrent(call) {
    const live=documentSource(call.document);
    if (call.source.token.isCancellationRequested || entries.get(call.entry.id) !== call.entry || (['completion','codeaction','signature','symbols','rename'].includes(call.entry.type) && registryEpoch !== call.epoch) || !live || live.isClosed || live.version !== call.version || live.languageEpoch !== call.languageEpoch) throw new Error('Language provider result became stale or canceled');
    if(call.documentId!==undefined&&(options.document(call.documentId)!==call.document||live.uri!==call.uri))throw new Error('Language provider document identity changed');
    if (call.workspace) actions.assertWorkspace(call.workspace, options);
  }
  function purgeCompletions() {
    for (const [id, cached] of completions) if (Date.now() - cached.started >= 6000 || cached.document.isClosed || cached.document.version !== cached.version || languageEpoch(cached.document) !== cached.languageEpoch || entries.get(cached.entry.id) !== cached.entry) completions.delete(id);
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
      return [handle, { entry, document: call.document, version: call.version, languageEpoch: call.languageEpoch, request,
        original: originals[index], snapshot: JSON.stringify(item), bytes: Buffer.byteLength(JSON.stringify(item)), started: Date.now() }];
    });
    for (let index = 0; index < staged.length; index++) { completions.set(...staged[index]); result.items[index]._vscliCompletionHandle = staged[index][0]; }
  }
  async function provide(params) {
    const signature = params.signatureRequest === true || entries.get(params.provider)?.type === 'signature';
    const symbol = params.symbolRequest === true || entries.get(params.provider)?.type === 'symbols';
    let accepted;
    try { return await performProvide(params, call => { accepted = call; }); }
    finally { if (!accepted?.genericWork) released(params); if (signature && !accepted?.signatureWork) signatures.released(params, accepted); if (symbol && !accepted?.symbolWork) symbols.released(params, accepted); }
  }
  async function prepareRename(params){return renameInvocation(params,true);}
  async function provideRenameEdits(params){return renameInvocation(params,false);}
  async function renameInvocation(params,prepare){
    let accepted;
    try{
      const keys=['session','owner','provider','request','document','version','position',...(prepare?[]:['newName'])];
      if(!params||typeof params!=='object'||Array.isArray(params)||Object.keys(params).some(key=>!keys.includes(key))||entries.get(params.provider)?.type!=='rename')throw new Error('Invalid rename provider request');
      if(!prepare)renameTypes.boundedText(params.newName);
      if(prepare&&!entries.get(params.provider)?.resolves)throw new Error('Rename provider has no preparation callback');
      return await performProvide(params,call=>accepted=call,prepare);
    }finally{if(!accepted?.genericWork)released(params);}
  }
  async function performProvide(params, accepted, renamePrepare) {
    const entry = entries.get(params.provider);
    if (params.session !== options.session || !entry || entry.owner !== params.owner) throw new Error('Stale or invalid provider owner/session');
    const document = options.document(params.document),authority=document&&documentSource(document);
    if(entry.type==='rename'&&renamePrepare===undefined)throw new Error('Rename requires its dedicated provider operation');
    if (!document || !authority || authority.isClosed || authority.version !== params.version || (entry.type==='rename'?!navigationOrdering.get('rename').ordered(authority).includes(entry):!score(entry.selector, authority))) throw new Error('Provider document changed or does not match its selector');
    if (calls.size >= MAX_PENDING) throw new Error('Language provider invocation limit reached');
    const callId = params.request === undefined ? Symbol() : params.request;
    if (typeof callId !== 'symbol' && (!Number.isSafeInteger(callId) || callId < 1 || calls.has(callId))) throw new Error('Invalid or duplicate provider request ID');
    const source = new CancellationTokenSource(), call = { source, entry, document, documentId: params.document, uri: authority.uri, version: authority.version, languageEpoch: authority.languageEpoch, epoch: registryEpoch };
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
          args = [document, providerPosition(params.position, document), source.token, { triggerKind: kind - 1, ...(kind === 2 ? { triggerCharacter: context.triggerCharacter } : {}) }]; break;
        }
        case 'signature': signatures.reserve(call, callId); args = signatures.args(params, call); break;
        case 'hover': case 'definition': case 'implementation': case 'typeDefinition': case 'declaration': args = [document, providerPosition(params.position, document), source.token]; break;
        case 'references': args = [document, providerPosition(params.position, document), { includeDeclaration: !!params.includeDeclaration }, source.token]; break;
        case 'formatting':
          if (!Number.isInteger(params.options?.tabSize) || params.options.tabSize < 1 || params.options.tabSize > 32 || typeof params.options.insertSpaces !== 'boolean') throw new Error('Invalid native formatting options');
          args = [document, { tabSize: params.options.tabSize, insertSpaces: params.options.insertSpaces }, source.token]; break;
        case 'symbols': symbols.reserve(call,callId); args = [document, source.token]; break;
        case 'rename': args=renamePrepare?[document,providerPosition(params.position,document),source.token]:[document,providerPosition(params.position,document),renameTypes.boundedText(params.newName),source.token]; break;
      }
    } catch (error) { calls.delete(callId); capacityChanged(); source.dispose(); throw error; }
    let timer;
    const work = Promise.resolve().then(() => { assertCurrent(call); const callback = entry.provider[entry.type==='rename'?(renamePrepare?'prepareRename':'provideRenameEdits'):entry.method]; assertCurrent(call); return invokeResult(entry, () => callback.apply(entry.provider, args)); }).then(resultCallback(entry, value => {
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
      else if(entry.type==='rename')result=renamePrepare?renameTypes.normalizePrepare(value,authority,args[1]):value?workspaceEditValues.snapshotWorkspaceEdit(value,()=>assertCurrent(call)):null;
      else result = normalize(entry.type, input, document);
      wireBudget(result,entry.type==='rename'?2*MAX_RESULT_BYTES:MAX_RESULT_BYTES);
      assertCurrent(call);
      if (entry.type === 'codeaction') actions.retain(call,originals,result,callId);
      if (entry.type === 'completion' && entry.resolves && result) retainCompletions(entry, call, originals, result, callId);
      if (entry.type === 'signature') signatures.retain(call, value, result, callId);
      wireBudget(result,entry.type==='rename'?2*MAX_RESULT_BYTES:MAX_RESULT_BYTES);
      assertCurrent(call);
      return result;
    })).catch(async error => {
      if(entry.type==='rename'){assertCurrent(call);const reason=await invokeResult(entry,()=>renameTypes.rejection(error));assertCurrent(call);if(reason)return reason;}
      actions.retire(callId,entry.owner);
      for (const [id, cached] of completions) if (cached.request === callId) completions.delete(id);
      if (entry.type === 'signature') signatures.retire({session:options.session,owner:entry.owner,request:callId});
      throw error;
    }).finally(() => { calls.delete(callId); capacityChanged(); source.dispose(); clearTimeout(timer); if (entry.type === 'signature') signatures.released(params, call); if (entry.type === 'symbols') symbols.released(params,call); released(params,call); });
    call.genericWork=true;
    if (entry.type === 'signature') call.signatureWork = true;
    if (entry.type === 'symbols') call.symbolWork = true;
    // A timed-out callback keeps its slot until it settles, bounding ignored cancellation.
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => { reject(new Error('Language provider deadline exceeded')); source.cancel(); }, options.timeoutMs || 5000); });
    let cancellationListener;
    const canceled = new Promise((_, reject) => { cancellationListener = source.token.onCancellationRequested(() => reject(new Error('Language provider invocation canceled'))); });
    return Promise.race([work, timeout, canceled]).finally(() => { clearTimeout(timer); cancellationListener.dispose(); });
  }
  async function resolveCompletion(params) {
    let accepted;
    try{return await performResolveCompletion(params,call=>accepted=call);}finally{if(!accepted?.genericWork)released(params);}
  }
  async function performResolveCompletion(params,accepted) {
    purgeCompletions();
    const cached = completions.get(params.handle), entry = entries.get(params.provider);
    if (params.session !== options.session || !cached || cached.entry !== entry || entry.owner !== params.owner || cached.request !== params.origin || cached.version !== params.version || options.document(params.document) !== cached.document) throw new Error('Stale completion resolve handle, owner, or document');
    if (calls.size >= MAX_PENDING) throw new Error('Language provider invocation limit reached');
    if (!Number.isSafeInteger(params.request) || params.request < 1 || calls.has(params.request)) throw new Error('Invalid or duplicate provider request ID');
    const source = new CancellationTokenSource(), call = { source, entry, document: cached.document, version: cached.version, languageEpoch: cached.languageEpoch, epoch: registryEpoch, completionOrigin: cached.request };
    calls.set(params.request, call);accepted(call);
    let timer;
    const work = Promise.resolve().then(() => invokeResult(entry, () => entry.provider.resolveCompletionItem(cached.original, source.token))).then(resultCallback(entry, value => {
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
    })).finally(() => { calls.delete(params.request); capacityChanged(); source.dispose(); clearTimeout(timer); released(params,call); });
    call.genericWork=true;
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => { reject(new Error('Language provider deadline exceeded')); source.cancel(); }, options.timeoutMs || 5000); });
    let listener;
    const canceled = new Promise((_, reject) => { listener = source.token.onCancellationRequested(() => reject(new Error('Language provider invocation canceled'))); });
    return Promise.race([work, timeout, canceled]).finally(() => { clearTimeout(timer); listener.dispose(); });
  }
  async function resolveAction(params) {
    let accepted;
    try{return await performResolveAction(params,call=>accepted=call);}finally{if(!accepted?.genericWork)released(params);}
  }
  async function performResolveAction(params,accepted) {
    const entry = entries.get(params.provider);
    if (params.session !== options.session || !entry || entry.owner !== params.owner || entry.type !== 'codeaction' || !entry.resolves) throw new Error('Stale code action resolve owner/session');
    const cached = actions.lookup(params,entry);
    if (calls.size >= MAX_PENDING) throw new Error('Language provider invocation limit reached');
    if (!Number.isSafeInteger(params.request) || params.request < 1 || calls.has(params.request)) throw new Error('Invalid or duplicate provider request ID');
    const source = new CancellationTokenSource(), call = {source,entry,document:cached.document,version:cached.version,languageEpoch:cached.languageEpoch,epoch:cached.epoch,workspace:cached.workspace,actionOrigin:cached.origin};
    assertCurrent(call); calls.set(params.request,call);accepted(call); let timer;
    const work = Promise.resolve().then(() => { assertCurrent(call); return invokeResult(entry, () => entry.provider.resolveCodeAction(cached.original,source.token)); }).then(resultCallback(entry, value => {
      assertCurrent(call); if (!actions.has(params.handle,cached)) throw new Error('Code action resolve handle retired');
      const result = actions.resolved(value,cached,call,params.handle); wireBudget(result); assertCurrent(call);
      if (!actions.has(params.handle,cached)) throw new Error('Code action resolve handle retired'); return result;
    })).finally(() => {calls.delete(params.request);capacityChanged();source.dispose();clearTimeout(timer);released(params,call);});
    call.genericWork=true;
    const timeout = new Promise((_,reject) => {timer=setTimeout(() => {reject(new Error('Language provider deadline exceeded'));source.cancel();},options.timeoutMs||5000);});
    let listener; const canceled = new Promise((_,reject) => {listener=source.token.onCancellationRequested(() => reject(new Error('Language provider invocation canceled')));});
    return Promise.race([work,timeout,canceled]).finally(() => {clearTimeout(timer);listener.dispose();});
  }
  function cancel(params) {
    if (params.session !== options.session) return false;
    signatures.retire(params);
    if (params.releaseSignatureHelp === true) return true;
    workspaceSymbols.retire(params.request,params.owner);
    actions.retire(params.request,params.owner);
    for (const [id, cached] of completions) if (cached.entry.owner === params.owner && cached.request === params.request) completions.delete(id);
    for (const pending of calls.values()) if (pending.entry.owner === params.owner && (pending.completionOrigin === params.request || pending.actionOrigin === params.request || pending.workspaceOrigin === params.request)) pending.source.cancel();
    const call = calls.get(params.request);
    if (!call || call.entry.owner !== params.owner) return false;
    call.source.cancel(); return true;
  }
  function documentChanged() { workspaceSymbols.purge();purgeCompletions(); actions.purge(); signatures.purge(); for (const call of calls.values()) {if(call.entry.type==='workspaceSymbols'){if(workspaceIdentity()!==call.workspace)call.source.cancel();continue;}const live=documentSource(call.document);if (!live||live.isClosed||live.version!==call.version||live.languageEpoch!==call.languageEpoch||call.documentId!==undefined&&options.document(call.documentId)!==call.document) call.source.cancel();}
    for (const call of calls.values()) if (call.entry.type!=='workspaceSymbols'&&call.workspace) { try { actions.assertWorkspace(call.workspace, options); } catch { call.source.cancel(); } } }

  function assertWorkspaceCall(call){
    if(call.source.token.isCancellationRequested||entries.get(call.entry.id)!==call.entry||workspaceIdentity()!==call.workspace||call.cached&&!workspaceSymbols.current(call.cached))throw new Error('Workspace symbol result became stale or canceled');
    if(now()>=call.deadline)throw new Error('Workspace symbol provider deadline exceeded');
  }
  function workspaceRequest(params,resolve){
    const keys=resolve?['session','owner','provider','request','origin','handle']:['session','owner','provider','request','query'];
    if(!params||typeof params!=='object'||Object.keys(params).some(key=>!keys.includes(key))||params.session!==options.session)throw new Error('Invalid workspace symbol request');
    const entry=entries.get(params.provider);
    if(!entry||entry.owner!==params.owner||entry.type!=='workspaceSymbols')throw new Error('Stale workspace symbol provider owner/session');
    if(calls.size>=MAX_PENDING)throw new Error('Language provider invocation limit reached');
    const id=params.request??Symbol();
    if(typeof id!=='symbol'&&(!Number.isSafeInteger(id)||id<1||calls.has(id)))throw new Error('Invalid or duplicate provider request ID');
    if(resolve&&(!Number.isSafeInteger(params.origin)||params.origin<1||!entry.resolves))throw new Error('Invalid workspace symbol resolve origin');
    return {entry,id};
  }
  async function provideWorkspaceSymbols(params){
    let accepted;try{return await workspaceWork(params,false,call=>accepted=call);}finally{if(!accepted?.genericWork)released(params);}
  }
  async function resolveWorkspaceSymbol(params){
    let accepted;try{return await workspaceWork(params,true,call=>accepted=call);}finally{if(!accepted?.genericWork)released(params);}
  }
  async function workspaceWork(params,resolve,accept){
    const {entry,id}=workspaceRequest(params,resolve),cached=resolve?workspaceSymbols.lookup(params,entry):undefined;
    const query=resolve?undefined:text(params.query,4096);
    const deadline=now()+(options.timeoutMs||5000);
    const source=new CancellationTokenSource(),call={entry,source,workspace:cached?.workspace??workspaceIdentity(),cached,workspaceOrigin:cached?.origin,deadline};
    calls.set(id,call);accept(call);let cancelTimer,listener;
    const work=Promise.resolve().then(()=>{
      assertWorkspaceCall(call);const callback=entry.provider[resolve?'resolveWorkspaceSymbol':'provideWorkspaceSymbols'];assertWorkspaceCall(call);
      return invokeResult(entry,()=>callback.apply(entry.provider,resolve?[cached.original,source.token]:[query,source.token]));
    }).then(resultCallback(entry,value=>{
      assertWorkspaceCall(call);
      const converted=resolve?workspaceSymbols.resolved(value,cached):workspaceSymbols.normalize(value,message=>options.diagnostic?.(entry.owner,new Error(message)));
      assertWorkspaceCall(call);
      const result=resolve?converted:workspaceSymbols.retain(call,converted,id);
      wireBudget(result);assertWorkspaceCall(call);return result;
    })).finally(()=>{calls.delete(id);capacityChanged();source.dispose();cancelTimer?.();released(params,call);});
    call.genericWork=true;
    const timeout=new Promise((_,reject)=>{cancelTimer=deadlineTimer(deadline,()=>{reject(new Error('Workspace symbol provider deadline exceeded'));source.cancel();});});
    const canceled=new Promise((_,reject)=>{listener=source.token.onCancellationRequested(()=>reject(new Error('Workspace symbol invocation canceled')));});
    return Promise.race([work,timeout,canceled]).finally(()=>{cancelTimer();listener.dispose();});
  }
  async function queryWorkspaceSymbols(owner,query){
    const guard=options.captureCaller?.(owner),workspace=workspaceIdentity();guard?.();
    query=text(query,4096);
    if(workspaceQueries>=8)throw new Error('Workspace symbol query invocation limit reached');
    const selected=[...entries.values()].filter(entry=>entry.type==='workspaceSymbols');
    workspaceQueries++;const deadline=now()+(options.queryTimeoutMs||options.timeoutMs||5000),cohort={},ids=[],rows=[];let bytes=2,cancelTimer,retired=false;
    const check=()=>{guard?.();if(retired||workspaceIdentity()!==workspace)throw new Error('Workspace symbol query source or caller retired');if(now()>=deadline)throw new Error('Workspace symbol query deadline exceeded');};
    try{
      const promises=selected.map(entry=>new Promise((resolve,reject)=>{
        const request=Symbol();ids.push({request,owner:entry.owner});
        if(workspaceWaiting.length>=1024){reject(new Error('Workspace symbol query queue exceeds 1024 entries'));return;}
        workspaceWaiting.push({cohort,check,params:{session:options.session,owner:entry.owner,provider:entry.id,request,query},resolve,reject});
      }).then(value=>{
        check();for(const symbol of value.symbols){if(rows.length>=512)throw new RangeError('Workspace symbol query exceeds 512 symbols');bytes+=Buffer.byteLength(JSON.stringify(symbol))+1;if(bytes>MAX_RESULT_BYTES)throw new RangeError('Workspace symbol query exceeds 1 MiB');rows.push({symbol,resolves:entry.resolves});}
      },error=>{check();options.diagnostic?.(entry.owner,error);}));
      unowned(pumpWorkspace);
      const timeout=new Promise((_,reject)=>{cancelTimer=deadlineTimer(deadline,()=>reject(new Error('Workspace symbol query deadline exceeded; callbacks remain occupied')));});
      await Promise.race([Promise.all(promises),timeout]);check();
      const result=workspaceSymbolTypes.sortAndDedup(rows).map(row=>workspaceSymbolTypes.toPublic(row.symbol));check();return result;
    }finally{
      retired=true;cancelTimer?.();workspaceQueries--;
      for(let index=workspaceWaiting.length-1;index>=0;index--)if(workspaceWaiting[index].cohort===cohort){const [job]=workspaceWaiting.splice(index,1);job.reject(new Error('Workspace symbol query retired'));}
      for(const id of ids){const call=calls.get(id.request);if(call&&call.entry.owner===id.owner)call.source.cancel();}
    }
  }
  function retireWorkspaceSymbols(params){
    if(!params||typeof params!=='object'||Array.isArray(params)||Object.keys(params).length!==3||Object.keys(params).some(key=>!['session','owner','origin'].includes(key))||params.session!==options.session||typeof params.owner!=='string'||!params.owner||params.owner.length>256||!Number.isSafeInteger(params.origin)||params.origin<1)return false;
    workspaceSymbols.retire(params.origin,params.owner);
    for(const call of calls.values())if(call.entry.type==='workspaceSymbols'&&call.entry.owner===params.owner&&call.workspaceOrigin===params.origin)call.source.cancel();
    return true;
  }
  function rank(params){
    const rename=params?.type==='rename',keys=['session','type','document','version','languageEpoch',...(rename?['position']:[])];
    if(!params||typeof params!=='object'||Array.isArray(params)||Object.keys(params).length!==keys.length||Object.keys(params).some(key=>!keys.includes(key))||params.session!==options.session||!navigationOrdering.has(params.type)||!Number.isSafeInteger(params.document)||params.document<1||!Number.isSafeInteger(params.version)||params.version<1||!Number.isSafeInteger(params.languageEpoch)||params.languageEpoch<0)throw new Error('Invalid ordered provider ranking request');
    const document=options.document(params.document),source=document&&documentSource(document);
    if(!source||source.isClosed||source.version!==params.version||source.languageEpoch!==params.languageEpoch)throw new Error('Ordered provider ranking source retired');
    const selected=navigationOrdering.get(params.type).ordered(source),fallback=rename?renameTypes.defaultLocation(source,providerPosition(params.position,document)):undefined;
    const live=documentSource(document);
    if(options.document(params.document)!==document||!live||live.isClosed||live.version!==source.version||live.languageEpoch!==source.languageEpoch||live.uri!==source.uri)throw new Error('Ordered provider ranking source changed');
    return {providers:selected.map(entry=>({id:entry.id,owner:entry.owner})),...(rename?{fallback}:{})};
  }
  async function queryRename(source,at,newName){
    source.recheck();const epoch=registryEpoch,selected=navigationOrdering.get('rename').ordered(source),deadline=performance.now()+(options.queryTimeoutMs||options.timeoutMs||5000);
    const recheck=()=>{source.recheck();if(registryEpoch!==epoch)throw new Error('Rename provider cohort retired');if(performance.now()>=deadline)throw new Error('Rename query deadline exceeded; callbacks remain occupied until actual settlement');};
    const value=await renameTypes.queryRename(selected,{...source,recheck},at,newName,(entry,prepare)=>{
      recheck();return renameInvocation({session:options.session,owner:entry.owner,provider:entry.id,request:Symbol(),document:source.document,version:source.version,position:at,...(prepare?{}:{newName})},prepare);
    });recheck();return value;
  }
  async function query(type,source,at) {
    source.recheck();
    if(calls.size>=MAX_PENDING)throw new Error('Language provider invocation limit reached');
    const selected=navigationOrdering.get(type)?.ordered(source);
    if(!selected)throw new TypeError('Invalid navigation provider family');
    const result=[];
    const deadline=performance.now()+(options.queryTimeoutMs||options.timeoutMs||5000);
    const check=()=>{source.recheck();if(performance.now()>=deadline)throw new Error('Navigation query deadline exceeded; callbacks remain occupied until actual settlement');};
    let bytes=2;
    for(const entry of selected){
      check();
      if(calls.size>=MAX_PENDING)throw new Error('Language provider invocation limit reached');
      let values;
      try{values=await provide({session:options.session,owner:entry.owner,provider:entry.id,request:Symbol(),document:source.document,version:source.version,position:at,includeDeclaration:true});}
      catch(error){check();options.diagnostic?.(entry.owner,error);continue;}
      check();
      for(const value of values||[]){
        if(result.length>=512)throw new RangeError('Navigation query aggregate exceeds 512 locations');
        bytes+=Buffer.byteLength(JSON.stringify(value))+1;if(bytes>MAX_RESULT_BYTES)throw new RangeError('Navigation query aggregate exceeds 1 MiB');
        result.push(value);
      }
    }
    source.recheck();return result;
  }
  function disposeOwner(owner) { for (const [id, entry] of entries) if (entry.owner === owner) { navigationOrdering.get(entry.type)?.remove(entry);workspaceSymbols.clearEntry(entry); entries.delete(id); for (const call of calls.values()) if (call.entry === entry) call.source.cancel(); } publish(); }
  return { forOwner, provide, prepareRename,provideRenameEdits,queryRename,resolveCompletion, resolveAction, provideWorkspaceSymbols,resolveWorkspaceSymbol,queryWorkspaceSymbols,retireWorkspaceSymbols,retainedWorkspaceSymbolCount:workspaceSymbols.count,query,rank,releaseRefusal,pending:request=>calls.has(request), retainedActionCount:actions.count, retainedSignatureCount:signatures.count, signaturePending:signatures.pending, symbolsPending:symbols.pending, cancel, snapshot, documentChanged, disposeOwner, pendingCount: () => calls.size, retainedCompletionCount: () => { purgeCompletions(); return completions.size; } };
}
module.exports = { createProviders, score, selector, normalize };
