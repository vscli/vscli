'use strict';
// Source contract: Microsoft VS Code stable 1.141.0, MIT License,
// commit 2a59476c9bfcb90b3ddc372c36762471b7dfad1c.
// extHostTextEditor.ts (type lifecycle/setDecorations), extHostTypeConverters.ts
// (DecorationRenderOptions, attachments, MarkdownString, range behavior).
// https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypeConverters.ts
// Typed inert DTOs preserve richer render/hover data; actual native rendering
// and hover qualification are separate from this bounded host gateway.
const {Uri,Disposable}=require('./api-types.cjs');
const coordinates=require('./coordinates.cjs');
const DecorationRangeBehavior=Object.freeze({OpenOpen:0,ClosedClosed:1,OpenClosed:2,ClosedOpen:3});
const OverviewRulerLane=Object.freeze({Left:1,Center:2,Right:4,Full:7});
const STYLE=Object.freeze(['backgroundColor','outline','outlineColor','outlineStyle','outlineWidth','border','borderColor','borderRadius','borderSpacing','borderStyle','borderWidth','fontStyle','fontWeight','textDecoration','cursor','color','opacity','letterSpacing','gutterIconPath','gutterIconSize','overviewRulerColor']);
const ATTACHMENT=Object.freeze(['contentText','contentIconPath','border','borderColor','fontStyle','fontWeight','textDecoration','color','backgroundColor','margin','width','height']);
const COLORS=new Set(['backgroundColor','outlineColor','borderColor','color','overviewRulerColor']);
const originalUriFrom=Uri.from,originalUriFile=Uri.file,originalUriFormat=Uri.prototype.toString;
function budget(max){
  let used=0;
  function charge(bytes){if(bytes>max-used)throw new RangeError('Decoration capture payload limit reached');used+=bytes;}
  function text(value){
    // Exact JSON string byte count without allocating an escaped copy.
    let bytes=2;
    for(let i=0;i<value.length;i++){
      const code=value.charCodeAt(i);
      if(code===34||code===92)bytes+=2;
      else if(code<32)bytes+=[8,9,10,12,13].includes(code)?2:6;
      else if(code<128)bytes++;
      else if(code<2048)bytes+=2;
      else if(code>=0xd800&&code<=0xdbff){
        const next=value.charCodeAt(i+1);if(next>=0xdc00&&next<=0xdfff){bytes+=4;i++;}else bytes+=6;
      }else if(code>=0xdc00&&code<=0xdfff)bytes+=6;
      else bytes+=3;
      if(bytes>max-used)throw new RangeError('Decoration capture payload limit reached');
    }
    charge(bytes+32);
  }
  return {charge,text};
}
function string(value,max,name,bound){
  if(typeof value!=='string'||value.length>max||Buffer.byteLength(value)>max)throw new TypeError(`Invalid or oversized decoration ${name}`);
  bound?.text(value);return value;
}
function icon(value,bound){
  let uri;
  if(typeof value==='string')uri=Reflect.apply(originalUriFile,Uri,[string(value,4096,'icon path')]);
  else{
    if(!(value instanceof Uri))throw new TypeError('Decoration icon/base path must be a Uri or string');
    const scheme=value.scheme,authority=value.authority,path=value.path,query=value.query,fragment=value.fragment;
    for(const part of [scheme,authority,path,query,fragment])string(part,4096,'URI component');
    uri=Reflect.apply(originalUriFrom,Uri,[{scheme,authority,path,query,fragment},true]);
  }
  return string(Reflect.apply(originalUriFormat,uri,[]),4096,'URI',bound);
}
function color(value,bound){
  if(typeof value==='string')return string(value,4096,'color',bound);
  if(!value||typeof value!=='object')throw new TypeError('Decoration color must be a string or ThemeColor');
  const id=value.id;string(id,256,'ThemeColor',bound);if(!id)throw new TypeError('Decoration ThemeColor must not be empty');
  return {id};
}
function style(value,attachment=false,bound){
  if(!value||typeof value!=='object'||Array.isArray(value))throw new TypeError('Decoration render options must be an object');
  bound?.charge(32);const result={};
  for(const key of attachment?ATTACHMENT:STYLE){
    const item=value[key];if(item===undefined)continue;
    result[key]=key==='gutterIconPath'||key==='contentIconPath'?icon(item,bound):COLORS.has(key)?color(item,bound):string(item,4096,key,bound);
  }
  if(!attachment){
    for(const key of ['before','after']){const item=value[key];if(item!==undefined)result[key]=style(item,true,bound);}
  }
  return result;
}
function renderOptions(value,typeOptions=false,bound){
  const result=style(value,false,bound);
  for(const key of ['light','dark']){const item=value[key];if(item!==undefined)result[key]=style(item,false,bound);}
  if(typeOptions){
    bound?.charge(96);const whole=value.isWholeLine,behavior=value.rangeBehavior,lane=value.overviewRulerLane;
    if(whole!==undefined){if(typeof whole!=='boolean')throw new TypeError('Decoration isWholeLine must be a boolean');result.isWholeLine=whole;}
    if(behavior!==undefined){if(!Number.isInteger(behavior)||behavior<0||behavior>3)throw new TypeError('Invalid DecorationRangeBehavior');result.rangeBehavior=behavior;}
    if(lane!==undefined){if(!Number.isInteger(lane)||lane<1||lane>7)throw new TypeError('Invalid OverviewRulerLane');result.overviewRulerLane=lane;}
  }
  return result;
}
function markup(value,bound){
  bound?.charge(160);
  if(typeof value==='string')return {value:string(value,65536,'hover text',bound)};
  if(!value||typeof value!=='object')throw new TypeError('Decoration hover must be text or MarkdownString');
  const content=value.value,language=value.language;
  string(content,65536,'hover markdown',bound);
  if(language!==undefined){
    string(language,128,'hover language',bound);
    if(content.length+language.length+9>65536||Buffer.byteLength(content)+Buffer.byteLength(language)+9>65536)throw new TypeError('Oversized decoration hover codeblock');
    return {value:'```'+language+'\n'+content+'\n```\n'};
  }
  const trusted=value.isTrusted,html=value.supportHtml,icons=value.supportThemeIcons,alerts=value.supportAlertSyntax,base=value.baseUri;
  const result={value:content};
  for(const [key,item] of [['supportHtml',html],['supportThemeIcons',icons],['supportAlertSyntax',alerts]]){
    if(item!==undefined){if(typeof item!=='boolean')throw new TypeError(`Invalid decoration ${key}`);result[key]=item;}
  }
  if(trusted!==undefined){
    if(typeof trusted==='boolean')result.isTrusted=trusted;
    else{
      if(!trusted||typeof trusted!=='object')throw new TypeError('Invalid decoration markdown trust');
      const commands=trusted.enabledCommands;
      if(!Array.isArray(commands))throw new TypeError('Invalid decoration markdown command list');
      const length=commands.length;if(!Number.isInteger(length)||length<0||length>32)throw new TypeError('Invalid decoration markdown command list');
      const items=[];bound?.charge(length*32);
      for(let i=0;i<length;i++)items.push(string(commands[i],256,'markdown command',bound));
      result.isTrusted={enabledCommands:items};
    }
  }
  if(base!==undefined){if(!(base instanceof Uri))throw new TypeError('Decoration markdown baseUri must be a Uri');result.baseUri=icon(base,bound);}
  return result;
}
function hover(value,bound){
  if(value===undefined||value===null||value==='')return undefined;
  if(!Array.isArray(value))return markup(value,bound);
  const length=value.length;if(!Number.isInteger(length)||length<0||length>32)throw new RangeError('Decoration hover item limit exceeded');
  const items=[];for(let i=0;i<length;i++)items.push(markup(value[i],bound));return items;
}
function wireBytes(value,max,name){
  const encoded=JSON.stringify(value);
  if(encoded.length>max)throw new RangeError(`Decoration ${name} payload limit reached`);
  const bytes=Buffer.byteLength(encoded);if(bytes>max)throw new RangeError(`Decoration ${name} payload limit reached`);return bytes;
}
function createEditorDecorations({session,enabled=false,notify,call,captureCaller,track,captureEditor,diagnostic=()=>{}}){
  const branded=new WeakMap(),live=new Map();let sequence=0,optionBytes=0;
  function retire(entry){
    if(!entry.alive)return;
    entry.alive=false;live.delete(entry.key);optionBytes-=entry.bytes;
    notify('decorationType',{session,owner:entry.owner,key:entry.key,action:'dispose'});
  }
  function assertType(entry){
    if(!entry||!entry.alive||live.get(entry.key)!==entry)throw new Error('Decoration type is disposed or unknown');
    entry.guard();
  }
  function createTextEditorDecorationType(owner,options){
    if(!enabled)throw new Error('Native editor decorations are not implemented by this host');
    const guard=captureCaller(owner);guard();
    if(live.size>=256||sequence>=4096)throw new RangeError('Decoration type limit reached');
    const projected=renderOptions(options,true,budget(16384)),bytes=wireBytes(projected,16384,'type');
    guard();
    if(live.size>=256||sequence>=4096||bytes>1024*1024-optionBytes)throw new RangeError('Decoration type limit reached');
    const key=`decoration-${++sequence}`,entry={key,owner,guard,bytes,alive:false,editors:new WeakSet(),disposable:undefined};
    const owned=track(owner,new Disposable(()=>retire(entry)));entry.disposable=owned;
    const result=Object.freeze({key,dispose(){owned.dispose();}});branded.set(result,entry);
    entry.alive=true;live.set(key,entry);optionBytes+=bytes;
    try{notify('decorationType',{session,owner,key,action:'create',options:projected});guard();assertType(entry);}
    catch(error){try{owned.dispose();}catch{/* Preserve the original creation failure. */}throw error;}
    return result;
  }
  function setDecorations(owner,document,base,type,input){
    const caller=captureCaller(owner),captured=captureEditor(owner,document,base),entry=branded.get(type);
    caller();assertType(entry);
    if(!Array.isArray(input))throw new TypeError('Decorations must be an array of Range or DecorationOptions');
    const length=input.length;if(!Number.isInteger(length)||length<0||length>4096)throw new RangeError('Decoration range limit exceeded');
    const decorations=[],bound=budget(1024*1024);bound.charge(256);let optionsArray;
    for(let i=0;i<length;i++){
      const item=input[i];
      if(!item||typeof item!=='object')throw new TypeError('Invalid decoration range');
      // The original converter chooses one array form from its first element.
      // Capture that element's range getter once rather than probing it twice.
      let value=item;
      if(i===0){const candidate=item.range;optionsArray=candidate!==undefined;if(optionsArray)value=candidate;}
      else if(optionsArray)value=item.range;
      bound.charge(128);
      const range=coordinates.range(value,'Decoration');
      const projected={range};
      if(optionsArray){
        const message=item.hoverMessage,render=item.renderOptions;
        const normalized=hover(message,bound);if(normalized!==undefined)projected.hoverMessage=normalized;
        if(render!==undefined)projected.renderOptions=renderOptions(render,false,bound);
      }
      decorations.push(projected);
    }
    const args={document:captured.source.document,version:captured.source.version,editor:captured.state.nativeId,key:entry.key,decorations};
    wireBytes(args,1024*1024,'setter');
    caller();captured.recheck();assertType(entry);
    if(!length&&!entry.editors.has(base))return;
    if(length)entry.editors.add(base);else entry.editors.delete(base);
    call(owner,'nativeEditorDecorations',args).then(result=>{
      caller();assertType(entry);captured.accept({document:result?.document,editor:result?.editor});
      if(result?.key!==entry.key)throw new Error('Native decoration acknowledgement did not match original type');
    }).catch(error=>{try{diagnostic(owner,error);}catch{/* Void API errors must not become unhandled rejections. */}});
  }
  return {forOwner(owner){const guard=captureCaller(owner);return{createTextEditorDecorationType(options){guard();return createTextEditorDecorationType(owner,options);}};},setDecorations,
    disposeOwner(owner){for(const entry of [...live.values()])if(entry.owner===owner){try{entry.disposable.dispose();}catch(error){try{diagnostic(owner,error);}catch{/* Cleanup continues for all original types. */}}}},
    types:{DecorationRangeBehavior,OverviewRulerLane}};
}
module.exports={createEditorDecorations,DecorationRangeBehavior,OverviewRulerLane};
