'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createEditorDecorations,DecorationRangeBehavior,OverviewRulerLane}=require('./editor-decorations.cjs');
const {Range,Uri,Disposable}=require('./api-types.cjs');
const {ThemeColor}=require('./surfaces.cjs');
const {MarkdownString}=require('./markdown-string.cjs');
const {createCallerContext}=require('./caller-context.cjs');
const {createNativeRequests}=require('./native-requests.cjs');
const {createApi}=require('./api.cjs');
const tick=()=>new Promise(resolve=>setImmediate(resolve));
function held(){let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};}
function fixture(handler,settings={}){
  const callers=createCallerContext(()=>{}),messages=[],calls=[],errors=[],owned=new Map(),document={},base={};let version=1,epoch=0,alive=true;
  const module=createEditorDecorations({session:7,enabled:true,notify(method,params){messages.push({method,params});settings.notify?.(method,params);},
    captureCaller:owner=>callers.bind(owner,()=>{}),track(owner,value){if(!owned.has(owner))owned.set(owner,[]);owned.get(owner).push(value);return value;},
    captureEditor(owner,doc,editor){const guard=callers.bind(owner,()=>{}),source={document:1,version},captured=epoch;if(doc!==document||editor!==base||!alive)throw Error('Retired editor');
      return{source,state:{nativeId:'left'},recheck(){guard();if(!alive||version!==source.version||epoch!==captured)throw Error('Editor source changed');},accept(value){guard();if(!alive||value.document!==1||value.editor!=='left')throw Error('Retired editor receipt');}};},
    call(owner,method,args){calls.push({owner,method,args});return Promise.resolve(handler?handler({owner,method,args}):{document:1,editor:'left',key:args.key});},diagnostic(owner,error){errors.push({owner,error});},...settings.dependencies});
  return{module,document,base,messages,calls,errors,callers,create:module.forOwner('a').createTextEditorDecorationType,
    set(type,ranges,owner='a'){return module.setDecorations(owner,document,base,type,ranges);},editUndo(){epoch++;},edit(){version++;epoch++;},hide(){alive=false;},retireOwner(owner){callers.retire(owner);module.disposeOwner(owner);}};
}
test('private frozen types create before setter, exact keys and idempotent disposal cannot be forged or reused',async()=>{
  const f=fixture(),type=f.create({color:'#ff0000',rangeBehavior:DecorationRangeBehavior.ClosedClosed});assert(Object.isFrozen(type));assert.equal(type.key,'decoration-1');assert.equal(f.messages[0].params.action,'create');
  f.set(type,[new Range(0,0,0,2)]);assert.equal(f.calls[0].method,'nativeEditorDecorations');assert.deepEqual(f.calls[0].args,{document:1,version:1,editor:'left',key:type.key,decorations:[{range:{start:{line:0,character:0},end:{line:0,character:2}}}]});await tick();
  type.dispose();type.dispose();assert.deepEqual(f.messages.map(value=>value.params.action),['create','dispose']);assert.throws(()=>f.set(type,[]),/disposed/);assert.throws(()=>f.set({key:type.key},[]),/unknown/);assert.equal(f.create({}).key,'decoration-2');
});
test('Range and DecorationOptions arrays preserve original order, getter snapshots once and rich inert metadata',async()=>{
  const f=fixture(),type=f.create({}),markdown=new MarkdownString('**猫🙂**');markdown.isTrusted={enabledCommands:['sample.run']};markdown.supportHtml=true;markdown.supportThemeIcons=true;markdown.baseUri=Uri.file('/tmp/猫');
  let reads=0;const value={get range(){reads++;return new Range(1,1,2,3);},hoverMessage:['plain',markdown],renderOptions:{before:{contentText:'→',color:new ThemeColor('editor.foreground')},dark:{after:{contentText:'尾'}}}};
  f.set(type,[value]);assert.equal(reads,1);const projected=f.calls[0].args.decorations[0];assert.deepEqual(projected.range,{start:{line:1,character:1},end:{line:2,character:3}});
  assert.deepEqual(projected.hoverMessage[0],{value:'plain'});assert.deepEqual(projected.hoverMessage[1].isTrusted,{enabledCommands:['sample.run']});assert.equal(projected.hoverMessage[1].supportHtml,true);assert.equal(projected.hoverMessage[1].baseUri,Uri.file('/tmp/猫').toString());
  assert.deepEqual(projected.renderOptions.before.color,{id:'editor.foreground'});assert.equal(projected.renderOptions.dark.after.contentText,'尾');await tick();
});
test('all type render fields, theme variants and URI/string icons are copied to primitive DTOs',()=>{
  const f=fixture(),uri=Uri.file('/tmp/猫.svg'),options={isWholeLine:true,rangeBehavior:0,overviewRulerLane:OverviewRulerLane.Full,color:new ThemeColor('editor.foreground'),backgroundColor:'#123456',outline:'1px solid',outlineColor:'#fff',outlineStyle:'solid',outlineWidth:'1px',border:'1px solid',borderColor:'#eee',borderRadius:'2px',borderSpacing:'1px',borderStyle:'solid',borderWidth:'1px',fontStyle:'italic',fontWeight:'bold',textDecoration:'underline line-through',cursor:'pointer',opacity:'0.5',letterSpacing:'1px',gutterIconPath:uri,gutterIconSize:'contain',overviewRulerColor:new ThemeColor('editorWarning.foreground'),before:{contentText:'→',contentIconPath:'/tmp/left.svg',margin:'1px',width:'2px',height:'3px',backgroundColor:'#fff'},light:{color:'#000'},dark:{color:'#fff'}};
  const type=f.create(options),wire=f.messages[0].params.options;assert.equal(wire.rangeBehavior,0);assert.deepEqual(wire.color,{id:'editor.foreground'});assert.equal(wire.gutterIconPath,uri.toString());assert.equal(wire.before.contentIconPath,Uri.file('/tmp/left.svg').toString());assert.equal(wire.dark.color,'#fff');
  options.fontWeight='normal';uri.path='/changed';assert.equal(wire.fontWeight,'bold');assert.equal(uri.path,'/changed');assert.equal(wire.gutterIconPath,Uri.file('/tmp/猫.svg').toString());type.dispose();
});
test('URI and type option getters are captured once and original URI formatters are not caller-controlled',()=>{
  const f=fixture(),uri=Uri.file('/tmp/icon.svg'),reads={};for(const key of ['scheme','authority','path','query','fragment']){const value=uri[key];Object.defineProperty(uri,key,{get(){reads[key]=(reads[key]||0)+1;return value;}});}
  uri.toString=()=>{throw Error('untrusted formatter');};let colors=0;f.create({get color(){colors++;return '#f00';},gutterIconPath:uri});assert.equal(colors,1);assert.deepEqual(reads,{scheme:1,authority:1,path:1,query:1,fragment:1});assert.equal(f.messages[0].params.options.gutterIconPath,'file:///tmp/icon.svg');
});
test('source edit/Undo, type disposal and caller owner ABA during range getter reject before request',()=>{
  for(const cause of ['epoch','type','owner']){
    const f=fixture(),type=f.create({}),row={};let reads=0;Object.defineProperty(row,'range',{get(){reads++;if(cause==='epoch')f.editUndo();else if(cause==='type')type.dispose();else{f.callers.retire('a');f.module.forOwner('a');}return new Range(0,0,0,1);}});
    assert.throws(()=>f.set(type,[row]),/source changed|disposed|retired/);assert.equal(reads,1);assert.equal(f.calls.length,0);
  }
});
test('type option getter retirement cannot publish a new decoration with renewed creator scope',()=>{
  const f=fixture(),options={get color(){f.callers.retire('a');f.module.forOwner('a');return '#f00';}};assert.throws(()=>f.create(options),/retired/);assert.equal(f.messages.length,0);
  const old=f.create;assert.throws(()=>old({}),/retired/);const fresh=f.module.forOwner('a').createTextEditorDecorationType;assert.equal(fresh({}).key,'decoration-1');
});
test('cross-owner consumer B may use live A type; disposing A retires type without consumer retarget',async()=>{
  const wait=held(),f=fixture(()=>wait.promise),type=f.create({color:'#f00'});f.set(type,[new Range(0,0,0,1)],'b');assert.equal(f.calls[0].owner,'b');
  f.retireOwner('a');f.module.forOwner('a').createTextEditorDecorationType({color:'#00f'});wait.resolve({document:1,editor:'left',key:type.key});await tick();assert.equal(f.errors.length,1);assert.match(f.errors[0].error.message,/disposed|retired/);assert.throws(()=>f.set(type,[new Range(0,0,0,1)],'b'),/disposed/);
});
test('empty never-assigned set is quiet; clearing a used type sends exactly one native clearing request',async()=>{
  const f=fixture(),type=f.create({});f.set(type,[]);assert.equal(f.calls.length,0);f.set(type,[new Range(0,0,0,1)]);f.set(type,[]);f.set(type,[]);assert.equal(f.calls.length,2);assert.deepEqual(f.calls[1].args.decorations,[]);await tick();
});
test('array count, mixed malformed entries, reversed coordinates and scalar style limits are whole refusals',()=>{
  const f=fixture(),type=f.create({});for(const input of [null,Array(4097).fill(new Range(0,0,0,1)),[new Range(0,0,0,1),{range:new Range(0,0,0,1)}],[{range:{start:{line:1,character:0},end:{line:0,character:0}}}]])assert.throws(()=>f.set(type,input),/array|limit|coordinates|reversed/);
  assert.throws(()=>f.create({fontWeight:'x'.repeat(4097)}),/oversized/);assert.equal(f.calls.length,0);
});
test('aggregate escaped capture guard refuses before a late range getter or any partial setter request',()=>{
  const f=fixture(),type=f.create({}),rows=Array.from({length:32},()=>({range:new Range(0,0,0,1),hoverMessage:'\0'.repeat(65536)}));let late=0;
  rows.push({get range(){late++;return new Range(0,0,0,1);}});assert.throws(()=>f.set(type,rows),/payload limit/);assert.equal(late,0);assert.equal(f.calls.length,0);
  const options={color:'\0'.repeat(3000)};assert.throws(()=>f.create(options),/payload limit/);assert.equal(f.messages.length,1);
});
test('256 live types bound is reclaimed on dispose, monotonically retired keys never repeat',()=>{
  const f=fixture(),types=Array.from({length:256},()=>f.create({}));assert.throws(()=>f.create({}),/type limit/);types[0].dispose();assert.equal(f.create({}).key,'decoration-257');f.module.disposeOwner('a');assert.equal(f.messages.filter(value=>value.params.action==='dispose').length,257);
});
test('lifetime type bound refuses after 4096 keys even when all preceding types were disposed',()=>{
  const f=fixture();for(let i=0;i<4096;i++)f.create({}).dispose();assert.throws(()=>f.create({}),/type limit/);assert.equal(f.messages.length,8192);
});
test('native failures and wrong typed acknowledgement report handled errors; hide does not borrow new editor',async()=>{
  const f=fixture(()=>({document:1,editor:'left',key:'wrong'})),type=f.create({});f.set(type,[new Range(0,0,0,1)]);await tick();assert.equal(f.errors.length,1);assert.match(f.errors[0].error.message,/original type/);
  f.hide();assert.throws(()=>f.set(type,[new Range(0,0,0,1)]),/Retired editor/);assert.equal(f.calls.length,1);
});
test('legacy capability false refuses before option getters, notification or native calls',()=>{
  const f=fixture(null,{dependencies:{enabled:false}});let reads=0;assert.throws(()=>f.create({get color(){reads++;return '#fff';}}),/not implemented/);assert.equal(reads,0);assert.equal(f.messages.length,0);
});
test('native framing retains exact decoration operation after timeout; only original document/editor/key releases',async()=>{
  const frames=[],timers=[];let terminal=0;const queue=createNativeRequests(frame=>frames.push(frame),()=>({active:true,id:9,owner:'command.origin'}),{setTimeout(fn){timers.push(fn);return fn;},clearTimeout(){}}),lifecycle={onTerminal(){terminal++;}};
  const promise=queue.request('nativeEditorDecorations',{owner:'consumer',args:{document:1,editor:'left',key:'decoration-1'}},lifecycle),rejected=assert.rejects(promise,/outcome is unknown/);timers[0]();await rejected;assert.equal(lifecycle.retained,true);assert.equal(frames[0].params.commandOwner,'command.origin');
  for(const result of [{document:1,editor:'left'},{document:2,editor:'left',key:'decoration-1'},{document:1,editor:'right',key:'decoration-1'},{document:1,editor:'left',key:'decoration-2'},{document:1,editor:'left',key:'decoration-1',extra:true}])assert.equal(queue.settle({id:frames[0].id,result}),false);
  assert.equal(terminal,0);assert.equal(queue.settle({id:frames[0].id,result:{document:1,editor:'left',key:'decoration-1'}}),true);assert.equal(terminal,1);
});
test('actual scoped facade uses original type-owner and editor identity across shared-document panes',async()=>{
  const calls=[],notifications=[];const runtime=createApi((method,params)=>{calls.push({method,params});return Promise.resolve({document:1,editor:params.args.editor,key:params.args.key});},(method,params)=>notifications.push({method,params}),{session:3,editorDecorations:true});
  runtime.sync({generation:1,documents:[{id:1,uri:Uri.file('/tmp/a.cpp').toString(),version:1,text:'猫🙂\r\nvalue',languageId:'cpp',languageEpoch:0,isDirty:true}],editors:[{id:'left',document:1,viewColumn:1,selections:[]},{id:'right',document:1,viewColumn:2,selections:[]}],activeEditor:'left'});
  const a=runtime.forExtension('a'),b=runtime.forExtension('b'),type=a.window.createTextEditorDecorationType({color:'#f00'}),right=b.window.visibleTextEditors[1];right.setDecorations(type,[new Range(0,0,0,1)]);await tick();
  assert.equal(notifications[0].params.owner,'a');assert.equal(calls[0].params.owner,'b');assert.equal(calls[0].params.args.editor,'right');assert.equal(right.document,a.window.activeTextEditor.document);assert.equal(right.document.getText(),'猫🙂\r\nvalue');
  type.dispose();assert.throws(()=>right.setDecorations(type,[]),/disposed/);assert.equal(notifications.at(-1).params.action,'dispose');
});
test('aggregate live option bytes refuse atomically and disposal reclaims exact registry capacity',()=>{
  const f=fixture(),options={color:'x'.repeat(3800),fontStyle:'x'.repeat(3800),fontWeight:'x'.repeat(3800),textDecoration:'x'.repeat(3800)},types=[];
  let refused=false;for(let i=0;i<256;i++){try{types.push(f.create(options));}catch(error){assert.match(error.message,/type limit/);refused=true;break;}}
  assert.equal(refused,true);assert(types.length>1&&types.length<256);
  const bytes=f.messages.reduce((sum,event)=>sum+Buffer.byteLength(JSON.stringify(event.params.options)),0);assert(bytes<=1024*1024);assert(bytes+Buffer.byteLength(JSON.stringify(options))>1024*1024);
  const count=f.messages.length;types[0].dispose();const replacement=f.create(options);assert.notEqual(replacement.key,types[0].key);assert.equal(f.messages.length,count+2);
});
test('consumer owner retirement while held cannot borrow a renewed lease or dispose creator type',async()=>{
  const wait=held(),f=fixture(()=>wait.promise),type=f.create({});f.set(type,[new Range(0,0,0,1)],'b');f.callers.retire('b');f.module.forOwner('b');
  wait.resolve({document:1,editor:'left',key:type.key});await tick();assert.equal(f.errors.length,1);assert.match(f.errors[0].error.message,/retired/);
  f.set(type,[new Range(0,1,0,2)],'a');await tick();assert.equal(f.calls.length,2);assert.equal(f.errors.length,1);assert.equal(f.messages.filter(message=>message.params.action==='dispose').length,0);
});
test('owner cleanup retires every type even when one native disposal notification throws',()=>{
  let failing=true;const f=fixture(null,{notify(_method,value){if(failing&&value.action==='dispose'){failing=false;throw Error('notification refused');}}}),a=f.create({}),b=f.create({});
  f.module.disposeOwner('a');assert.throws(()=>f.set(a,[]),/disposed/);assert.throws(()=>f.set(b,[]),/disposed/);assert.equal(f.errors.length,1);assert.equal(f.create({}).key,'decoration-3');
});
