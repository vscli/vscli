'use strict';
// Source-guided native adapter, Microsoft VS Code stable 1.141.0 (MIT).
// https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocumentContentProviders.ts
// The native bridge coalesces refresh bursts into one latest intent per URI;
// upstream serializes lastEvent promises. Actual callbacks stay occupied until
// settlement, including providers which ignore cancellation.
const { Uri, Disposable } = require('./api-types.cjs');
const { CancellationTokenSource } = require('./cancellation.cjs');
const uriFormat = Uri.prototype.toString, uriFrom = Uri.from;
const MAX_PROVIDERS = 64, MAX_DOCUMENTS = 128, MAX_CALLS = 8;
const MAX_CONTENT = 1024 * 1024, MAX_URI = 4096;
function snapshotUri(value) {
  if (!Uri.isUri(value)) throw new TypeError('Virtual document resource must be a URI');
  const fields = {};
  for (const key of ['scheme','authority','path','query','fragment']) {
    const part = value[key];
    if (typeof part !== 'string' || part.length > MAX_URI || Buffer.byteLength(part) > MAX_URI || part.includes('\0'))
      throw new Error('Invalid or oversized virtual document URI');
    fields[key] = part;
  }
  const uri = Reflect.apply(uriFrom, Uri, [fields,true]);
  const key = Reflect.apply(uriFormat, uri, []);
  if (key.length > MAX_URI || Buffer.byteLength(key) > MAX_URI) throw new Error('Virtual document URI exceeds 4096 bytes');
  return {uri,key};
}
function content(value) {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== 'string') throw new TypeError('Text document content provider must return a string, null or undefined');
  if (value.length > MAX_CONTENT || Buffer.byteLength(value) > MAX_CONTENT) throw new Error('Virtual document content exceeds 1 MiB');
  return value.replace(/\r\n|\r/g,'\n');
}
function createTextContentProviders(options) {
  const entries = new Map(), opening = new Map(), refreshes = new Map();
  let generation = 0, nextHandle = 0, actual = 0, scheduled = false;
  const unowned = options.unowned || (call => call());
  const bindProvider = options.bindProvider || ((_owner,provider) => provider);
  const invokeResult = options.invokeProviderResult || ((_provider,call) => Promise.resolve(call()));
  const bindResult = options.bindProviderResult || ((_provider,call) => call);
  const bindCleanup = options.bindProviderCleanup || ((_provider,call) => call);
  function snapshot() {return {session:options.session,generation,providers:[...entries.values()].map(entry=>({scheme:entry.scheme,owner:entry.owner,handle:entry.handle}))};}
  function publish() {options.notify('textContentProviders',snapshot());}
  function advance() {if(!Number.isSafeInteger(generation+1))throw new Error('Text content provider generation exhausted');generation++;}
  function current(entry) {entry.guard();if(entries.get(entry.handle)!==entry)throw new Error('Text content provider was disposed');}
  function lookup(key) {
    let count=0;
    for(const mirror of options.mirrors()) {
      if(++count>MAX_DOCUMENTS)throw new Error('Virtual document mirror inventory exceeds 128');
      if(mirror.uri===key)return mirror;
    }
  }
  function mirrorCurrent(mirror,key) {
    const now=options.mirror(mirror.document);
    return now && now.uri===key && now.model===mirror.model ? now : undefined;
  }
  function report(entry,error) {
    let message='Virtual document provider retired or refresh failed';
    try {message=bindResult(entry.provider,()=>String(error?.message||error).slice(0,1024))();}catch{}
    try {options.diagnostic?.(entry.owner,new Error(message));}catch{}
  }
  function queueDrain() {
    if(scheduled)return;scheduled=true;
    queueMicrotask(()=>unowned(()=>{scheduled=false;drain();}));
  }
  async function provide(entry,uri,token) {
    current(entry);
    const value=await unowned(()=>invokeResult(entry.provider,()=>{
      const method=entry.provider.provideTextDocumentContent;current(entry);
      return Reflect.apply(method,entry.provider,[uri,token]);
    }));
    const normalized=bindResult(entry.provider,()=>content(value))();
    current(entry);
    return normalized;
  }
  function cancelState(state) {state.pending=false;state.token?.cancel();}
  function documentChanged() {
    for(const [key,state] of refreshes) {
      if(!mirrorCurrent(state.mirror,key)) {cancelState(state);refreshes.delete(key);}
    }
    for(const [key,state] of opening)if(state.mirror && !mirrorCurrent(state.mirror,key))state.token?.cancel();
    queueDrain();
  }
  function changed(entry,value) {
    if(entries.get(entry.handle)!==entry)return;
    try {
      const {uri,key}=bindResult(entry.provider,()=>snapshotUri(value))();current(entry);
      if(uri.scheme!==entry.scheme)return;
      const mirror=lookup(key);if(!mirror)return;
      let state=refreshes.get(key);
      if(!state) {
        if(refreshes.size>=MAX_DOCUMENTS)throw new Error('Virtual document refresh limit exceeded');
        state={entry,mirror,uri,key,sequence:0,pending:false,running:false};refreshes.set(key,state);
      } else {
        // Different registered providers may signal this URI. Preserve the
        // latest event's original provider and document identity explicitly.
        state.entry=entry;state.mirror=mirror;state.uri=uri;
      }
      if(!Number.isSafeInteger(state.sequence+1))throw new Error('Virtual document refresh generation exhausted');
      state.sequence++;state.pending=true;state.token?.cancel();queueDrain();
    } catch(error) {report(entry,error);}
  }
  function drain() {
    for(const state of refreshes.values()) {
      if(actual>=MAX_CALLS)break;
      if(!state.pending||state.running)continue;
      const {entry,key,uri}=state,mirror=mirrorCurrent(state.mirror,key);
      if(!mirror || entries.get(entry.handle)!==entry){cancelState(state);continue;}
      const sequence=state.sequence,source=new CancellationTokenSource();
      state.pending=false;state.running=true;state.runningEntry=entry;state.token=source;actual++;
      unowned(()=>Promise.resolve().then(async()=>{
        const value=await provide(entry,uri,source.token);
        if(source.token.isCancellationRequested||sequence!==state.sequence||refreshes.get(key)!==state||value===undefined)return;
        current(entry);const now=mirrorCurrent(mirror,key);
        if(!now || now.version!==mirror.version)return;
        if(now.text===value)return;
        const result=await options.call(entry.owner,'nativeVirtualDocumentChange',{uri:key,content:value,document:now.document,version:now.version,providerOwner:entry.owner,providerHandle:entry.handle});
        current(entry);
        const published=mirrorCurrent(now,key);
        if(!result || result.document!==now.document || !published || published.text!==value || published.version<=now.version)
          throw new Error('Virtual document change mirror was not published before acknowledgement');
      }).catch(error=>report(entry,error)).finally(()=>{
        source.dispose();actual--;state.running=false;state.runningEntry=undefined;state.token=undefined;
        const now=mirrorCurrent(state.mirror,key);if(now)state.mirror=now;
        queueDrain();
      }));
    }
  }
  async function open(owner,value) {
    const guard=options.captureCaller(owner);guard();
    const {uri,key}=snapshotUri(value);guard();
    const cached=lookup(key);if(cached)return cached.model;
    let shared=opening.get(key);
    if(!shared) {
      if(actual>=MAX_CALLS||opening.size>=MAX_CALLS)throw new Error('Text content provider callback limit exceeded');
      const candidates=[...entries.values()].filter(entry=>entry.scheme===uri.scheme).reverse();
      if(!candidates.length)throw new Error(`No text document content provider for scheme '${uri.scheme}'`);
      shared={token:new CancellationTokenSource(),entry:undefined,mirror:undefined};actual++;opening.set(key,shared);
      shared.promise=unowned(()=>Promise.resolve().then(async()=>{
        for(const entry of candidates) {
          shared.entry=entry;const normalized=await provide(entry,uri,shared.token.token);
          guard();current(entry);
          if(shared.token.token.isCancellationRequested)throw new Error('Virtual document open canceled');
          if(normalized===undefined)continue;
          const result=await options.call(owner,'nativeVirtualDocumentOpen',{uri:key,content:normalized,providerOwner:entry.owner,providerHandle:entry.handle});
          guard();current(entry);
          const mirror=result&&options.mirror(result.document);
          if(!mirror||mirror.uri!==key||mirror.text!==normalized)throw new Error('Virtual document mirror was not available before open acknowledgement');
          shared.mirror=mirror;return mirror.model;
        }
        throw new Error(`Unable to resolve virtual document '${key}'`);
      }).finally(()=>{shared.token.dispose();actual--;if(opening.get(key)===shared)opening.delete(key);queueDrain();}));
    }
    const document=await shared.promise;guard();
    if(lookup(key)?.model!==document)throw new Error('Virtual document closed before open acknowledgement');
    return document;
  }
  function forOwner(owner) {
    return {registerTextDocumentContentProvider(scheme,rawProvider) {
      const guard=options.captureCaller(owner);guard();
      if(typeof scheme!=='string'||scheme.length>128||!/^[A-Za-z][A-Za-z0-9+.-]*$/.test(scheme)||['file','untitled'].includes(scheme.toLowerCase()))throw new Error('Invalid or reserved text content provider scheme');
      if(!rawProvider||!['object','function'].includes(typeof rawProvider))throw new TypeError('Invalid text document content provider');
      if(entries.size>=MAX_PROVIDERS)throw new Error('Text content provider registration limit exceeded');
      const capturedGeneration=generation,provider=bindProvider(owner,rawProvider);
      const method=provider.provideTextDocumentContent;
      if(typeof method!=='function')throw new TypeError('Text content provider requires provideTextDocumentContent');
      const event=provider.onDidChange;guard();
      if(!Number.isSafeInteger(nextHandle+1)||!Number.isSafeInteger(generation+entries.size+2))throw new Error('Text content provider identity exhausted');
      const entry={scheme,owner,handle:nextHandle+1,provider,guard};let cleanup;
      try {
        if(typeof event==='function') {
          const subscription=Reflect.apply(event,provider,[value=>unowned(()=>changed(entry,value))]);
          const dispose=bindResult(provider,()=>subscription?.dispose)();
          if(typeof dispose!=='function')throw new TypeError('Content provider event must return a disposable');
          cleanup=bindCleanup(provider,()=>Reflect.apply(dispose,subscription,[]));
        }
        guard();
        if(generation!==capturedGeneration||entries.size>=MAX_PROVIDERS)throw new Error('Text content provider registry changed during registration');
        nextHandle=entry.handle;entries.set(entry.handle,entry);advance();publish();
        const registration=new Disposable(()=>{
          if(entries.get(entry.handle)===entry) {
            entries.delete(entry.handle);advance();
            for(const [key,state] of refreshes) {
              if(state.entry===entry){cancelState(state);refreshes.delete(key);}
              else if(state.runningEntry===entry)state.token?.cancel();
            }
            for(const state of opening.values())if(state.entry===entry)state.token.cancel();
            try {publish();}finally {const release=cleanup;cleanup=undefined;release?.();}
          }
        });
        entry.dispose=()=>registration.dispose();
        return options.track(owner,registration);
      } catch(error) {
        if(entries.get(entry.handle)===entry){entries.delete(entry.handle);advance();try{publish();}catch{}}
        cleanup?.();throw error;
      }
    }};
  }
  function disposeOwner(owner) {for(const entry of [...entries.values()])if(entry.owner===owner)entry.dispose();}
  return {forOwner,open,documentChanged,disposeOwner,snapshot};
}
module.exports={createTextContentProviders};
