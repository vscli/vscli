'use strict';
const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 let active,last;
 const register=(id,callback)=>context.subscriptions.push(v.commands.registerCommand('progress.'+id,callback));
 register('begin',async()=>{
  assert(!active);
  const expected={value:'猫'};expected.self=expected;
  const result=await v.window.withProgress({location:v.ProgressLocation.Window,title:'Profile',cancellable:true},(progress,token)=>{
   assert.equal(token.isCancellationRequested,false);token.onCancellationRequested(()=>assert.fail('Window cancellation')).dispose();
   last=progress;return new Promise(resolve=>{active={progress,resolve:()=>resolve(expected)};});
  });
  assert.strictEqual(result,expected);assert.strictEqual(result.self,result);active=undefined;
  last.report({get message(){assert.fail('Completed report read user data');}});
  return v.window.showInformationMessage('Progress value preserved');
 });
 register('step',()=>{assert(active);active.progress.report({message:'phase 猫',increment:50});});
 register('finish',()=>{assert(active);active.resolve();});
 register('fail',async()=>{
  const failure=new Error('original task failure');
  try {await v.window.withProgress({location:v.ProgressLocation.Window,title:'Failure'},async()=>{throw failure;});assert.fail('Task rejection lost');}
  catch(error){assert.strictEqual(error,failure);}
  return v.window.showInformationMessage('Progress failure preserved');
 });
};
