'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const {createApi} = require('./api.cjs');
const {Uri, Position, Range, Selection} = require('./api-types.cjs');

function held() {
  let resolve;
  const promise = new Promise(value => { resolve = value; });
  return {promise, resolve};
}

function fixture(editorEndOfLineEdits = true) {
  const receipt = held(), calls = [];
  const document = {id: 17, uri: Uri.file('/tmp/marker-semantics.cpp').toString(),
    text: '猫🙂alpha\r\nsecond\r\n', version: 7, languageId: 'cpp', languageEpoch: 2,
    eol: 2, isDirty: true};
  const runtime = createApi((method, params) => {
    calls.push({method, params});
    return receipt.promise;
  }, () => {}, {session: 31, editorEndOfLineEdits});
  let generation = 0;
  const sync = () => runtime.sync({generation: ++generation, documents: [document],
    editors: [{id: 'original-pane', document: 17, viewColumn: 1,
      selections: [{anchor: {line: 0, character: 3}, active: {line: 0, character: 3}}]}],
    activeEditor: 'original-pane', active: 17});
  sync();
  const api = runtime.forExtension('marker.owner');
  return {runtime, document, sync, calls, receipt, editor: api.window.activeTextEditor};
}

test('public edit insert/delete force markers while replace omits flag, awaiting original native ACK', async () => {
  const f = fixture(), editor = f.editor;
  // Public metadata cannot supply the target or version used by the edit builder.
  editor.document._snapshot = {id: 99, version: 100};
  let settled = false;
  const completion = editor.edit(builder => {
    builder.insert(new Position(0, 8), 'λ');
    builder.delete(new Range(1, 0, 1, 2));
    builder.replace(new Range(0, 3, 0, 5), 'β');
  }).then(result => { settled = true; return result; });
  assert.deepEqual(f.calls, [{method: 'edit', params: {
    session: 31, owner: 'marker.owner', document: 17, version: 7, editor: 'original-pane',
    undoStopBefore: true, undoStopAfter: true,
    edits: [
      {range: {start: {line: 0, character: 8}, end: {line: 0, character: 8}}, newText: 'λ', forceMoveMarkers: true},
      {range: {start: {line: 1, character: 0}, end: {line: 1, character: 2}}, newText: '', forceMoveMarkers: true},
      {range: {start: {line: 0, character: 3}, end: {line: 0, character: 5}}, newText: 'β'},
    ],
  }}]);
  await Promise.resolve();
  assert.equal(settled, false);
  assert.equal(editor.document.getText(), '猫🙂alpha\r\nsecond\r\n');
  // Only the native mirror changes document text; accepting the edit has no local text simulation.
  f.document.text = '猫🙂βphaλ\r\ncond\r\n';
  f.document.version = 8;
  f.sync();
  f.receipt.resolve({applied: true});
  assert.equal(await completion, true);
  assert.equal(settled, true);
  assert.equal(editor.document.version, 8);
  assert.equal(editor.document.getText(), f.document.text);
});

test('marker edits cannot send partial work after their captured source version changes', async () => {
  const f = fixture();
  await assert.rejects(f.editor.edit(builder => {
    builder.insert(new Position(0, 3), 'λ');
    builder.delete(new Range(1, 0, 1, 2));
    f.document.version++;
    f.sync();
    builder.replace(new Range(0, 3, 0, 5), 'β');
  }), /source changed/);
  assert.deepEqual(f.calls, []);
  assert.equal(f.editor.document.getText(), '猫🙂alpha\r\nsecond\r\n');
});

test('accepted marker edit ACK cannot reacquire a retired original caller lease', async () => {
  const f = fixture();
  const completion = f.editor.edit(builder => builder.insert(new Position(0, 3), 'λ'));
  const rejected = assert.rejects(completion, /retired/);
  assert.equal(f.calls[0].params.owner, 'marker.owner');
  assert.equal(f.calls[0].params.version, 7);
  f.runtime.disposeOwner('marker.owner');
  f.runtime.forExtension('marker.owner');
  f.receipt.resolve({applied: true});
  await rejected;
  assert.equal(f.calls.length, 1);
});

test('public edits normalize all delimiters using private captured EOL before native dispatch', async () => {
  for (const eol of [1, 2]) {
    const f=fixture();f.document.eol=eol;f.document.version++;f.sync();
    // Public metadata cannot replace the original native delimiter contract.
    f.editor.document._snapshot={eol:eol===1?2:1};
    const completion=f.editor.edit(builder=>builder.insert(new Position(0,0),'α\nβ\rγ\r\n'));
    assert.equal(f.calls.length,1);assert.equal(f.calls[0].params.edits[0].newText,eol===2?'α\r\nβ\r\nγ\r\n':'α\nβ\nγ\n');
    assert.equal(f.calls[0].params.edits[0].forceMoveMarkers,true);f.receipt.resolve({applied:true});assert.equal(await completion,true);
  }
});
test('CRLF expansion exceeding the native mirror budget rejects before any native edit request',async()=>{
  const f=fixture();const original=f.editor.document.getText();
  await assert.rejects(f.editor.edit(builder=>builder.insert(new Position(0,0),'\n'.repeat(2*1024*1024+1))),/4 MiB mirror budget/);
  assert.equal(f.calls.length,0);assert.equal(f.editor.document.getText(),original);
});

test('replace accepts Position, Range and Selection while preserving non-forced replacement markers', async () => {
  const f=fixture();
  const completed=f.editor.edit(builder=>{
    builder.replace(new Position(0,3),'λ\n');
    builder.replace(new Range(1,0,1,1),'R');
    builder.replace(new Selection(1,2,1,3),'S');
    builder.replace(new Position(999,999),'end');
  });
  assert.equal(f.calls.length,1);
  const edits=f.calls[0].params.edits;
  assert.deepEqual(edits[0],{range:{start:{line:0,character:3},end:{line:0,character:3}},newText:'λ\r\n'});
  assert.deepEqual(edits[1].range,{start:{line:1,character:0},end:{line:1,character:1}});
  assert.deepEqual(edits[2].range,{start:{line:1,character:2},end:{line:1,character:3}});
  assert.deepEqual(edits[3].range,{start:{line:2,character:0},end:{line:2,character:0}});
  assert(edits.every(edit=>!Object.hasOwn(edit,'forceMoveMarkers')));
  assert.equal(f.editor.document.getText(),'猫🙂alpha\r\nsecond\r\n');
  f.receipt.resolve({applied:true});assert.equal(await completed,true);
});

test('replace rejects unbranded positions and callback-retained use before reading position getters', async () => {
  const f=fixture();
  await assert.rejects(f.editor.edit(builder=>builder.replace({line:0,character:0},'bad')),/Invalid range/);
  let retained;
  const completed=f.editor.edit(builder=>{retained=builder;builder.replace(new Position(0,0),'ok');});
  let reads=0;const late=Object.create(Position.prototype);
  Object.defineProperties(late,{line:{get(){reads++;return 0;}},character:{get(){reads++;return 0;}}});
  assert.throws(()=>retained.replace(late,'late'),/only valid during/);
  assert.equal(reads,0);assert.equal(f.calls.length,1);
  f.receipt.resolve({applied:true});assert.equal(await completed,true);
});

test('Position getter reentrancy cannot replace an edit capture with a newer source', async () => {
  const f=fixture(),point=Object.create(Position.prototype);let changed=false;
  Object.defineProperties(point,{line:{get(){if(!changed){changed=true;f.document.version++;f.sync();}return 0;}},character:{get(){return 0;}}});
  await assert.rejects(f.editor.edit(builder=>builder.replace(point,'stale')),/source changed/);
  assert.equal(f.calls.length,0);assert.equal(f.editor.document.getText(),'猫🙂alpha\r\nsecond\r\n');
});

test('editor undo-stop options snapshot each getter once before callback and preserve defaults and coercion', async () => {
  for (const [options, before, after] of [[undefined,true,true],[{},false,false],
    [{undoStopBefore:false,undoStopAfter:true},false,true],
    [{undoStopBefore:1,undoStopAfter:0},true,false]]) {
    const f=fixture();
    const done=f.editor.edit(builder=>builder.insert(new Position(0,0),'x'),options);
    assert.equal(f.calls[0].params.undoStopBefore,before);
    assert.equal(f.calls[0].params.undoStopAfter,after);
    f.receipt.resolve({applied:true});assert.equal(await done,true);
  }
  const f=fixture(),reads=[];
  const options={get undoStopBefore(){reads.push('before');return false;},get undoStopAfter(){reads.push('after');return true;}};
  const done=f.editor.edit(builder=>{assert.deepEqual(reads,['before','after']);builder.insert(new Position(0,0),'x');},options);
  assert.deepEqual(reads,['before','after']);f.receipt.resolve({applied:true});assert.equal(await done,true);
});

test('undo-stop getter changes to the captured source reject the whole edit before native dispatch', async () => {
  const f=fixture();
  await assert.rejects(f.editor.edit(builder=>builder.insert(new Position(0,0),'lost'),{
    get undoStopBefore(){f.document.version++;f.sync();return false;},undoStopAfter:false,
  }),/source changed/);
  assert.equal(f.calls.length,0);
  for (const options of [null, false, 'invalid']) {
    await assert.rejects(f.editor.edit(()=>assert.fail('invalid options ran callback'),options),/options must be an object/);
  }
  assert.equal(f.calls.length,0);
});

test('empty editor builders skip native work and undo boundaries', async () => {
  for (const isReadonly of [false,true]) {
    const f=fixture();f.document.isReadonly=isReadonly;f.sync();
    assert.equal(await f.editor.edit(()=>{}, {undoStopBefore:true,undoStopAfter:true}),true);
    assert.equal(f.calls.length,0);
  }
});

test('editor EOL builders validate enums, retain only the last request and await native mirrors', async () => {
  const f=fixture();
  const done=f.editor.edit(builder=>{
    builder.setEndOfLine(2);builder.insert(new Position(0,3),'\nX');builder.setEndOfLine(1);
  },{undoStopBefore:false,undoStopAfter:true});
  assert.equal(f.calls[0].params.setEndOfLine,1);
  assert.equal(f.calls[0].params.edits[0].newText,'\r\nX');
  assert.equal(f.calls[0].params.undoStopBefore,false);
  assert.equal(f.editor.document.eol,2,'native EOL is not simulated locally');
  f.document.eol=1;f.document.text='猫🙂\nXalpha\nsecond\n';f.document.version++;f.sync();
  f.receipt.resolve({applied:true});assert.equal(await done,true);
  assert.equal(f.editor.document.eol,1);assert.equal(f.editor.document.getText(),f.document.text);
});

test('EOL-only editor requests retain explicit native work even for the current EOL', async () => {
  const f=fixture();let retained;
  const done=f.editor.edit(builder=>{retained=builder;builder.setEndOfLine(2);});
  assert.equal(f.calls.length,1);assert.deepEqual(f.calls[0].params.edits,[]);
  assert.equal(f.calls[0].params.setEndOfLine,2);
  assert.throws(()=>retained.setEndOfLine(1),/only valid during/);
  f.receipt.resolve({applied:true});assert.equal(await done,true);
});

test('invalid or stale EOL builders reject the entire text/EOL capture before dispatch', async () => {
  for (const value of [0,3,'1',null,undefined,NaN]) {
    const f=fixture();
    await assert.rejects(f.editor.edit(builder=>{builder.insert(new Position(0,0),'lost');builder.setEndOfLine(value);}),/Invalid endOfLine/);
    assert.equal(f.calls.length,0);
  }
  const f=fixture();
  await assert.rejects(f.editor.edit(builder=>{builder.setEndOfLine(1);f.document.version++;f.sync();}),/source changed/);
  assert.equal(f.calls.length,0);assert.equal(f.editor.document.eol,2);
});

test('editor EOL requires the negotiated native backend before sending any text prefix', async () => {
  const f=fixture(false);
  await assert.rejects(f.editor.edit(builder=>{builder.insert(new Position(0,0),'lost');builder.setEndOfLine(1);}),/not implemented by this host/);
  assert.equal(f.calls.length,0);assert.equal(f.editor.document.eol,2);
});
