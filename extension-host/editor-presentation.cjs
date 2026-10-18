/*---------------------------------------------------------------------------------------------
 * Copyright (c) Microsoft Corporation. All rights reserved.
 * Licensed under the MIT License. See https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/LICENSE.txt
 * Source-derived ExtHostTextEditorOptions normalization; native request proofs
 * and bounded snapshot handling are VSCLI adaptations.
 * https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTextEditor.ts
 *--------------------------------------------------------------------------------------------*/
'use strict';
const {Range}=require('./api-types.cjs');
const coordinates=require('./coordinates.cjs');
const FIELDS=Object.freeze(['tabSize','indentSize','insertSpaces','cursorStyle','lineNumbers']);
const TextEditorCursorStyle=Object.freeze({Line:1,Block:2,Underline:3,LineThin:4,BlockOutline:5,UnderlineThin:6});
const TextEditorLineNumbersStyle=Object.freeze({Off:0,On:1,Relative:2,Interval:3});
const TextEditorRevealType=Object.freeze({Default:0,InCenter:1,InCenterIfOutsideViewport:2,AtTop:3});
function positive(value){return Number.isSafeInteger(value)&&value>0;}
function decodeOptions(value){
  if(value===undefined)return undefined;
  if(!value||typeof value!=='object'||!positive(value.tabSize)||!positive(value.indentSize)||
      !(positive(value.originalIndentSize)||value.originalIndentSize==='tabSize')||typeof value.insertSpaces!=='boolean'||
      !Number.isInteger(value.cursorStyle)||value.cursorStyle<1||value.cursorStyle>6||!Number.isInteger(value.lineNumbers)||value.lineNumbers<0||value.lineNumbers>3)
    throw new Error('Invalid native editor options');
  return Object.freeze({tabSize:value.tabSize,indentSize:value.indentSize,originalIndentSize:value.originalIndentSize,
    insertSpaces:value.insertSpaces,cursorStyle:value.cursorStyle,lineNumbers:value.lineNumbers});
}
function decodeVisibleRanges(value){
  if(value===undefined)return undefined;
  if(!Array.isArray(value)||value.length>4096)throw new Error('Invalid native visible ranges');
  return Object.freeze(value.map(range=>Object.freeze(coordinates.range(range,'Visible range'))));
}
function integerOption(value,special){
  if(value===special)return special;
  let result;
  if(typeof value==='number')result=Math.floor(value);
  else if(typeof value==='string'){
    if(value.length>4096||Buffer.byteLength(value)>4096)throw new RangeError('Editor option string exceeds 4096 bytes');
    result=parseInt(value,10);
  }else return null;
  if(Number.isNaN(result)||result<=0)return null;
  if(!Number.isSafeInteger(result))throw new RangeError('Editor option exceeds the native safe integer range');
  return result;
}
function normalizeOptions(input,current,field){
  if(!current)throw new Error('Native editor options are not available');
  const values={};
  if(field)values[field]=input;
  else {
    // Snapshot each extension getter once. The caller rechecks its original
    // lease, source and pane after this entire capture, before presentation/RPC.
    if(input===null||input===undefined)throw new TypeError('Editor options must be an object');
    for(const key of FIELDS)values[key]=input[key];
  }
  const update={},optimistic={...current};
  for(const key of FIELDS){
    if(field&&key!==field)continue;
    let value=values[key];
    if(!field&&value===undefined)continue;
    if(key==='tabSize'||key==='indentSize'){
      value=integerOption(value,key==='tabSize'?'auto':'tabSize');
      if(value===null)continue;
      const old=key==='indentSize'?current.originalIndentSize:current.tabSize;
      if(typeof value==='number'&&value===old)continue;
      update[key]=value;
      if(typeof value==='number'){
        optimistic[key]=value;
        if(key==='indentSize')optimistic.originalIndentSize=value;
      }
    }else if(key==='insertSpaces'){
      value=value==='auto'?'auto':value==='false'?false:Boolean(value);
      if(typeof value==='boolean'&&value===current.insertSpaces)continue;
      update[key]=value;if(typeof value==='boolean')optimistic[key]=value;
    }else{
      const low=key==='cursorStyle'?1:0,high=key==='cursorStyle'?6:3;
      if(!Number.isInteger(value)||value<low||value>high)throw new TypeError(`Invalid editor ${key}`);
      if(value===current[key])continue;
      update[key]=value;optimistic[key]=value;
    }
  }
  return {update,optimistic};
}
function createEditorPresentation(){
  let actual,overlay,ranges=[],optionsKnown=false,rangesKnown=false;
  const current=()=>overlay?.values||actual;
  function accept(options,visibleRanges){
    let optionsChanged=false,rangesChanged=false;
    if(options!==undefined){
      optionsChanged=optionsKnown&&FIELDS.some(key=>actual[key]!==options[key]);
      actual=options;optionsKnown=true;overlay=undefined;
    }
    if(visibleRanges!==undefined){
      const next=visibleRanges;
      rangesChanged=rangesKnown&&(ranges.length!==next.length||next.some((value,index)=>{
        const old=ranges[index];return value.start.line!==old.start.line||value.start.character!==old.start.character||value.end.line!==old.end.line||value.end.character!==old.end.character;
      }));
      ranges=next;rangesKnown=true;
    }
    return {optionsChanged,rangesChanged,options:actual,visibleRanges:publicRanges()};
  }
  function publicRanges(){return ranges.map(value=>new Range(value.start,value.end));}
  return {current,accept,visibleRanges:publicRanges,
    optionsValue(assign){
      const result={};
      for(const key of FIELDS)Object.defineProperty(result,key,{enumerable:true,get(){return current()?.[key];},set(value){assign(value,key);}});
      return result;
    },
    stage(owner,values){const entry={owner,values};overlay=entry;return()=>{if(overlay===entry)overlay=undefined;};},
    retire(owner){if(owner===undefined||overlay?.owner===owner)overlay=undefined;},
  };
}
module.exports={createEditorPresentation,normalizeOptions,decodeOptions,decodeVisibleRanges,TextEditorCursorStyle,TextEditorLineNumbersStyle,TextEditorRevealType};
