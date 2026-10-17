'use strict';
/*---------------------------------------------------------------------------------------------
 * Copyright (c) Microsoft Corporation. All rights reserved.
 * Licensed under the MIT License. Source: immutable stable VS Code 1.141.0,
 * 2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/languageFeatureRegistry.ts.
 * The source's current entry order, pre-sort exclusive scan, per-family clock
 * and MatchCandidate cache are retained. Native adaptations bound admitted
 * entries/clock and take already captured, runtime-private document values.
 *--------------------------------------------------------------------------------------------*/
const {Uri}=require('./api-types.cjs');
const {score}=require('./language-selector.cjs');
const parseUri=Uri.parse.bind(Uri),formatUri=Uri.prototype.toString;
function key(uri){return uri===undefined?undefined:Reflect.apply(formatUri,typeof uri==='string'?parseUri(uri,true):uri,[]);}
function isExclusive(selector){return Array.isArray(selector)?selector.every(isExclusive):typeof selector==='string'?false:!!selector?.exclusive;}
function createProviderOrdering({initialClock=0}={}){
  if(!Number.isSafeInteger(initialClock)||initialClock<0)throw new RangeError('Invalid provider ordering clock');
  const entries=[];let clock=initialClock,lastCandidate;
  function register(provider,isBuiltin=false){
    if(entries.length>=128)throw new RangeError('Provider ordering exceeds 128 entries');
    if(clock>=Number.MAX_SAFE_INTEGER)throw new RangeError('Provider ordering clock exhausted');
    if(entries.some(entry=>entry.provider===provider))throw new Error('Provider already registered in this family');
    if(typeof isBuiltin!=='boolean')throw new TypeError('Trusted builtin provider metadata must be boolean');
    entries.push({provider,selector:provider.selector,time:clock++,score:-1,isBuiltin});lastCandidate=undefined;
  }
  function remove(provider){
    const index=entries.findIndex(entry=>entry.provider===provider);
    if(index<0)return false;
    entries.splice(index,1);lastCandidate=undefined;return true;
  }
  function ordered(source,recursive=false){
    const candidate={uri:key(source.uri),language:source.languageId,notebookUri:key(source.notebookUri),notebookType:source.notebookType,recursive};
    if(!lastCandidate||Object.keys(candidate).some(field=>lastCandidate[field]!==candidate[field])){
      lastCandidate=candidate;
      // Scan the CURRENT array before sorting. Registration appends to that
      // array, so neither newest-only nor highest-score-exclusive is equivalent.
      for(const entry of entries){
        entry.score=score(entry.selector,source);
        if(isExclusive(entry.selector)&&entry.score>0){
          if(recursive)entry.score=0;
          else{for(const other of entries)other.score=0;entry.score=1000;break;}
        }
      }
      entries.sort((a,b)=>b.score-a.score||Number(a.isBuiltin)-Number(b.isBuiltin)||b.time-a.time);
    }
    return entries.filter(entry=>entry.score>0).map(entry=>entry.provider);
  }
  return {register,remove,ordered,size:()=>entries.length};
}
module.exports={createProviderOrdering,isExclusive};
