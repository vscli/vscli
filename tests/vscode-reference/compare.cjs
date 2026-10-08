'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { execFileSync } = require('node:child_process');
const { isDeepStrictEqual } = require('node:util');
const { createApi } = require('../../extension-host/api.cjs');
const { layers, configurationTrace } = require('./contracts.cjs');

// Textual comparison only. Reordering standard modifiers does not resolve
// physical keys, scan codes, layouts, context expressions, or command behavior.
function keyText(key) {
  return key.toLowerCase().trim().split(/\s+/).map(stroke => {
    const parts = stroke.split('+');
    const key = parts.pop();
    return [...parts.sort(), key === 'esc' ? 'escape' : key].join('+');
  }).join(' ');
}

function compareBindings(reference, native) {
  const counts = { sameRuleFields: 0, sameKeyCommandArgs: 0, sameCommandId: 0, missingCommandId: 0 };
  const rules = reference.map((rule, index) => {
    const command = native.filter(candidate => candidate.command === rule.command);
    const keyArgs = command.filter(candidate => keyText(candidate.key) === keyText(rule.key)
      && Object.hasOwn(candidate, 'args') === Object.hasOwn(rule, 'args')
      && isDeepStrictEqual(candidate.args, rule.args));
    const fields = keyArgs.filter(candidate => (candidate.when ?? null) === (rule.when ?? null));
    const category = fields.length ? 'sameRuleFields' : keyArgs.length ? 'sameKeyCommandArgs'
      : command.length ? 'sameCommandId' : 'missingCommandId';
    counts[category]++;
    return { index, category, reference: rule, nativeCandidates: command };
  });
  return { referenceRules: reference.length, nativeRules: native.length,
    referenceCommandIds: new Set(reference.map(rule => rule.command)).size,
    nativeCommandIds: new Set(native.map(rule => rule.command)).size, counts, rules };
}

async function shimTrace() {
  const current = structuredClone(layers);
  const host = createApi(() => { throw new Error('Unexpected native request'); }, () => {});
  host.configure(process.cwd(), require('./package.json').contributes.configuration, current);
  return configurationTrace(host.api, host.api.Uri.file(path.join(process.cwd(), 'fixture.js')),
    async (layer, key, value) => {
      current[layer][key] = value;
      host.updateConfiguration(current);
    });
}

async function main(directory, binary) {
  assert.ok(directory && binary, 'Usage: node compare.cjs <reference-output> <vscli-binary>');
  const read = name => JSON.parse(fs.readFileSync(path.join(directory, name), 'utf8'));
  const reference = read('inventory.json');
  assert.equal(reference.version, '1.95.0');
  assert.equal(reference.commit, '912bb683695358a54ae0c670461738984cbb5b95');
  const profile = { linux: 'linux', darwin: 'macos', win32: 'windows' }[reference.platform];
  assert.ok(profile, `Unsupported reference platform: ${reference.platform}`);
  const executable = path.resolve(binary);
  const native = JSON.parse(execFileSync(executable, ['--list-keybindings', '--keymap', profile],
    { encoding: 'utf8', timeout: 15000, maxBuffer: 10 * 1024 * 1024 }));
  const bindings = compareBindings(reference.bindings, native);
  const expected = read('configuration.json');
  const actual = await shimTrace();
  assert.equal(expected.length, 25, 'Reference contract observations must not silently disappear');
  const report = {
    schema: 1, reference: { version: reference.version, commit: reference.commit,
      platform: reference.platform, arch: reference.arch, keyboardLayout: reference.keyboardLayout },
    native: { profile, sha256: createHash('sha256').update(fs.readFileSync(executable)).digest('hex') },
    limitations: [
      'Binding categories compare rule fields, not resolver outcomes, rule precedence, or command effects.',
      'Repeated reference rules remain separate; one native rule may match more than one reference rule.',
      'Keyboard delivery, physical layouts, IME, terminal conflicts and command behavior are not qualified.',
      'Configuration traces qualify only the shared read/event cases; writes use host-specific fixture adapters.',
    ],
    configuration: { observations: expected.length, equal: isDeepStrictEqual(actual, expected) },
    bindings,
  };
  fs.writeFileSync(path.join(directory, 'comparison.json'), JSON.stringify(report, null, 2) + '\n');
  fs.writeFileSync(path.join(directory, 'configuration-vscli.json'), JSON.stringify(actual, null, 2) + '\n');
  console.log(JSON.stringify({ ...report, bindings: { ...bindings, rules: undefined } }, null, 2));
  assert.deepEqual(actual, expected, 'VSCLI configuration observations differ from the pinned reference');
}

if (require.main === module) main(...process.argv.slice(2)).catch(error => { console.error(error); process.exitCode = 1; });
module.exports = { compareBindings, shimTrace };
