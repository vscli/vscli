'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createEditorTabs}=require('./editor-tabs.cjs');
const {createCallerContext}=require('./caller-context.cjs');
const {Uri}=require('./api-types.cjs');
const inputs=require('./tab-inputs.cjs');
const {createApi}=require('./api.cjs');
const {createNativeRequests}=require('./native-requests.cjs');
function held(){let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};}
function tab(id,name=id,patch={}){return{id,label:name,isActive:true,isDirty:false,isPinned:true,isPreview:false,input:{kind:'text',uri:Uri.file(`/tmp/${name}.cpp`).toString()},...patch};}
function model(){return{groups:[{id:'g1',viewColumn:1,isActive:true,tabs:[tab('a'),tab('b','b',{isActive:false,isPinned:false,isPreview:true})]},
  {id:'g2',viewColumn:2,isActive:false,tabs:[tab('shared','a')]}]};}
function fixture(handler,enabled=true){
  const callers=createCallerContext(()=>{}),calls=[];
  const module=createEditorTabs({enabled,captureCaller:owner=>callers.bind(owner,()=>{}),bindEvent:callers.bindEvent,
    call(owner,method,args){calls.push({owner,method,args});return handler?handler(args):Promise.resolve({closed:false,kind:args.kind,ids:args.ids.slice()});}});
  const api=module.forOwner('owner');let current=model();
  function publish(value=current,fire=true){current=value;const callback=module.publish(module.prepare(value));if(fire)callback();return callback;}
  publish();return{module,api,calls,callers,publish,get current(){return current;},retire(){callers.retire('owner');module.disposeOwner('owner');module.forOwner('owner');}};
}

test('frozen shared tab/group objects and getter arrays retain live identities and separate same-resource memberships',()=>{
  const f=fixture(),groups=f.api.all,group=groups[0],a=group.tabs[0],shared=groups[1].tabs[0];
  assert(Object.isFrozen(f.api));assert(Object.isFrozen(groups));assert(Object.isFrozen(group));assert(Object.isFrozen(group.tabs));assert(Object.isFrozen(a));
  assert.notEqual(group.tabs,group.tabs);assert.notEqual(f.api.all,f.api.all);assert.equal(a.group,group);assert.equal(group.activeTab,a);assert.equal(f.api.activeTabGroup,group);
  assert(a.input instanceof inputs.TabInputText);assert.equal(a.input.uri.toString(),shared.input.uri.toString());assert.notEqual(a,shared);
  assert.equal(f.module.forOwner('other').all[0],group);assert.equal(f.module.forOwner('owner'),f.api);
  f.publish();assert.equal(f.api.all[0],group);assert.equal(group.tabs[0],a);
  assert.throws(()=>{a.label='forged';},TypeError);assert.throws(()=>group.tabs.push(shared),TypeError);
});
test('all mirrors are visible before deferred events, frozen events fire once and quiet snapshots emit nothing',()=>{
  const f=fixture(),events=[],old=f.api.all[0].tabs[0];
  f.api.onDidChangeTabGroups(event=>{events.push(['groups',event]);assert.equal(f.api.activeTabGroup.viewColumn,2);assert.equal(f.api.all[0].tabs[0].isDirty,true);});
  f.api.onDidChangeTabs(event=>{events.push(['tabs',event]);assert.equal(f.api.activeTabGroup.viewColumn,2);assert.equal(event.changed[0],old);});
  f.current.groups[0].isActive=false;f.current.groups[1].isActive=true;f.current.groups[0].tabs[0].isDirty=true;
  const fire=f.publish(f.current,false);assert.equal(events.length,0);assert.equal(old.isDirty,true);fire();fire();
  assert.deepEqual(events.map(([kind])=>kind),['groups','tabs']);
  for(const [,event]of events){assert(Object.isFrozen(event));for(const key of ['opened','closed','changed'])assert.equal(Object.isFrozen(event[key]),false);}
  f.publish();assert.equal(events.length,2);
});
test('native snapshots are detached from input DTO and input mutations never alter private close authority',async()=>{
  const f=fixture(),a=f.api.all[0].tabs[0];
  f.current.groups[0].tabs[0].label='unpublished';f.current.groups[0].id='forged';assert.equal(a.label,'a');
  a.input.uri=Uri.file('/tmp/forged.cpp');await f.api.close(a,true);
  assert.deepEqual(f.calls,[{owner:'owner',method:'nativeTabsClose',args:{kind:'tabs',ids:['a'],preserveFocus:true}}]);
});
test('all stable input kinds decode to actual shared value constructors',()=>{
  const f=fixture(),uri=Uri.file('/tmp/猫').toString();
  const cases=[['text',{uri},inputs.TabInputText],['textDiff',{original:uri,modified:uri},inputs.TabInputTextDiff],
    ['custom',{uri,viewType:'custom'},inputs.TabInputCustom],['webview',{viewType:'web'},inputs.TabInputWebview],
    ['notebook',{uri,notebookType:'book'},inputs.TabInputNotebook],['notebookDiff',{original:uri,modified:uri,notebookType:'book'},inputs.TabInputNotebookDiff],['terminal',{},inputs.TabInputTerminal]];
  for(const [kind,fields,Class]of cases){const value=model();value.groups[0].tabs[0].input={kind,...fields};f.publish(value);assert(f.api.all[0].tabs[0].input instanceof Class);}
});
test('closed snapshots remain readable but cannot acquire a replacement same ID or new group membership',async()=>{
  const f=fixture(),old=f.api.all[0].tabs[0],oldGroup=old.group;
  const next=model();next.groups[0].tabs.shift();next.groups[0].tabs[0].isActive=true;f.publish(next);
  assert.equal(old.label,'a');assert.equal(old.group,oldGroup);assert.equal(old.isActive,false);await assert.rejects(f.api.close(old),/retired/);
  f.publish(model());assert.notEqual(f.api.all[0].tabs[0],old);await assert.rejects(f.api.close(old),/retired/);
  const moved=model();moved.groups[0].tabs.shift();moved.groups[0].tabs[0].isActive=true;moved.groups[1].tabs=[tab('a')];f.publish(moved);
  assert.notEqual(f.api.all[1].tabs[0],old);assert.equal(f.calls.length,0);
});
test('close accepts tab/group homogeneous arrays and duplicates in exact input order; [] is true without RPC',async()=>{
  const f=fixture(),[g1,g2]=f.api.all,[a,b]=g1.tabs;
  assert.equal(await f.api.close([]),true);assert.equal(f.calls.length,0);
  assert.equal(await f.api.close([b,a,b]),false);assert.deepEqual(f.calls[0].args,{kind:'tabs',ids:['b','a','b'],preserveFocus:false});
  await f.api.close([g2,g1],true);assert.deepEqual(f.calls[1].args,{kind:'groups',ids:['g2','g1'],preserveFocus:true});
  for(const value of [null,{}, {label:a.label,input:a.input,group:g1},[a,g1]])await assert.rejects(f.api.close(value),/Invalid|mixed/);
  await assert.rejects(f.api.close(a,'yes'),/boolean/);assert.equal(f.calls.length,2);
});
test('array getter new replacement and caller ABA cannot retarget captured cohort',async()=>{
  for(const cause of ['replacement','owner']){
    const f=fixture(),a=f.api.all[0].tabs[0],values=[a];let reads=0;
    Object.defineProperty(values,0,{get(){reads++;
      if(cause==='owner')f.retire();
      else{const next=model();next.groups[0].tabs.shift();next.groups[0].tabs[0].isActive=true;f.publish(next);f.publish(model());}
      return a;
    }});
    await assert.rejects(f.api.close(values),/changed|retired/);assert.equal(reads,1);assert.equal(f.calls.length,0);
  }
});
test('ordinary dirty/focus updates and tab ordering do not revoke an unchanged tab membership',async()=>{
  const f=fixture(),a=f.api.all[0].tabs[0],values=[a];
  Object.defineProperty(values,0,{get(){const next=model();next.groups[0].tabs[0].isDirty=true;next.groups[0].isActive=false;next.groups[1].isActive=true;next.groups[0].tabs.reverse();f.publish(next);return a;}});
  assert.equal(await f.api.close(values),false);assert.deepEqual(f.calls[0].args.ids,['a']);
  const group=f.api.all[0],batch=[group];
  Object.defineProperty(batch,0,{get(){f.publish(model());return group;}});
  await assert.rejects(f.api.close(batch),/changed/);assert.equal(f.calls.length,1);
});
test('close bounds precede element getters, forged object getters are never used as authority',async()=>{
  const f=fixture(),values=new Array(513);let reads=0;Object.defineProperty(values,0,{get(){reads++;throw Error('late getter');}});
  await assert.rejects(f.api.close(values),/512/);assert.equal(reads,0);
  await assert.rejects(f.api.close({get tabs(){reads++;throw Error('forged getter');}}),/Invalid/);assert.equal(reads,0);assert.equal(f.calls.length,0);
});
test('actual close receipt awaits native removal/events; expected target retirement does not reject true',async()=>{
  const wait=held(),f=fixture(()=>wait.promise),a=f.api.all[0].tabs[0],events=[];let completed=false;
  f.api.onDidChangeTabs(event=>events.push(event));const promise=f.api.close(a).then(value=>{completed=true;return value;});
  await Promise.resolve();assert.equal(completed,false);assert.equal(a.isActive,true);assert.equal(events.length,0);
  const next=model();next.groups[0].tabs.shift();next.groups[0].tabs[0].isActive=true;f.publish(next);
  assert.equal(events[0].closed[0],a);assert.equal(f.api.all[0].tabs[0].label,'b');
  wait.resolve({closed:true,kind:'tabs',ids:['a']});assert.equal(await promise,true);
});
test('empty native group has no active tab and can remain after an acknowledged group close',async()=>{
  const wait=held(),f=fixture(()=>wait.promise),group=f.api.all[0],promise=f.api.close(group);
  f.publish({groups:[{id:'g1',viewColumn:1,isActive:true,tabs:[]}]});assert.equal(f.api.activeTabGroup,group);assert.equal(group.activeTab,undefined);assert.deepEqual(group.tabs,[]);
  wait.resolve({closed:true,kind:'groups',ids:['g1']});assert.equal(await promise,true);
});
test('wrong, extra, missing and reordered close ACKs refuse while native false remains false',async()=>{
  for(const reply of [null,true,{closed:true},{closed:1,kind:'tabs',ids:['a','b']},{closed:true,kind:'groups',ids:['a','b']},
    {closed:true,kind:'tabs',ids:['b','a']},{closed:true,kind:'tabs',ids:['a','b'],extra:1}]){
    const f=fixture(()=>Promise.resolve(reply));await assert.rejects(f.api.close(f.api.all[0].tabs),/acknowledgement/);
  }
  const f=fixture();assert.equal(await f.api.close(f.api.all[0].tabs),false);
});
test('held caller retirement cannot accept terminal or reuse original facade; listeners are inert after retirement',async()=>{
  const wait=held(),f=fixture(()=>wait.promise);let events=0;const subscription=f.api.onDidChangeTabs(()=>events++);
  const promise=f.api.close(f.api.all[0].tabs[0]),rejected=assert.rejects(promise,/retired/);f.retire();
  f.current.groups[0].tabs[0].isDirty=true;f.publish();assert.equal(events,0);
  wait.resolve({closed:false,kind:'tabs',ids:['a']});await rejected;await assert.rejects(f.api.close([]),/retired/);subscription.dispose();
});
test('invalid inventories and foreign/stale plans cannot partially publish state or revive authority',()=>{
  const f=fixture(),group=f.api.all[0],a=group.tabs[0];
  const bad=[{groups:[]},model(),model(),model(),model()];
  bad[1].groups[0].tabs[1].id='a';bad[2].groups[0].tabs[0].input.uri='not a URI';bad[3].groups[0].tabs[0].isDirty=1;bad[4].groups[0].tabs.push(tab('extra'));
  for(const value of bad){assert.throws(()=>f.module.prepare(value));assert.equal(f.api.all[0],group);assert.equal(group.tabs[0],a);assert.equal(a.isDirty,false);}
  const plan=f.module.prepare(model());f.publish();assert.throws(()=>f.module.publish(plan),/retired/);
  const other=fixture();assert.throws(()=>other.module.publish(f.module.prepare(model())),/retired/);
});
test('native and legacy-uninitialized tab inventories never fabricate getters or successful close',async()=>{
  const f=fixture(undefined,false);assert.throws(()=>f.api.all,/not initialized/);await assert.rejects(f.api.close([]),/not initialized/);
  const callers=createCallerContext(()=>{}),module=createEditorTabs({enabled:true,captureCaller:owner=>callers.bind(owner,()=>{}),bindEvent:callers.bindEvent,call(){throw Error('no call');}}),api=module.forOwner('new');
  assert.throws(()=>api.activeTabGroup,/not initialized/);await assert.rejects(api.close([]),/not initialized/);
});
test('closed active groups follow global active identity while their last tabs retain parent-derived presentation',()=>{
  const f=fixture(),old=f.api.all[0],a=old.tabs[0];
  f.publish({groups:[{...model().groups[1],viewColumn:1,isActive:true}]});
  assert.equal(old.isActive,false);assert.equal(f.api.activeTabGroup.viewColumn,1);
  // A removed group still retains its own last active tab ID upstream.
  assert.equal(old.activeTab,a);assert.equal(a.isActive,true);
});
test('mutable event arrays cannot mutate private group/tab getter arrays or close targets',async()=>{
  const f=fixture();let payload;
  f.api.onDidChangeTabs(value=>{payload=value;value.changed.push({forged:true});});
  f.current.groups[0].tabs[0].isDirty=true;f.publish();assert.equal(payload.changed.length,2);
  assert.equal(f.api.all[0].tabs.length,2);assert.equal(f.api.all[0].tabs[0].label,'a');
  await assert.rejects(f.api.close(payload.changed),/Invalid/);assert.equal(f.calls.length,0);
});
test('maximum bounded inventory is admitted; count/string/aggregate metadata overflow refuses atomically',()=>{
  const f=fixture(),maximum={groups:Array.from({length:4},(_,g)=>({id:`g${g}`,viewColumn:g+1,isActive:g===0,
    tabs:Array.from({length:128},(_,t)=>tab(`tab-${g}-${t}`,`name-${g}-${t}`,{isActive:t===0}))}))};
  f.publish(maximum);assert.equal(f.api.all.reduce((sum,group)=>sum+group.tabs.length,0),512);
  const original=f.api.all[0],a=original.tabs[0];
  const oversized=structuredClone(maximum);oversized.groups[0].tabs.push(tab('extra'));
  assert.throws(()=>f.module.prepare(oversized),/count/);
  const labels=structuredClone(maximum);labels.groups[0].tabs[0].label='x'.repeat(1025);assert.throws(()=>f.module.prepare(labels),/label/);
  const bytes=structuredClone(maximum);for(const group of bytes.groups)for(const item of group.tabs){item.label='x'.repeat(1024);item.input.uri='sample:/'+'x'.repeat(4088);}
  assert.throws(()=>f.module.prepare(bytes),/metadata budget/);assert.equal(f.api.all[0],original);assert.equal(original.tabs[0],a);
});
test('actual scoped facade shares identities and publishes editor/document/tab state before close ACK and events',async()=>{
  const wait=held(),calls=[];const runtime=createApi((method,params)=>{calls.push({method,params});return wait.promise;},()=>{},{session:7,editorTabs:true});
  const doc={id:1,uri:Uri.file('/tmp/a.cpp').toString(),text:'猫🙂\r\nsecond',version:1,eol:2,languageId:'cpp',isDirty:false};
  const sync=(generation,tabs,editors,activeEditor)=>runtime.sync({generation,documents:[doc],tabs,editors,activeEditor});
  sync(1,model(),[{id:'left',document:1,viewColumn:1,selections:[]}],'left');
  const a=runtime.forExtension('a'),b=runtime.forExtension('b'),target=a.window.tabGroups.all[0].tabs[0],events=[];
  assert.equal(b.window.tabGroups.all[0].tabs[0],target);assert.equal(a.TabInputText,inputs.TabInputText);assert(target.input instanceof a.TabInputText);
  a.window.tabGroups.onDidChangeTabs(event=>{events.push(event);assert.equal(a.window.activeTextEditor,undefined);assert.equal(a.workspace.textDocuments[0].getText(),doc.text);assert.equal(a.window.tabGroups.all[0].tabs.length,0);});
  const pending=b.window.tabGroups.close(target,true);
  assert.equal(calls[0].method,'nativeTabsClose');assert.equal(calls[0].params.owner,'b');assert.deepEqual(calls[0].params.args,{kind:'tabs',ids:['a'],preserveFocus:true});
  sync(2,{groups:[{id:'g1',viewColumn:1,isActive:true,tabs:[]}]},[],null);
  assert.equal(events[0].closed[0],target);wait.resolve({closed:true,kind:'tabs',ids:['a']});assert.equal(await pending,true);
  runtime.disposeOwner('a');runtime.forExtension('a');assert.throws(()=>a.window.tabGroups.onDidChangeTabs(()=>{}),/retired/);
});
test('actual tab framing retains timed-out local capacity until exact kind and ordered IDs terminal',async()=>{
  const frames=[],timers=[];const queue=createNativeRequests(frame=>frames.push(frame),()=>({active:true,id:41,owner:'original.command'}),{setTimeout(callback,delay){assert.equal(delay,360000);timers.push(callback);return callback;},clearTimeout(){}});
  const runtime=createApi(queue.request,()=>{},{session:9,editorTabs:true});
  runtime.sync({generation:1,documents:[],editors:[],activeEditor:null,tabs:model()});const api=runtime.forExtension('consumer'),target=api.window.tabGroups.all[0].tabs;
  const failures=[];
  try{
    for(let index=0;index<8;index++)failures.push(assert.rejects(api.window.tabGroups.close(target),/Native tab close timed out; outcome is unknown/));
    assert.equal(frames.length,8);for(const callback of timers)callback();await Promise.all(failures);
    assert.equal(frames[0].params.commandOwner,'original.command');assert.equal(frames[0].params.owner,'consumer');
    const wrong=[{closed:true,kind:'tabs',ids:['b','a']},{closed:true,kind:'groups',ids:['a','b']},{closed:'yes',kind:'tabs',ids:['a','b']},
      {closed:true,kind:'tabs'},{closed:true,kind:'tabs',ids:['a','b'],extra:1}];
    for(const result of wrong)assert.equal(queue.settle({id:frames[0].id,result}),false);
    await assert.rejects(api.window.tabGroups.close(target),/queue limit/);assert.equal(frames.length,8);
    assert.equal(queue.settle({id:frames[0].id,result:{closed:false,kind:'tabs',ids:['a','b']}}),true);
    const fresh=api.window.tabGroups.close(target);assert.equal(frames.length,9);
    assert.equal(queue.settle({id:frames[8].id,result:{closed:false,kind:'tabs',ids:['a','b']}}),true);assert.equal(await fresh,false);
    assert.equal(api.window.tabGroups.all[0].tabs.length,2);
  }finally{queue.shutdown();await Promise.allSettled(failures);}
});

test('native atomic publication cannot invoke extension-replaced URI parser or tab constructors',()=>{
  const f=fixture(),originalParse=Uri.parse,originalText=inputs.TabInputText;
  let callbacks=0;
  try{
    Uri.parse=()=>{callbacks++;throw Error('extension parser entered native publication');};
    inputs.TabInputText=function(){callbacks++;throw Error('extension constructor entered native publication');};
    const plan=f.module.prepare(model());f.module.publish(plan)();
    assert.equal(callbacks,0);
    assert(f.api.all[0].tabs[0].input instanceof originalText);
    assert.equal(f.api.all[0].tabs[0].input.uri.toString(),tab('a').input.uri);
  }finally{Uri.parse=originalParse;inputs.TabInputText=originalText;}
});
