'use strict';

// Shared observer draft: use the real pinned VS Code API first; do not bake in
// answers inferred from VSCLI's eventual implementation.
async function diagnosticTrace(vscode, { includeDocumentBridge = true } = {}) {
  const A = vscode.Uri.parse('file:///vscli-diagnostics-reference/a.cpp');
  const B = vscode.Uri.parse('file:///vscli-diagnostics-reference/b.cpp');
  const aliases = new Map([[A.toString(), 'a.cpp'], [B.toString(), 'b.cpp']]);
  const uri = value => aliases.get(value.toString()) || value.toString();
  const encode = value => value === undefined ? { undefined: true } : value;
  const position = value => ({ line: value.line, character: value.character });
  const range = value => ({ start: position(value.start), end: position(value.end) });
  const diagnostic = value => ({
    range: range(value.range), message: value.message, severity: value.severity,
    source: encode(value.source),
    code: encode(value.code && typeof value.code === 'object'
      ? { value: value.code.value, target: uri(value.code.target) } : value.code),
    tags: encode(value.tags),
    relatedInformation: encode(value.relatedInformation?.map(info => ({
      location: { uri: uri(info.location.uri), range: range(info.location.range) }, message: info.message,
    }))),
  });
  const diagnostics = value => value === undefined ? encode(value) : value.map(diagnostic);
  const capture = operation => {
    try { return { result: encode(operation()) }; }
    catch (error) { return { error: { name: error.name, message: error.message } }; }
  };
  const make = (message, severity) => new vscode.Diagnostic(new vscode.Range(0, 1, 0, 3), message, severity);
  const snapshots = [], events = [], retainedEventUris = [];
  const one = vscode.languages.createDiagnosticCollection('vscli-reference-one');
  // Same public name must not merge independently created collections.
  const two = vscode.languages.createDiagnosticCollection('vscli-reference-one');
  let phase = '', lastEvent = 0, twoDisposed = false;
  const aggregate = () => vscode.languages.getDiagnostics().filter(([resource]) => aliases.has(resource.toString()))
    .map(([resource, values]) => [uri(resource), diagnostics(values)]).sort(([a], [b]) => a.localeCompare(b));
  const observe = name => snapshots.push({ name,
    one: { name: one.name, a: diagnostics(one.get(A)), b: diagnostics(one.get(B)), hasA: one.has(A), hasB: one.has(B),
      entries: [...one].map(([resource, values]) => [uri(resource), diagnostics(values)]) },
    two: twoDisposed ? { disposed: true } : { name: two.name, a: diagnostics(two.get(A)), b: diagnostics(two.get(B)) }, aggregate: aggregate(),
  });
  const listener = vscode.languages.onDidChangeDiagnostics(event => {
    const resources = event.uris.filter(resource => aliases.has(resource.toString())).map(uri).sort();
    if (resources.length) {
      lastEvent = Date.now();
      retainedEventUris.push(event.uris);
      events.push({ phase, uris: resources, aggregate: aggregate(), arrayFrozen: Object.isFrozen(event.uris) });
    }
  });
  // This records observed batches, not a contractual exact millisecond delay.
  // Matching should compare batch URI sets and listener-time state; only the
  // explicit same-turn burst is a coalescing observation.
  async function settle(name, operation) {
    phase = name; lastEvent = Date.now(); operation();
    const started = Date.now();
    while (Date.now() - lastEvent < 200) {
      if (Date.now() - started > 3000) throw new Error(`Diagnostic events did not settle: ${name}`);
      await new Promise(resolve => setTimeout(resolve, 20));
    }
    observe(name);
  }
  let document;
  try {
    observe('empty');
    const missing = one.get(A);
    snapshots.push({ name: 'missing-read-array', arrayFrozen: Object.isFrozen(missing),
      arrayPush: capture(() => missing.push(make('missing array mutation'))),
      nextRead: diagnostics(one.get(A)), has: one.has(A) });
    const rich = make('猫🙂 rich');
    rich.source = 'native-reference'; rich.code = { value: 'E001', target: vscode.Uri.parse('https://example.invalid/E001') };
    rich.tags = [vscode.DiagnosticTag.Unnecessary, vscode.DiagnosticTag.Deprecated];
    rich.relatedInformation = [new vscode.DiagnosticRelatedInformation(new vscode.Location(B, new vscode.Range(1, 0, 1, 2)), 'related 猫')];
    const warning = make('warning', vscode.DiagnosticSeverity.Warning);
    warning.code = 7;
    await settle('single-set-rich-metadata', () => one.set(A, [rich, warning]));

    const returned = one.get(A), original = [make('input array')];
    const immutable = {
      arrayFrozen: Object.isFrozen(returned), diagnosticFrozen: Object.isFrozen(returned[0]),
      diagnosticIdentity: returned[0] === rich,
      arrayPush: capture(() => returned.push(make('attempted array mutation'))),
      arrayElementAssignment: capture(() => { returned[0] = warning; }),
    };
    snapshots.push({ name: 'immutable-read-attempts', ...immutable, read: diagnostics(one.get(A)) });
    await settle('held-input-and-read', () => {
      rich.message = 'mutated original diagnostic'; original.push(make('input second'));
      one.set(B, original); original.push(make('input post-set')); one.set(A, [make('replacement')]);
    });
    snapshots.push({ name: 'held-read-after-replacement', held: diagnostics(returned), read: diagnostics(one.get(A)), input: diagnostics(original), collectionInput: diagnostics(one.get(B)) });

    const information = make('other owner', vscode.DiagnosticSeverity.Information); information.code = 'STR';
    await settle('two-collection-aggregation', () => two.set(A, [information, make('hint', vscode.DiagnosticSeverity.Hint)]));
    await settle('bulk-duplicates-merge', () => one.set([[A, [make('bulk first')]], [A, [make('bulk second')]]]));
    await settle('bulk-undefined-resets-only-preceding', () => one.set([[A, [make('discard')]], [A, undefined], [A, [make('survive')]], [A, [make('also survive')]]]));
    await settle('bulk-final-undefined', () => one.set([[A, [make('discard final')]], [A, undefined]]));
    await settle('bulk-empty-nonfinal-group', () => one.set([[A, undefined], [B, [make('b final')]]]));
    await settle('bulk-empty-final-group', () => one.set([[B, undefined], [A, [make('a before final')]]]));
    await settle('empty-array-membership', () => one.set(A, []));
    await settle('delete-and-absent-delete', () => { one.delete(A); one.delete(A); });
    await settle('set-undefined-single', () => { one.set(A, [make('remove single')]); one.set(A, undefined); });

    const thisArg = { token: 'thisArg' }, iteration = [];
    one.forEach(function(resource, values, collection) {
      iteration.push({ uri: uri(resource), values: diagnostics(values), receiver: this === thisArg,
        collectionIdentity: collection === one, arrayFrozen: Object.isFrozen(values) });
    }, thisArg);
    snapshots.push({ name: 'forEach-iterator', forEach: iteration,
      iterator: [...one].map(([resource, values]) => ({ uri: uri(resource), values: diagnostics(values), arrayFrozen: Object.isFrozen(values) })) });

    await settle('same-turn-coalesced-burst', () => {
      one.set(A, [make('burst one')]); one.set(A, [make('burst two')]);
      one.set(B, [make('burst b')]); two.set(B, [make('burst owner b')]);
    });
    await settle('set-undefined-all', () => one.set(undefined));
    await settle('clear', () => { one.set(A, [make('clear a')]); one.set(B, [make('clear b')]); one.clear(); });

    if (includeDocumentBridge) {
      document = await vscode.workspace.openTextDocument({ content: 'α🙂\r\n猫\r\n', language: 'plaintext' });
      aliases.set(document.uri.toString(), 'open-document');
      const editor = await vscode.window.showTextDocument(document, { preview: false });
      await settle('document-set-before-edit', () => one.set(document.uri, [make('document diagnostic')]));
      await editor.edit(builder => builder.insert(new vscode.Position(0, 0), 'PREFIX'));
      await settle('document-edit-retention', () => {});
      snapshots.push({ name: 'document-edited-local-read', version: document.version, text: document.getText(), diagnostics: diagnostics(one.get(document.uri)) });
      let closeSubscription, closeTimer;
      const closed = new Promise((resolve, reject) => {
        closeSubscription = vscode.workspace.onDidCloseTextDocument(value => { if (value === document) resolve(); });
        closeTimer = setTimeout(() => reject(new Error('Fixture document did not close')), 3000);
      });
      try { await Promise.all([closed, vscode.commands.executeCommand('workbench.action.revertAndCloseActiveEditor')]); }
      finally { clearTimeout(closeTimer); closeSubscription.dispose(); }
      await settle('document-close-retention', () => {});
      snapshots.push({ name: 'closed-document-local-read', closed: document.isClosed, diagnostics: diagnostics(one.get(document.uri)) });
    }

    await settle('dispose-second-collection', () => { two.dispose(); twoDisposed = true; });
    // Capture post-disposal failures without asserting undocumented details.
    const disposed = {};
    for (const [name, operation] of Object.entries({ get: () => two.get(A), has: () => two.has(A),
      set: () => two.set(A, [make('disposed')]), clear: () => two.clear(), delete: () => two.delete(A),
      iterate: () => [...two], forEach: () => two.forEach(() => {}), disposeAgain: () => two.dispose(), name: () => two.name })) {
      disposed[name] = capture(operation);
    }
    snapshots.push({ name: 'post-disposal-observations', methods: disposed });
    return { schema: 1, snapshots, events, retainedEventUris: retainedEventUris.map(resources =>
      ({ uris: resources.filter(resource => aliases.has(resource.toString())).map(uri).sort(), arrayFrozen: Object.isFrozen(resources) })) };
  } finally { listener.dispose(); one.dispose(); two.dispose(); }
}
module.exports = { diagnosticTrace };
