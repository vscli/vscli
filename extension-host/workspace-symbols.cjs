'use strict';
/* Copyright (c) Microsoft Corporation. MIT license. Source-guided stable
 * 1.141.0, commit 2a59476c9bfcb90b3ddc372c36762471b7dfad1c:
 * extHostLanguageFeatures.NavigateTypeAdapter, extHostTypeConverters.WorkspaceSymbol,
 * search.getWorkspaceSymbols and arrays.groupBy. Native adaptations bound
 * canonical metadata and retain exact original objects for lazy resolution.
 */
const {Uri,Range}=require('./api-types.cjs');
const {SymbolInformation,Location}=require('./provider-types.cjs');
const {uriSnapshot}=require('./language-selector.cjs');
const coordinates=require('./coordinates.cjs');
const parseUri=Uri.parse.bind(Uri);
const MAX_SYMBOLS=512,MAX_RESULT=1024*1024,MAX_CACHE=2*MAX_RESULT;
function string(value,max,label){if(typeof value!=='string'||value.length>max||Buffer.byteLength(value)>max)throw new TypeError(`Invalid or oversized workspace symbol ${label}`);return value;}
function normalizeSymbol(item,name){
  const kind=item.kind,tags=item.tags,containerName=item.containerName,location=item.location;
  if(!Number.isInteger(kind)||kind<0||kind>25)throw new TypeError('Invalid workspace symbol kind');
  if(!location||typeof location!=='object')throw new TypeError('Workspace symbol requires a location');
  const uri=location.uri,range=location.range;
  const result={name:string(name,4096,'name'),kind:kind+1,location:{uri:uriSnapshot(uri)}};
  if(range)result.location.range=coordinates.range(range,'Workspace symbol');
  if(containerName!==undefined)result.containerName=string(containerName,4096,'containerName');
  if(tags!==undefined){
    if(!Array.isArray(tags)||tags.length>16)throw new TypeError('Invalid workspace symbol tags');
    result.tags=[];const count=tags.length;
    for(let index=0;index<count;index++){if(tags[index]!==1)throw new TypeError('Invalid workspace symbol tag');result.tags.push(1);}
    if(tags.length!==count)throw new Error('Workspace symbol tags changed');
  }
  return result;
}
function normalize(value,diagnostic){
  if(value==null)return {symbols:[],originals:[]};
  if(!Array.isArray(value)||value.length>MAX_SYMBOLS)throw new RangeError('Workspace symbol result exceeds 512 entries');
  const count=value.length,symbols=[],originals=[];let bytes=2;
  for(let index=0;index<count;index++){
    const item=value[index],name=item?.name;
    if(!item||!name){diagnostic?.('Workspace symbol skipped: missing name');continue;}
    const symbol=normalizeSymbol(item,name);bytes+=Buffer.byteLength(JSON.stringify(symbol))+1;
    if(bytes>MAX_RESULT)throw new RangeError('Workspace symbols exceed 1 MiB');
    symbols.push(symbol);originals.push(item);
  }
  if(value.length!==count)throw new Error('Workspace symbol result changed during capture');
  return {symbols,originals};
}
function comparePosition(a,b){return a.line-b.line||a.character-b.character;}
function compareRange(a,b){return comparePosition(a.start,b.start)||comparePosition(a.end,b.end);}
function compareText(a,b){return a<b?-1:a>b?1:0;}
function intersects(a,b){return comparePosition(a.end,b.start)>0&&comparePosition(b.end,a.start)>0;}
function compareItems(a,b){
  const left=a.symbol,right=b.symbol;
  let result=compareText(left.name,right.name)||left.kind-right.kind||compareText(left.location.uri,right.location.uri);
  if(!result){
    if(left.location.range&&right.location.range){if(!intersects(left.location.range,right.location.range))result=compareRange(left.location.range,right.location.range);}
    else if(a.resolves!==b.resolves)result=a.resolves?-1:1;
  }
  return result||compareText(left.containerName??'',right.containerName??'');
}
function sortAndDedup(rows){
  const sorted=rows.slice().sort(compareItems),result=[];let first;
  for(const row of sorted){if(!first||compareItems(first,row)!==0){first=row;result.push(row);}}
  return result;
}
function toPublic(symbol){
  const value=symbol.location.range,range=value?new Range(value.start.line,value.start.character,value.end.line,value.end.character):undefined;
  const result=new SymbolInformation(symbol.name,symbol.kind-1,symbol.containerName,new Location(parseUri(symbol.location.uri,true),range));
  result.tags=symbol.tags?.slice();return result;
}
function createWorkspaceSymbols(options){
  const cache=new Map();let nextHandle=0;
  function current(cached){return cache.get(cached.handle)===cached&&options.registered(cached.entry)&&options.workspaceIdentity()===cached.workspace;}
  function clearEntry(entry){for(const [id,cached]of cache)if(cached.entry===entry)cache.delete(id);}
  function retire(origin,owner){for(const [id,cached]of cache)if(cached.origin===origin&&cached.entry.owner===owner)cache.delete(id);}
  function retain(call,converted,origin){
    if(!call.entry.resolves||typeof origin==='symbol')return {symbols:converted.symbols};
    let oldCount=0,bytes=0;for(const cached of cache.values())if(cached.entry===call.entry)oldCount++;else bytes+=cached.bytes;
    const count=converted.symbols.length;
    if(cache.size-oldCount+count>MAX_SYMBOLS||nextHandle>Number.MAX_SAFE_INTEGER-count)throw new RangeError('Workspace symbol handle budget or identity exhausted');
    const staged=[];
    for(let index=0;index<count;index++){
      const symbol=converted.symbols[index],handle=nextHandle+index+1;
      const payload=Buffer.byteLength(JSON.stringify(symbol))+128;bytes+=payload;
      if(bytes>MAX_CACHE)throw new RangeError('Workspace symbol cache exceeds 2 MiB');
      staged.push({handle,entry:call.entry,origin,workspace:call.workspace,original:converted.originals[index],symbol,bytes:payload});
    }
    clearEntry(call.entry);nextHandle+=count;
    for(const value of staged)cache.set(value.handle,value);
    return {symbols:staged.map(value=>({...value.symbol,_vscliWorkspaceSymbolHandle:value.handle}))};
  }
  function lookup(params,entry){
    const cached=cache.get(params.handle);
    if(!Number.isSafeInteger(params.handle)||params.handle<1||!cached||cached.entry!==entry||cached.origin!==params.origin||!current(cached))throw new Error('Stale workspace symbol resolve handle or original list');
    return cached;
  }
  function resolved(value,cached){
    if(!current(cached))throw new Error('Workspace symbol list retired during resolve');
    if(value==null)return null;
    const name=value.name;if(!name)throw new TypeError('Resolved workspace symbol requires a name');
    const symbol=normalizeSymbol(value,name);
    if(Buffer.byteLength(JSON.stringify(symbol))>MAX_RESULT)throw new RangeError('Resolved workspace symbol exceeds 1 MiB');
    if(!current(cached))throw new Error('Workspace symbol list retired during normalization');
    return {...symbol,_vscliWorkspaceSymbolHandle:cached.handle};
  }
  function purge(){for(const [id,cached]of cache)if(!current(cached))cache.delete(id);}
  return {normalize,retain,lookup,resolved,current,clearEntry,retire,purge,count:()=>{purge();return cache.size;}};
}
module.exports={createWorkspaceSymbols,normalize,sortAndDedup,toPublic,compareItems};
