'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const {TextDocument,Range}=require('./api-types.cjs');
const {CompletionItem,SignatureHelp,SignatureInformation}=require('./provider-types.cjs');
const {CodeAction}=require('./code-action-types.cjs');
const {createProviders}=require('./providers.cjs');
function fixture(){
 const document=new TextDocument({id:1,uri:'file:///tmp/state.cpp',text:'猫🙂 original\r\n',version:3,languageId:'cpp',isDirty:true});
 let epoch=0;
 const providers=createProviders({session:7,document:id=>id===1?document:undefined,languageEpoch:()=>epoch,
  timeoutMs:5000,notify:()=>{},track:(_,disposable)=>disposable});
 const request=id=>({session:7,owner:'test.extension',provider:id,document:1,version:3,position:{line:0,character:3}});
 function change(language){epoch++;document._update({...document._snapshot,languageId:language});providers.documentChanged();}
 return{document,providers,request,change};
}
function deferred(){let resolve;const promise=new Promise(done=>resolve=done);return{promise,resolve};}
test('metadata-only language changes cancel held callbacks without releasing their actual slots',async()=>{
 const f=fixture(),held=Array.from({length:8},deferred);let entered=0;
 f.providers.forOwner('test.extension').registerHoverProvider('*',{provideHover(){return held[entered++].promise;}});
 const id=f.providers.snapshot()[0].id;
 const calls=held.map((_,index)=>f.providers.provide({...f.request(id),request:index+1}));
 const failures=Promise.allSettled(calls);
 await Promise.resolve();await Promise.resolve();assert.equal(entered,8);
 f.change('python');f.change('cpp');
 const outcomes=await failures;assert(outcomes.every(value=>value.status==='rejected'));
 assert.equal(f.providers.pendingCount(),8,'canceled promises do not release actual work');
 await assert.rejects(f.providers.provide({...f.request(id),request:9}),/invocation limit/);
 // Public metadata cannot reauthorize an old private lifetime.
 f.document._snapshot.languageEpoch=0;
 for(const item of held)item.resolve(null);
 await new Promise(resolve=>setImmediate(resolve));
 assert.equal(f.providers.pendingCount(),0);assert.equal(f.document.version,3);assert(f.document.isDirty);
 assert.equal(f.document.getText(),'猫🙂 original\r\n');
});
test('metadata ABA retires completion resolve handles while preserving document and version',async()=>{
 const f=fixture();let resolves=0;
 f.providers.forOwner('test.extension').registerCompletionItemProvider('*',{
  provideCompletionItems:()=>[new CompletionItem('item')],resolveCompletionItem:item=>{resolves++;return item;}});
 const id=f.providers.snapshot()[0].id;
 const list=await f.providers.provide({...f.request(id),request:10});assert.equal(f.providers.retainedCompletionCount(),1);
 f.change('python');f.change('cpp');assert.equal(f.providers.retainedCompletionCount(),0);
 await assert.rejects(f.providers.resolveCompletion({...f.request(id),request:11,origin:10,handle:list.items[0]._vscliCompletionHandle}),/Stale/);
 assert.equal(resolves,0);assert.equal(f.document.version,3);
});
test('language lifetime retires signature help and code-action caches including workspace targets',async()=>{
 const f=fixture(),owner=f.providers.forOwner('test.extension');
 owner.registerSignatureHelpProvider('*',{provideSignatureHelp(){const help=new SignatureHelp();help.signatures=[new SignatureInformation('f(x)')];return help;}},'(');
 const signature=f.providers.snapshot().find(value=>value.type==='signature').id;
 await f.providers.provide({...f.request(signature),request:20,signatureRequest:true,signatureContext:{triggerKind:1,isRetrigger:false}});
 assert.equal(f.providers.retainedSignatureCount(),1);
 let resolves=0;
 owner.registerCodeActionsProvider('*',{provideCodeActions:()=>[new CodeAction('Fix')],resolveCodeAction:item=>{resolves++;return item;}});
 // Registering a provider changes the registry; reacquire current signature.
 await f.providers.provide({...f.request(signature),request:21,signatureRequest:true,signatureContext:{triggerKind:1,isRetrigger:false}});
 const action=f.providers.snapshot().find(value=>value.type==='codeaction').id;
 const result=await f.providers.provide({...f.request(action),request:22,range:new Range(0,0,0,3),selection:{anchor:{line:0,character:0},active:{line:0,character:3}},actionContext:{triggerKind:1},workspace:[{document:1,version:3,uri:f.document.uri.toString()}]});
 assert.equal(f.providers.retainedActionCount(),1);assert.equal(f.providers.retainedSignatureCount(),1);
 f.change('python');f.change('cpp');
 assert.equal(f.providers.retainedActionCount(),0);assert.equal(f.providers.retainedSignatureCount(),0);
 await assert.rejects(f.providers.resolveAction({...f.request(action),request:23,origin:22,handle:result[0]._vscliCodeActionHandle}),/Stale|stale/);
 assert.equal(resolves,0);assert.equal(f.document.version,3);
});
