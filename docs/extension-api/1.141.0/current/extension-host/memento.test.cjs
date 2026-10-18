'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { AsyncLocalStorage } = require('node:async_hooks');
const { createMementos } = require('./memento.cjs');
test('Memento patches reflect pending values, retain later overlays, and preserve each origin', async () => {
  const calls = [], origin = new AsyncLocalStorage();
  const storage = createMementos((method, params) => new Promise((resolve,reject)=>calls.push({method,params,resolve,reject,origin:origin.getStore()})), 1,
    { 'test.one': {global:{original:1}, workspace:{local:true}} });
  const one = storage.forOwner('test.one'), two = storage.forOwner('test.two');
  const a = origin.run('A',()=>one.globalState.update('__proto__',null));
  const b = origin.run('B',()=>one.globalState.update('later',2));
  assert.equal(one.globalState.get('__proto__'), null); assert.equal(one.globalState.get('later'),2);
  assert.equal(calls[0].origin,'A'); assert.equal(calls[1].origin,'B');
  calls[0].resolve({values:JSON.parse('{"original":1,"__proto__":null}')}); await a;
  assert.equal(one.globalState.get('later'),2);
  calls[1].reject(new Error('disk full')); await assert.rejects(b,/disk full/);
  assert.equal(one.globalState.get('later','fallback'),'fallback');
  assert.equal(one.workspaceState.get('local'),true); assert.equal(two.globalState.get('original'),undefined);
  const remove = one.globalState.update('__proto__',undefined);
  assert(!one.globalState.keys().includes('__proto__'));
  calls[2].resolve({values:{original:1}}); await remove;
  assert.throws(()=>one.globalState.setKeysForSync(['original']),/not implemented/);
});
test('Memento input and queue budgets reject before native storage', async () => {
  const calls = [];
  const state = createMementos((method,params)=>new Promise((resolve,reject)=>calls.push({params,resolve,reject})),1).forOwner('test.one').globalState;
  await assert.rejects(state.update('huge','x'.repeat(65536)),/64 KiB/);
  await assert.rejects(state.update('bad',()=>{}),/JSON/);
  const cycle = {}; cycle.self=cycle;
  await assert.rejects(state.update('cycle',cycle),/nesting/);
  assert.equal(calls.length,0);
  const pending = Array.from({length:8},(_,i)=>state.update(String(i),i));
  const settling = Promise.allSettled(pending);
  await assert.rejects(state.update('ninth',9),/queue/);
  for(const call of calls) call.reject(new Error('cancel'));
  await settling; assert.deepEqual(state.keys(),[]);
});
test('dynamic owner state merge seeds new packages without overwriting live Mementos', () => {
  const store = createMementos(()=>{},1,{'test.one':{global:{old:1}}});
  const first = store.forOwner('test.one');
  store.merge({'test.one':{global:{old:99}},'test.two':{global:{new:2},workspace:{saved:true}}});
  assert.equal(first.globalState.get('old'),1);
  assert.equal(store.forOwner('test.two').globalState.get('new'),2);
  assert.equal(store.forOwner('test.two').workspaceState.get('saved'),true);
});
