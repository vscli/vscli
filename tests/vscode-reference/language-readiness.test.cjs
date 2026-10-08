'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { waitForComments } = require('./language-readiness.cjs');

function probe(readyAfter, block = false) {
  let text = 'vscli_probe', attempts = 0, closed = false;
  const document = { getText: () => text, positionAt: offset => offset };
  const editor = { edit: async apply => { apply({ replace: (_range, value) => { text = value; } }); return true; } };
  const vscode = {
    Range: class {}, Selection: class {},
    workspace: { openTextDocument: async () => document },
    window: { showTextDocument: async () => editor },
    commands: { executeCommand: async command => {
      if (command === 'workbench.action.revertAndCloseActiveEditor') { closed = true; return; }
      assert.equal(command, block ? 'editor.action.blockComment' : 'editor.action.addCommentLine');
      assert.equal(text, 'vscli_probe');
      if (++attempts >= readyAfter) text = block ? '/*vscli_probe*/' : '# vscli_probe';
    } },
  };
  return { vscode, state: () => ({ attempts, closed }) };
}

test('waits for delayed language registration without invoking snippets', async () => {
  const fixture = probe(3);
  await waitForComments(fixture.vscode, 'shellscript', { lineComment: '#' });
  assert.deepEqual(fixture.state(), { attempts: 3, closed: true });
});

test('supports block-only languages and closes the readiness document', async () => {
  const fixture = probe(1, true);
  await waitForComments(fixture.vscode, 'css', { blockComment: ['/*', '*/'] });
  assert.deepEqual(fixture.state(), { attempts: 1, closed: true });
});

test('fails closed on missing registration instead of emitting an unstable trace', async () => {
  const fixture = probe(Infinity);
  await assert.rejects(waitForComments(fixture.vscode, 'shellscript', { lineComment: '#' }, 0), /did not become ready/);
  assert.deepEqual(fixture.state(), { attempts: 1, closed: true });
});
