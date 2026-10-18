'use strict';
/* Copyright (c) Microsoft Corporation. MIT license.
 * Source-guided selector scoring: stable 1.141.0, 2a59476c9bfcb90b3ddc372c36762471b7dfad1c
 * src/vs/editor/common/languageSelector.ts and api/common/extHostTypeConverters.ts.
 * Native adaptation: bounded snapshots; notebook filters require a real backend.
 */
const path=require('node:path');
const {Uri}=require('./api-types.cjs');
const {compileGlob}=require('./watcher-glob.cjs');
const parseUri=Uri.parse.bind(Uri),fromUri=Uri.from.bind(Uri),fileUri=Uri.file.bind(Uri),formatUri=Uri.prototype.toString;
const fsPath=Object.getOwnPropertyDescriptor(Uri.prototype,'fsPath').get;
const matchers=new WeakMap();
function bounded(value,max,label){
  if(typeof value!=='string'||value.length>max||Buffer.byteLength(value)>max||value.includes('\0'))throw new TypeError(`Invalid or over-budget selector ${label}`);
  return value;
}
function uriSnapshot(value){
  if(!value||typeof value!=='object')throw new TypeError('Selector RelativePattern baseUri requires a URI');
  const fields={};let bytes=0;
  for(const key of ['scheme','authority','path','query','fragment']){fields[key]=bounded(value[key],4096,'base URI');bytes+=Buffer.byteLength(fields[key]);}
  if(bytes>4096)throw new RangeError('Selector base URI exceeds 4096 bytes');
  return bounded(Reflect.apply(formatUri,fromUri(fields,true),[]),4096,'base URI');
}
function capturePattern(value){
  if(typeof value==='string')return bounded(value,4096,'pattern');
  if(!value||typeof value!=='object')throw new TypeError('Selector pattern requires a glob or RelativePattern');
  const baseUri=value.baseUri,base=baseUri===undefined?value.base:undefined,pattern=bounded(value.pattern,4096,'pattern');
  return {baseUri:baseUri===undefined?uriSnapshot(fileUri(bounded(base,4096,'base'))):uriSnapshot(baseUri),pattern};
}
function matcher(pattern){
  if(typeof pattern==='string'){const glob=compileGlob(pattern);return file=>pattern===file||glob(file);}
  const base=path.normalize(Reflect.apply(fsPath,parseUri(pattern.baseUri,true),[])),glob=compileGlob(pattern.pattern);
  return file=>{
    const actual=process.platform==='linux'?file:file.toLowerCase(),parent=process.platform==='linux'?base:base.toLowerCase();
    const boundary=parent.endsWith(path.sep)?parent:parent+path.sep;
    if(actual!==parent&&!actual.startsWith(boundary))return false;
    let offset=base.length;while(file[offset]===path.sep)offset++;
    return glob(file.slice(offset));
  };
}
function selector(value){
  const values=Array.isArray(value)?value:[value];
  if(values.length>32)throw new RangeError('Provider selector exceeds 32 entries');
  const result=[];let bytes=0;
  for(let index=0;index<values.length;index++){
    const entry=values[index];if(entry==null)continue;
    let captured;
    if(typeof entry==='string')captured={language:bounded(entry,128,'language')};
    else{
      if(typeof entry!=='object')throw new TypeError('Invalid provider selector');
      const language=entry.language,scheme=entry.scheme,pattern=entry.pattern,notebookType=entry.notebookType,exclusive=entry.exclusive;
      if(notebookType)throw new Error('Notebook language selectors require an unsupported native notebook backend');
      captured={};
      if(language!==undefined&&language!==null)captured.language=bounded(language,128,'language');
      if(scheme!==undefined&&scheme!==null)captured.scheme=bounded(scheme,128,'scheme');
      if(pattern!==undefined&&pattern!==null)captured.pattern=capturePattern(pattern);
      if(exclusive!==undefined){if(typeof exclusive!=='boolean')throw new TypeError('Selector exclusive must be boolean');captured.exclusive=exclusive;}
    }
    bytes+=Buffer.byteLength(JSON.stringify(captured));if(bytes>128*1024)throw new RangeError('Provider selector exceeds 128 KiB');
    if(captured.pattern)matchers.set(captured,matcher(captured.pattern));
    result.push(captured);
  }
  return result;
}
function score(entries,document){
  const uri=typeof document.uri==='string'?parseUri(document.uri,true):document.uri;
  let best=0;
  for(const entry of entries){
    let result=0,mismatch=false;
    for(const [key,actual]of [['scheme',uri.scheme],['language',document.languageId]]){
      if(!entry[key])continue;
      if(entry[key]===actual)result=10;else if(entry[key]==='*')result=Math.max(result,5);else{mismatch=true;break;}
    }
    if(mismatch)continue;
    if(entry.pattern){const test=matchers.get(entry)??matcher(entry.pattern);if(!test(Reflect.apply(fsPath,uri,[])))continue;result=10;}
    best=Math.max(best,result);if(best===10)return best;
  }
  return best;
}
module.exports={selector,score,uriSnapshot};
