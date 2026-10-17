'use strict';
/* Copyright (c) Microsoft Corporation. MIT license. Source-guided from stable
 * 1.141.0 commit 2a59476c9bfcb90b3ddc372c36762471b7dfad1c:
 * extHostLanguageFeatures.ts, extHostApiCommands.ts, goToSymbol.ts,
 * referencesModel.ts. Optional-host bounds and serial provider scheduling are
 * native adaptations; public query results never move focus or edit text.
 */
const {Uri,Range}=require('./api-types.cjs');
const {Location}=require('./provider-types.cjs');
const coordinates=require('./coordinates.cjs');
const {uriSnapshot}=require('./language-selector.cjs');
const parseUri=Uri.parse.bind(Uri),formatUri=Uri.prototype.toString;
const COMMANDS=Object.freeze({
  'vscode.executeDefinitionProvider':'definition',
  'vscode.executeReferenceProvider':'references',
  'vscode.executeImplementationProvider':'implementation',
  'vscode.executeTypeDefinitionProvider':'typeDefinition',
  'vscode.executeDeclarationProvider':'declaration',
});
const MAX_RESULTS=512,MAX_BYTES=1024*1024;
function normalizeNavigation(type,value){
  if(value==null)return [];
  if(type==='references'&&!Array.isArray(value))return [];
  const values=Array.isArray(value)?value:[value];
  const count=values.length;
  if(!Number.isSafeInteger(count)||count>MAX_RESULTS)throw new RangeError('Navigation provider exceeds 512 locations');
  const result=[];let bytes=2;
  for(let index=0;index<count;index++){
    const item=values[index];if(!item||typeof item!=='object')throw new TypeError('Invalid navigation location');
    let converted;
    const targetUri=type==='references'?undefined:item.targetUri;
    if(targetUri){
      const targetRange=item.targetRange,targetSelectionRange=item.targetSelectionRange,originSelectionRange=item.originSelectionRange;
      converted={targetUri:uriSnapshot(targetUri),targetRange:coordinates.range(targetRange,'Navigation target')};
      if(targetSelectionRange!=null)converted.targetSelectionRange=coordinates.range(targetSelectionRange,'Navigation selection');
      if(originSelectionRange!=null)converted.originSelectionRange=coordinates.range(originSelectionRange,'Navigation origin');
    }else{
      const uri=item.uri,range=item.range;
      converted={uri:uriSnapshot(uri),range:coordinates.range(range,'Navigation location')};
    }
    bytes+=Buffer.byteLength(JSON.stringify(converted))+1;
    if(bytes>MAX_BYTES)throw new RangeError('Navigation provider exceeds 1 MiB');
    result.push(converted);
  }
  if(values.length!==count)throw new Error('Navigation result changed during normalization');
  return result;
}
function comparePosition(a,b){return a.line-b.line||a.character-b.character;}
function compareRange(a,b){return comparePosition(a.start,b.start)||comparePosition(a.end,b.end);}
function comparableUri(value){
  const uri=parseUri(value,true);
  // ReferencesModel compares full URI keys, including query and fragment.
  // URI's own serializer canonicalizes authority; ExtUri preserves path case.
  return Reflect.apply(formatUri,uri,[]);
}
function sortAndDedup(values,sourceUri){
  const sourceScheme=parseUri(sourceUri,true).scheme,internal=new Set(['walkThroughSnippet','vscode-chat-code-block','vscode-chat-code-compare-block']);
  const rows=values.filter(value=>{const scheme=parseUri(value.targetUri??value.uri,true).scheme;return scheme===sourceScheme||!internal.has(scheme);})
    .map(value=>({value,uri:comparableUri(value.targetUri??value.uri),range:value.targetRange??value.range}));
  rows.sort((a,b)=>(a.uri<b.uri?-1:a.uri>b.uri?1:0)||compareRange(a.range,b.range));
  return rows.filter((row,index)=>!index||row.uri!==rows[index-1].uri||compareRange(row.range,rows[index-1].range)!==0).map(row=>row.value);
}
function toPublic(value){
  const range=value=>new Range(value.start.line,value.start.character,value.end.line,value.end.character);
  if(value.targetUri!==undefined)return {targetUri:parseUri(value.targetUri,true),targetRange:range(value.targetRange),
    targetSelectionRange:value.targetSelectionRange?range(value.targetSelectionRange):undefined,
    originSelectionRange:value.originSelectionRange?range(value.originSelectionRange):undefined};
  return new Location(parseUri(value.uri,true),range(value.range));
}
function sameSource(a,b){return !!a&&!!b&&!b.isClosed&&a.model===b.model&&a.document===b.document&&a.version===b.version&&a.uri===b.uri&&a.languageEpoch===b.languageEpoch;}
function createNavigationProviders(options){
  let pending=0;
  async function execute(owner,id,args){
    const type=COMMANDS[id];if(!type)throw new Error('Unknown navigation provider command');
    const guard=options.captureCaller(owner),workspace=options.workspaceIdentity();guard();
    if(pending>=8)throw new Error('Navigation query invocation limit reached');
    if(args.length<2)throw new TypeError('Navigation query requires URI and Position');
    // Capture every trusted source before reading any extension-owned value.
    const sources=[...options.mirrors()];if(sources.length>128)throw new RangeError('Navigation query model inventory exceeds 128');
    const uri=uriSnapshot(args[0]),position=coordinates.position(args[1],'Navigation query');
    guard();if(options.workspaceIdentity()!==workspace)throw new Error('Navigation query workspace or settings retired');
    let source=sources.find(record=>record.uri===uri&&!record.isClosed);
    pending++;
    try{
      if(!source){
        const opened=await options.open(owner,parseUri(uri,true));guard();
        const published=options.sourceFor(opened);
        if(!published||published.isClosed||published.model!==opened)throw new Error('Navigation open mirror was not published');
        // A canonical file alias may return a retained original model. Never
        // replace its before-getter proof with newly observed source content.
        source=sources.find(record=>record.document===published.document)??published;
      }
      if(!source)throw new Error('Navigation query requires a native mirrored document');
      const original=source;
      const recheck=()=>{
        guard();if(options.workspaceIdentity()!==workspace||!sameSource(original,options.snapshot(original.document)))throw new Error('Navigation query source or workspace changed');
      };
      recheck();
      const result=await options.query(type,{...original,recheck},position);
      recheck();
      if(!Array.isArray(result)||result.length>MAX_RESULTS||Buffer.byteLength(JSON.stringify(result))>MAX_BYTES)throw new RangeError('Navigation query aggregate exceeds bounded result');
      const answer=sortAndDedup(result,source.uri).map(toPublic);recheck();return answer;
    }finally{pending--;}
  }
  return {execute,commands:Object.keys(COMMANDS),pending:()=>pending};
}
module.exports={COMMANDS,normalizeNavigation,sortAndDedup,createNavigationProviders};
