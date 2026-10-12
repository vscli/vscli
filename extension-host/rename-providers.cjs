'use strict';
// Copyright (c) Microsoft Corporation. MIT license. Source-guided from VS Code
// 1.141.0 (2a59476c9bfcb90b3ddc372c36762471b7dfad1c), RenameSkeleton and
// RenameAdapter. Native bounds and private mirror authority are adaptations.
const {Uri,Range}=require('./api-types.cjs');
const coordinates=require('./coordinates.cjs');
const {uriSnapshot}=require('./language-selector.cjs');
const parseUri=Uri.parse.bind(Uri);
const COMMANDS=['vscode.prepareRename','vscode.executeDocumentRenameProvider'];
function boundedText(value){
  if(typeof value!=='string'||value.length>65536||Buffer.byteLength(value)>65536)throw new TypeError('Rename text exceeds 64 KiB or has invalid type');
  return value;
}
function sourceText(source){
  const value=source.canonicalText??source.text;
  if(typeof value!=='string')throw new Error('Rename requires private native source text');
  return value;
}
function offset(text,at){
  let start=0;
  for(let line=0;line<at.line;line++){const lf=text.indexOf('\n',start);if(lf<0)throw new RangeError('Rename position outside document');start=lf+1;}
  const lf=text.indexOf('\n',start),end=lf<0?text.length:lf>start&&text[lf-1]==='\r'?lf-1:lf;
  const result=start+at.character;
  if(result>end||result>start&&result<end&&/[\uD800-\uDBFF]/.test(text[result-1])&&/[\uDC00-\uDFFF]/.test(text[result]))throw new RangeError('Rename position outside document or splits UTF-16');
  return result;
}
function defaultLocation(source,at){
  const text=sourceText(source),absolute=offset(text,at),lineStart=absolute-at.character;
  const lf=text.indexOf('\n',lineStart),lineEnd=lf<0?text.length:lf>lineStart&&text[lf-1]==='\r'?lf-1:lf;
  const line=text.slice(lineStart,lineEnd),column=at.character;
  // Pinned default word helper bounds scanning to 1000 UTF-16 units. Adjust
  // window endpoints so native UTF-8 models never receive split surrogates.
  let begin=Math.max(0,column-500),end=Math.min(line.length,column+500);
  if(begin>0&&/[\uDC00-\uDFFF]/.test(line[begin]))begin--;
  if(end<line.length&&/[\uDC00-\uDFFF]/.test(line[end]))end++;
  const window=line.slice(begin,end),pattern=/(-?\d*\.\d\w*)|([^`~!@#$%^&*()\-=+\[\]{}\\|;:'",.<>/?\s]+)/g;
  for(const match of window.matchAll(pattern)){
    const start=begin+match.index,finish=start+match[0].length;
    if(start<=column&&column<=finish)return {range:{start:{line:at.line,character:start},end:{line:at.line,character:finish}},placeholder:match[0]};
  }
  return {range:{start:{...at},end:{...at}},placeholder:''};
}
function normalizePrepare(value,source,at){
  if(value==null)return null;
  let range,placeholder;
  if(Range.isRange(value)){
    range=coordinates.range(value,'Rename preparation');
    const text=sourceText(source);placeholder=text.slice(offset(text,range.start),offset(text,range.end));
  }else if(typeof value==='object'){
    const original=value.range;placeholder=value.placeholder;
    if(!original||!placeholder)return null;
    range=coordinates.range(original,'Rename preparation');
  }
  if(!range||!placeholder)return null;
  if(range.start.line>at.line||range.end.line<at.line)return null;
  return {range,placeholder:boundedText(placeholder)};
}
function rejection(error){
  const message=typeof error==='string'?error:error instanceof Error?error.message:undefined;
  return message?{rejectReason:boundedText(message)}:undefined;
}
async function queryRename(selected,source,at,newName,invoke){
  let index=0;const reasons=[];
  let location;
  for(;index<selected.length;index++){
    source.recheck();if(!selected[index].resolves)break;
    const value=await invoke(selected[index],true);source.recheck();
    if(!value)continue;
    if(value.rejectReason){reasons.push(value.rejectReason);continue;}
    location=value;break;
  }
  if(!location){index=0;location=defaultLocation(source,at);if(reasons.length)location.rejectReason=reasons.join('\n');}
  if(newName===undefined)return location;
  if(location.rejectReason)return {edits:[],rejectReason:location.rejectReason};
  const rejects=[];
  for(;index<selected.length;index++){
    source.recheck();const value=await invoke(selected[index],false);source.recheck();
    if(!value){rejects.push('No result.');continue;}
    if(value.rejectReason){rejects.push(value.rejectReason);continue;}
    return value;
  }
  return {edits:[],rejectReason:rejects.join('\n')};
}
function sameSource(original,current){return !!current&&!current.isClosed&&current.model===original.model&&current.document===original.document&&current.version===original.version&&current.languageEpoch===original.languageEpoch&&current.uri===original.uri;}
function createRenameProviders(options){
  let pending=0;
  async function execute(owner,id,args){
    const guard=options.captureCaller(owner),workspace=options.workspaceIdentity();guard();
    if(pending>=8)throw new Error('Rename query invocation limit reached');
    if(!COMMANDS.includes(id)||args.length<2)throw new TypeError('Rename query requires URI and Position');
    const sources=[...options.mirrors()];if(sources.length>128)throw new RangeError('Rename model inventory exceeds 128');
    const uri=uriSnapshot(args[0]),at=coordinates.position(args[1],'Rename query');
    const newName=id===COMMANDS[1]?boundedText(args[2]):undefined;
    guard();if(options.workspaceIdentity()!==workspace)throw new Error('Rename query workspace changed');
    let source=sources.find(value=>value.uri===uri&&!value.isClosed);pending++;
    try{
      if(!source){
        const opened=await options.open(owner,parseUri(uri,true));guard();
        const published=options.sourceFor(opened);
        if(!published||published.isClosed||published.model!==opened)throw new Error('Rename open mirror was not published');
        source=sources.find(value=>value.document===published.document)??published;
      }
      const original=source;
      const recheck=()=>{guard();if(options.workspaceIdentity()!==workspace||!sameSource(original,options.snapshot(original.document)))throw new Error('Rename query source or workspace changed');};
      recheck();offset(sourceText(source),at);
      const value=await options.query({...source,recheck},at,newName);recheck();
      if(value.rejectReason)throw new Error(value.rejectReason);
      const result=newName===undefined?{range:new Range(value.range.start.line,value.range.start.character,value.range.end.line,value.range.end.character),placeholder:value.placeholder}:require('./workspace-edit-values.cjs').toPublicWorkspaceEdit(value);
      recheck();return result;
    }finally{pending--;}
  }
  return {commands:COMMANDS,execute,pending:()=>pending};
}
module.exports={boundedText,defaultLocation,normalizePrepare,rejection,queryRename,createRenameProviders};
