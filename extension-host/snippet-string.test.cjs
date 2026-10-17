'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { SnippetString } = require('./snippet-string.cjs');
const { createApi } = require('./api.cjs');
const { normalize } = require('./providers.cjs');

test('snippet text escaping, explicit and automatic tabstops preserve builder identity', () => {
  const snippet = new SnippetString();
  assert.equal(snippet.appendText('猫 $x}\\'), snippet);
  assert.equal(snippet.appendTabstop(9).appendTabstop().appendTabstop(), snippet);
  assert.equal(snippet.value, '猫 \\$x\\}\\\\$9$1$2');
  assert.equal(new SnippetString('raw $1').value, 'raw $1');
  assert.equal(new SnippetString(null).value, '');
  assert.equal(SnippetString.isSnippetString({ value: 'raw $1' }), true);
  assert.equal(SnippetString.isSnippetString(null), false);
  assert.throws(() => snippet.appendText(42), TypeError);
});

test('nested placeholder callbacks share numbering and escape only literal text', () => {
  const snippet = new SnippetString();
  const result = snippet.appendPlaceholder(nested => {
    nested.appendText('outer: ').appendPlaceholder(inner => {
      inner.appendText('$猫}').appendTabstop();
    });
  }).appendTabstop();
  assert.equal(result, snippet);
  assert.equal(snippet.value, '${1:outer: ${2:\\$猫\\}$3}}$4');
  assert.equal(new SnippetString().appendPlaceholder('$}', 7).appendTabstop().value, '${7:\\$\\}}$1');
});

test('choices escape comma pipe and backslash while retaining dollar and closing brace', () => {
  const values = Object.freeze(['a,b', 'c|d', 'e\\f', '$猫}']);
  const snippet = new SnippetString();
  assert.equal(snippet.appendChoice(values), snippet);
  assert.equal(snippet.appendChoice([], 9).appendTabstop(), snippet);
  assert.equal(snippet.value, '${1|a\\,b,c\\|d,e\\\\f,$猫}|}${9||}$2');
  assert.deepEqual(values, ['a,b', 'c|d', 'e\\f', '$猫}']);
});

test('variable defaults escape dollar and brace, retain backslash, and support callbacks', () => {
  const snippet = new SnippetString();
  snippet.appendVariable('NAME').appendVariable('EMPTY', '')
    .appendVariable('TEXT', '$猫}\\').appendVariable('NESTED', nested => {
      nested.appendPlaceholder('value').appendTabstop();
    }).appendTabstop();
  assert.equal(snippet.value, '${NAME}${EMPTY}${TEXT:\\$猫\\}\\}${NESTED:${1:value}$2}$3');
});

test('throwing nested callback leaves outer value intact and consumes only outer default number', () => {
  const snippet = new SnippetString('before');
  const error = new Error('callback failed');
  assert.throws(() => snippet.appendPlaceholder(nested => {
    nested.appendTabstop().appendTabstop();
    throw error;
  }), candidate => candidate === error);
  assert.equal(snippet.value, 'before');
  assert.equal(snippet.appendTabstop().value, 'before$2');
});

test('extension facade exports the same constructor and completion retains the exact snippet source', () => {
  const runtime = createApi(() => { throw new Error('Unexpected native request'); }, () => {},
    { session: 1, languageProviders: true });
  const vscode = runtime.forExtension('fixture.snippets');
  assert.equal(vscode.SnippetString, SnippetString);
  const snippet = new vscode.SnippetString().appendPlaceholder(nested => {
    nested.appendText('猫').appendTabstop();
  }).appendChoice(['first', 'second']).appendVariable('TM_FILENAME');
  const result = normalize('completion', [new vscode.CompletionItem('builder'),
    { label: 'snippet', insertText: snippet }]);
  assert.equal(result.items[1].insertText, snippet.value);
  assert.equal(result.items[1].insertTextFormat, 2);
});
