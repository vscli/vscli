'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { createPrompts } = require('./prompts.cjs');

test('native prompts retain owner/session and return the original selected item', async () => {
  const item = { label: 'Unicode 😀', description: 'description', detail: 'detail', data: 7 };
  const calls = [];
  const api = createPrompts(async (method, params) => { calls.push([method, params]); return { value: 1 }; }, 9, 'test.owner', { pending: 0, bytes: 0 });
  assert.equal(await api.showQuickPick(Promise.resolve(['string', item]), { matchOnDetail: true }), item);
  assert.equal(calls[0][0], 'prompt');
  assert.equal(calls[0][1].session, 9);
  assert.equal(calls[0][1].owner, 'test.owner');
  assert.deepEqual(calls[0][1].items[1], { label: item.label, description: 'description', detail: 'detail' });
});

test('cancel returns undefined and empty accepted input stays empty', async () => {
  const api = createPrompts(async () => ({ value: null }), 1, 'test.owner', { pending: 0, bytes: 0 });
  assert.equal(await api.showQuickPick(['one']), undefined);
  assert.equal(await api.showInputBox({ value: 'before' }), undefined);
  const input = createPrompts(async () => ({ value: '' }), 1, 'test.owner', { pending: 0, bytes: 0 });
  assert.equal(await input.showInputBox(), '');
});

test('unsupported variants and budgets reject before showing native UI', async () => {
  let calls = 0;
  const budget = { pending: 0, bytes: 0 };
  const api = createPrompts(async () => { calls++; return { value: null }; }, 1, 'test.owner', budget);
  for (const options of [{ password: 'yes' }, { validateInput() {} }, { valueSelection: [0, 1] }]) await assert.rejects(api.showInputBox(options));
  for (const options of [{ canPickMany: true }, { onDidSelectItem() {} }, { ignoreFocusOut: true }]) await assert.rejects(api.showQuickPick(['one'], options));
  await assert.rejects(api.showQuickPick(['one'], {}, {}));
  await assert.rejects(api.showQuickPick(Array(129).fill('one')));
  await assert.rejects(api.showQuickPick(['x'.repeat(4097)]));
  await assert.rejects(api.showQuickPick(Array(128).fill('x'.repeat(1024))));
  assert.equal(calls, 0);
  assert.deepEqual(budget, { pending: 0, bytes: 0 });
});

test('group queue and text budgets are shared and released after completion/errors', async () => {
  const pending = [];
  const budget = { pending: 0, bytes: 0 };
  const request = () => new Promise(resolve => pending.push(resolve));
  const a = createPrompts(request, 1, 'a', budget), z = createPrompts(request, 1, 'z', budget);
  const calls = Array.from({ length: 8 }, (_, i) => (i % 2 ? a : z).showInputBox());
  await assert.rejects(a.showInputBox(), /queue limit/);
  for (const resolve of pending) resolve({ value: null });
  await Promise.all(calls);
  assert.deepEqual(budget, { pending: 0, bytes: 0 });
  const large = a.showQuickPick(Array(32).fill('x'.repeat(1024)));
  await Promise.resolve();
  const before = budget.bytes;
  await assert.rejects(z.showQuickPick(Array(32).fill('x'.repeat(1024))), /budget/);
  assert.equal(budget.bytes, before);
  pending.at(-1)({ value: null });
  await large;
  assert.deepEqual(budget, { pending: 0, bytes: 0 });
});

test('password intent is captured once, strictly typed and returns exact Unicode or cancellation', async () => {
  const calls = [], budget = {pending:0,bytes:0};
  const value = 'synthetic猫e\u0301👩‍💻';
  const api = createPrompts(async (method,params)=>{calls.push(params);return {value};},7,'owner',budget);
  let reads = 0;
  assert.equal(await api.showInputBox({value,get password(){assert.equal(++reads,1);return true;}}),value);
  assert.equal(calls[0].password,true);assert.equal(calls[0].value,value);
  assert.equal(await api.showInputBox({password:false}),value);assert.equal(calls[1].password,false);
  for(const password of [null,1,'true',{}])await assert.rejects(api.showInputBox({password}),/password/);
  await assert.rejects(api.showQuickPick(['one'],{password:true}),/not implemented/);
  assert.equal(calls.length,2);assert.deepEqual(budget,{pending:0,bytes:0});
});

test('cached prompt methods and held item/reply continuations retain their original owner lease',async()=>{
  const {createApi}=require('./api.cjs');let resolve,requests=0;
  const runtime=createApi(()=>{requests++;return new Promise(done=>{resolve=done;});},()=>{},{session:1});
  const old=runtime.forExtension('owner').window;
  let items;const pick=old.showQuickPick(new Promise(done=>{items=done;}));
  runtime.disposeOwner('owner');const fresh=runtime.forExtension('owner').window;
  items(['late']);await assert.rejects(pick,/retired/);assert.equal(requests,0);
  await assert.rejects(old.showInputBox({password:true}),/retired/);
  const input=fresh.showInputBox({password:true});assert.equal(requests,1);
  runtime.disposeOwner('owner');runtime.forExtension('owner');resolve({value:'late'});
  await assert.rejects(input,/retired/);
  const live=runtime.forExtension('owner').window.showInputBox({password:true});resolve({value:'current'});
  assert.equal(await live,'current');assert.equal(requests,2);
});

test('an option getter cannot transfer a prompt to a renewed owner before native dispatch',async()=>{
  const {createApi}=require('./api.cjs');let requests=0;
  const runtime=createApi(async()=>{requests++;return {value:null};},()=>{},{session:1});
  const old=runtime.forExtension('owner').window;
  await assert.rejects(old.showInputBox({get password(){runtime.disposeOwner('owner');runtime.forExtension('owner');return true;}}),/retired/);
  assert.equal(requests,0);
  assert.equal(await runtime.forExtension('owner').window.showInputBox({password:true}),undefined);
  assert.equal(requests,1);
});
