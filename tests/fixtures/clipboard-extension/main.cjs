"use strict";
const assert=require('node:assert/strict'),v=require('vscode');
const seed='seed 猫🙂\r\n',text='λ🙂 clipboard\r\nsecond\r\n';
exports.activate=context=>{
 const register=(name,callback)=>context.subscriptions.push(v.commands.registerCommand('fixture.clipboard.'+name,callback));
 register('roundtrip',async()=>{
  const document=v.window.activeTextEditor?.document,before=document?.getText(),version=document?.version;
  assert(Object.isFrozen(v.env.clipboard));assert.equal(await v.env.clipboard.readText(),seed);
  await assert.rejects(v.env.clipboard.writeText('\ud800'),/UTF-16/);
  assert.equal(await v.env.clipboard.writeText(text),undefined);assert.equal(await v.env.clipboard.readText(),text);
  if(document){assert.equal(document.getText(),before);assert.equal(document.version,version);}
  return v.window.showInformationMessage('Clipboard roundtrip complete');
 });
 register('unavailable',async()=>{
  await assert.rejects(v.env.clipboard.readText(),/clipboard unavailable/);
  await assert.rejects(v.env.clipboard.writeText(text),/clipboard unavailable/);
  return v.window.showInformationMessage('Clipboard backend unavailable');
 });
 register('retired',async()=>{
  await assert.rejects(v.env.clipboard.writeText(text),/clipboard caller retired/);
  return v.window.showInformationMessage('Original clipboard caller retired');
 });
 register('parallel',async()=>{
  const requests=Array.from({length:8},()=>v.env.clipboard.readText());
  await assert.rejects(v.env.clipboard.writeText('forbidden ninth'),/queue limit/);
  assert.deepEqual(await Promise.all(requests),Array(8).fill(seed));
  return v.window.showInformationMessage('Parallel clipboard slots retained');
 });
};
