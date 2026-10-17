'use strict';
const v = require('vscode'), assert = require('node:assert/strict');
exports.activate = context => {
  const source = v.window.activeTextEditor.document, root = v.Uri.file(v.workspace.rootPath);
  const original = { source: '猫🙂 source\r\nnext\r\n', target: 'α🙂 target\r\n', hidden: 'γ🙂 source\r\n', unopened: 'β🙂 source\r\n', unrelated: 'δ🙂 unrelated\r\n' };
  const uri = name => v.Uri.joinPath(root, name + '.cpp');
  const announce = text => v.window.showInformationMessage(text);
  const register = (name, fn) => context.subscriptions.push(v.commands.registerCommand('refactorFixture.' + name, fn));
  const saved = new Map(), documents = { source };
  let prepared = false, applying = false, expectedSaved = 4, eventFailure, untitled, savedUntitled, saveAsMode;
  const totalSaved = () => [...saved.values()].reduce((a, b) => a + b, 0);
  function exact(name, renamed) {
    if (name === 'unrelated') return 'independent ' + original[name];
    const prefix = name === 'unopened' ? '' : 'dirty ';
    return prefix + original[name].replace(name === 'target' ? 'target' : 'source', renamed ? 'renamed' : name === 'target' ? 'target' : 'source');
  }
  function collect() {
    for (const name of ['target', 'hidden', 'unopened', 'unrelated']) {
      const match = v.workspace.textDocuments.find(doc => doc.uri.toString() === uri(name).toString());
      if (match) {
        if (documents[name]) assert.strictEqual(match, documents[name], name + ' identity');
        else documents[name] = match;
      }
    }
  }
  function cohortText(renamed) {
    collect();
    assert.strictEqual(v.window.activeTextEditor.document, source);
    for (const name of ['source', 'target', 'hidden', 'unopened', 'unrelated']) {
      assert(documents[name], name + ' model must be published');
      assert(v.workspace.textDocuments.includes(documents[name]));
      assert.equal(documents[name].getText(), exact(name, renamed), name + ' text');
    }
    assert.equal(documents.unrelated.isDirty, true);
    assert.equal(v.window.visibleTextEditors.filter(editor => editor.document === documents.target).length, 2);
    assert(v.window.visibleTextEditors.every(editor => editor.document !== documents.hidden && editor.document !== documents.unopened));
  }
  function captureEvent(fn) { try { fn(); } catch (error) { eventFailure ??= String(error.stack || error).slice(0, 4096); } }
  context.subscriptions.push(v.workspace.onDidChangeTextDocument(event => {
    if (applying && prepared && ['source', 'target', 'hidden', 'unopened'].some(name => event.document.uri.toString() === uri(name).toString())) {
      captureEvent(() => cohortText(true)); // Complete text cohort precedes every change event.
    }
  }));
  context.subscriptions.push(v.workspace.onDidOpenTextDocument(document => {
    if (saveAsMode && untitled && document.uri.scheme === 'file' && document.uri.toString() !== source.uri.toString()) savedUntitled = document;
  }));
  context.subscriptions.push(v.workspace.onDidSaveTextDocument(document => {
    captureEvent(() => {
      assert.equal(document.isDirty, false);
      if (prepared) {
        cohortText(true);
        const name = ['source', 'target', 'hidden', 'unopened'].find(name => document.uri.toString() === uri(name).toString());
        assert(name, 'unrelated work must not be saved');
        assert.strictEqual(document, documents[name]);
      } else if (saveAsMode) {
        assert(document === source || document === savedUntitled);
        if (document === savedUntitled) { assert(untitled.isClosed); assert.notStrictEqual(document, untitled); }
      }
      saved.set(document.uri.toString(), (saved.get(document.uri.toString()) || 0) + 1);
    });
  }));
  async function prepare() {
    assert.equal(source.getText(), 'dirty ' + original.source); assert(source.isDirty);
    documents.target = await v.workspace.openTextDocument(uri('target'));
    documents.hidden = await v.workspace.openTextDocument(uri('hidden'));
    documents.unrelated = await v.workspace.openTextDocument(uri('unrelated'));
    const editor = await v.window.showTextDocument(documents.target, { preview: false, viewColumn: v.ViewColumn.Beside });
    assert.equal(await editor.edit(builder => builder.insert(new v.Position(0, 0), 'dirty ')), true);
    const shared = await v.window.showTextDocument(documents.target, { preview: false, viewColumn: v.ViewColumn.Beside, preserveFocus: true });
    assert.notStrictEqual(editor, shared); assert.strictEqual(editor.document, shared.document);
    const edits = new v.WorkspaceEdit(); edits.insert(documents.hidden.uri, new v.Position(0, 0), 'dirty '); edits.insert(documents.unrelated.uri, new v.Position(0, 0), 'independent ');
    assert.equal(await v.workspace.applyEdit(edits, { isRefactoring: false }), true);
    await v.window.showTextDocument(source, { preview: false, viewColumn: 1 });
    assert(!v.workspace.textDocuments.some(doc => doc.uri.toString() === uri('unopened').toString()));
    assert.equal(totalSaved(), 0); prepared = true;
    return announce('API refactoring cohort prepared;dirty/shared/hidden/unopened=true');
  }
  function edits(newName = 'renamed') {
    const result = new v.WorkspaceEdit();
    for (const name of ['source', 'target', 'hidden']) result.replace(documents[name].uri, new v.Range(0, 10, 0, 16), newName);
    result.replace(uri('unopened'), new v.Range(0, 4, 0, 10), newName);
    return result;
  }
  async function verifyApplied(label, savedExpected) {
    cohortText(true); assert.equal(eventFailure, undefined, eventFailure); assert.equal(totalSaved(), savedExpected);
    for (const name of ['source', 'target', 'hidden', 'unopened']) assert.equal(documents[name].isDirty, savedExpected !== 4, name + ' dirty');
    applying = false;
    return announce('API refactoring ' + label + ';applied=true;didSave=' + savedExpected + ';identity/shared/hidden=true');
  }
  register('prepare', prepare);
  register('apply', async () => {
    assert(prepared); expectedSaved = 4; applying = true;
    assert.equal(await v.workspace.applyEdit(edits(), { isRefactoring: true }), true);
    return verifyApplied('default', 4);
  });
  register('applyOff', async () => {
    assert(prepared); expectedSaved = 0; applying = true;
    assert.equal(await v.workspace.applyEdit(edits(), { isRefactoring: true }), true);
    return verifyApplied('off', 0);
  });
  context.subscriptions.push(v.languages.registerRenameProvider('cpp', {
    prepareRename(document, position) { assert.strictEqual(document, source); return new v.Range(0, 10, 0, 16); },
    provideRenameEdits(document, position, newName) { assert.strictEqual(document, source); assert(prepared); applying = true; return edits(newName); },
  }));
  register('armRename', () => { assert(prepared); expectedSaved = 4; return announce('API refactoring F2 armed'); });
  register('applied', () => verifyApplied('F2', 4));
  register('undo', () => {
    cohortText(false); assert.equal(totalSaved(), expectedSaved);
    for (const name of ['source', 'target', 'hidden']) assert(documents[name].isDirty);
    assert.equal(documents.unopened.isDirty, expectedSaved === 4);
    return announce('API refactoring grouped Undo;allOriginal=true;didSave=' + expectedSaved);
  });
  register('redo', () => verifyApplied('Redo', expectedSaved));
  register('single', async () => {
    assert.equal(source.getText(), 'dirty ' + original.source);
    const edit = new v.WorkspaceEdit(); edit.replace(source.uri, new v.Range(0, 10, 0, 16), 'renamed');
    assert.equal(await v.workspace.applyEdit(edit, { isRefactoring: true }), true);
    assert.equal(source.getText(), exact('source', true)); assert(source.isDirty); assert.equal(totalSaved(), 0);
    return announce('API refactoring single;applied=true;didSave=0;dirty=true');
  });
  async function saveAs(cancel) {
    saveAsMode = true;
    untitled = await v.workspace.openTextDocument({ language: 'cpp', content: 'untitled 猫🙂 before\r\n' });
    const edits = new v.WorkspaceEdit(); edits.insert(source.uri, new v.Position(0, 0), 'named '); edits.insert(untitled.uri, new v.Position(0, 0), 'saved ');
    assert.equal(await v.workspace.applyEdit(edits, { isRefactoring: true }), true);
    assert.equal(eventFailure, undefined, eventFailure);
    assert.equal(source.getText(), 'named dirty ' + original.source); assert.equal(source.isDirty, false);
    if (cancel) { assert.equal(totalSaved(), 1); assert(untitled.isDirty); assert.equal(untitled.isClosed, false); assert.equal(untitled.getText(), 'saved untitled 猫🙂 before\r\n'); }
    else { assert.equal(totalSaved(), 2); assert(untitled.isClosed); assert(savedUntitled); assert.equal(savedUntitled.isDirty, false); assert.equal(savedUntitled.getText(), 'saved untitled 猫🙂 before\r\n'); }
    if (cancel) await new Promise(resolve => setTimeout(resolve, 100)); // Let the real native warning remain observable before the fixture's final notice.
    return announce('API refactoring SaveAs ' + (cancel ? 'canceled' : 'accepted') + ';applied=true;didSave=' + totalSaved() + ';namedCommitted=true');
  }
  register('saveAs', () => saveAs(false)); register('cancelSaveAs', () => saveAs(true));
  register('conflict', async () => {
    assert(prepared); applying = true;
    assert.equal(await v.workspace.applyEdit(edits(), { isRefactoring: true }), true);
    cohortText(true); assert.equal(eventFailure, undefined, eventFailure); assert.equal(totalSaved(), 3);
    assert.equal(documents.target.isDirty, true);
    for (const name of ['source', 'hidden', 'unopened']) assert.equal(documents[name].isDirty, false);
    applying = false;
    await new Promise(resolve => setTimeout(resolve, 100)); // Observe actual warning before overwriting status with the test receipt.
    return announce('API refactoring conflict;applied=true;didSave=3;dirtyFailedTarget=true;noRetry=true');
  });
  register('unsupported', async () => {
    const before = source.getText(), version = source.version;
    const edits = new v.WorkspaceEdit(); edits.insert(source.uri, new v.Position(0, 0), 'must not apply '); edits.createFile(uri('unsupported-created'), { contents: new Uint8Array([65]) });
    assert.equal(await v.workspace.applyEdit(edits, { isRefactoring: true }), false);
    assert.equal(source.getText(), before); assert.equal(source.version, version); assert.equal(totalSaved(), 0);
    return announce('API refactoring unsupported;wholeRefused=true;didSave=0');
  });
};
