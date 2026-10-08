'use strict';

// Both hosts run these reads. Only the fixture's settings writer is host-specific;
// this does not qualify VSCLI's unsupported Configuration.update API.
const layers = [
  { 'vscliReference.enabled': false, 'vscliReference.object': { nested: { b: 2 }, list: [3] },
    'vscliReference.machine': 'user', '[javascript]': { 'vscliReference.language': 2 } },
  { 'vscliReference.enabled': true, 'vscliReference.object': { nested: { a: 4 } },
    'vscliReference.machine': 'workspace', 'vscliReference.language': 3,
    '[javascript][typescript]': { 'vscliReference.language': 4 } },
];

async function configurationTrace(vscode, resource, update) {
  const trace = [];
  const record = (name, value) => trace.push({ name, value });
  const get = scope => vscode.workspace.getConfiguration('vscliReference', scope);
  const held = get();
  for (const key of ['enabled', 'object', 'null', 'language', 'machine']) {
    record(`initial.get.${key}`, held.get(key));
    record(`initial.inspect.${key}`, held.inspect(key));
  }
  record('initial.direct.object', held.object);
  record('missing.fallback', held.get('missing', 'fallback'));
  record('missing.inspect', held.inspect('missing'));
  record('has', ['enabled', 'null', 'missing', ''].map(key => held.has(key)));
  record('language.uri', get(resource).get('language'));
  record('language.javascript', get({ uri: resource, languageId: 'javascript' }).get('language'));
  record('language.typescript', get({ languageId: 'typescript' }).get('language'));
  record('language.inspect', get({ languageId: 'javascript' }).inspect('language'));
  const affects = event => ({
    parent: event.affectsConfiguration('vscliReference'),
    exact: event.affectsConfiguration('vscliReference.enabled'),
    sibling: event.affectsConfiguration('vscliReference.object'),
    partial: event.affectsConfiguration('vscliRef'),
    child: event.affectsConfiguration('vscliReference.enabled.child'),
    uri: event.affectsConfiguration('vscliReference.enabled', resource),
    null: event.affectsConfiguration('vscliReference.enabled', null),
  });
  let observed, event;
  const subscription = vscode.workspace.onDidChangeConfiguration(change => {
    if (change.affectsConfiguration('vscliReference.enabled')) {
      event = change;
      observed = get().get('enabled');
    }
  });
  try {
    await update(0, 'vscliReference.enabled', true);
    if (!event) throw new Error('Masked user change did not deliver a configuration event');
    record('masked.event', affects(event));
    record('masked.listenerRead', observed);
    const retained = event;
    event = undefined;
    await update(1, 'vscliReference.enabled', false);
    if (!event) throw new Error('Workspace change did not deliver a configuration event');
    record('changed.event', affects(event));
    record('changed.listenerRead', observed);
    record('held.get', held.get('enabled'));
    record('held.inspect', held.inspect('enabled'));
    record('retained.event', affects(retained));
  } finally {
    subscription.dispose();
  }
  // Preserve undefined distinctly from null and missing object properties.
  return JSON.parse(JSON.stringify(trace, (_key, value) => value === undefined ? { $undefined: true } : value));
}

module.exports = { layers, configurationTrace };
