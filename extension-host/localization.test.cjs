'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createLocalization } = require('./localization.cjs');
const { createApi } = require('./api.cjs');
const { Uri } = require('./uri.cjs');
function fixture(t, translations) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-l10n-'));
  t.after(() => fs.rmSync(root, {recursive:true, force:true}));
  const folder = path.join(root, 'package');
  fs.mkdirSync(path.join(folder, 'l10n'), {recursive:true});
  fs.writeFileSync(path.join(folder, 'l10n/bundle.l10n.fr.json'), JSON.stringify(translations));
  return { root, item: {id:'fixture.one', folder, manifest:{l10n:'./l10n'}} };
}
test('all public localization overloads format positional/named values, booleans, zero and comment disambiguation', t => {
  const f = fixture(t, {'Hello {0}':'Bonjour {0}', 'Hello {name}':'Salut {name}', 'Ready {0} {1}':'Prêt {0} {1}', 'Open/verbcontext':'Ouvrir', Empty:''});
  const service = createLocalization('fr'); service.add([f.item]);
  const api = service.forOwner(f.item.id);
  assert.equal(api.t('Hello {0}', '世界'), 'Bonjour 世界');
  assert.equal(api.t('Hello {name}', {name:'猫'}), 'Salut 猫');
  assert.equal(api.t('Ready {0} {1}', false, 0), 'Prêt false 0');
  assert.equal(api.t({message:'Open',comment:['verb','context']}), 'Ouvrir');
  assert.equal(api.t({message:'Hello {name}',args:{name:'🙂'},comment:''}), 'Salut 🙂');
  assert.equal(api.t('Empty'), '');
  assert.equal(api.t('Missing {0}', 'value'), 'Missing value');
  assert.equal(api.t('unbound {name}'), 'unbound {name}');
  assert.equal(api.uri.toString(), Uri.file(fs.realpathSync(path.join(f.item.folder, 'l10n/bundle.l10n.fr.json'))).toString());
});
test('English performs no bundle I/O and absent bundles preserve original messages', () => {
  const api = createLocalization('en');
  api.add([{id:'fixture.english',folder:'/missing-root',manifest:{l10n:'missing'}}]);
  const english = api.forOwner('fixture.english');
  assert.equal(english.t('Value {0}', 0), 'Value 0');
  assert.equal(english.bundle, undefined); assert.equal(english.uri, undefined);
  const notices = [], foreign = createLocalization('fr', (...args) => notices.push(args));
  foreign.add([{id:'fixture.missing',folder:'/missing-root',manifest:{l10n:'missing'}}]);
  assert.equal(foreign.forOwner('fixture.missing').t('Hello'), 'Hello');
  assert.equal(notices.length, 1);
});
test('selected owners and retained generations keep their own mutable translation bundles', t => {
  const f = fixture(t, {Hello:'Bonjour'});
  const service = createLocalization('fr'); service.add([f.item]);
  service.add([{...f.item,folder:'/not-a-new-generation'}]);
  const one = service.forOwner('fixture.one'), other = service.forOwner('fixture.two');
  one.bundle.Hello = 'Salut';
  assert.equal(one.t('Hello'), 'Salut'); assert.equal(other.t('Hello'), 'Hello');
  assert.equal(other.bundle, undefined); assert.equal(other.uri, undefined);
});
test('escaping and oversized bundles are diagnosed without reading unrestricted data or losing fallback', t => {
  const f = fixture(t, {Hello:'Bonjour'}), notices = [];
  fs.mkdirSync(path.join(f.root, 'outside'));
  fs.writeFileSync(path.join(f.root, 'outside/bundle.l10n.fr.json'), '{"Secret":"outside"}');
  const service = createLocalization('fr', (...args) => notices.push(args));
  service.add([{...f.item,manifest:{l10n:'../outside'}}]);
  assert.equal(service.forOwner('fixture.one').bundle, undefined);
  assert.match(notices[0][1], /escapes/);
  fs.writeFileSync(path.join(f.item.folder, 'l10n/bundle.l10n.fr.json'), ' '.repeat(1024*1024+1));
  service.add([{...f.item,id:'fixture.large'}]);
  assert.match(notices[1][1], /1 MiB/);
  assert.equal(service.forOwner('fixture.large').t('Hello'), 'Hello');
});
test('actual scoped API exposes the preferred language and correct owner bundle', t => {
  const f = fixture(t, {Hello:'Bonjour'});
  const runtime = createApi(() => { throw Error('no native request'); }, () => {}, {locale:'fr',applicationRoot:'/native/app'});
  runtime.configureLocalization([f.item]);
  const api = runtime.forExtension('fixture.one');
  assert.equal(api.env.language, 'fr'); assert.equal(api.env.appName, 'VSCLI');
  assert.equal(api.env.appRoot, '/native/app'); assert.equal(api.env.uiKind, api.UIKind.Desktop);
  assert.equal(api.env.remoteName, undefined);
  assert.equal(api.l10n.t('Hello'), 'Bonjour');
  assert.equal(runtime.forExtension('fixture.other').l10n.t('Hello'), 'Hello');
});
test('invalid locale tags and non-string bundle values cannot select unrelated files', t => {
  for (const value of ['', '../outside', 'en/../../', 'en_US', 'x'.repeat(65)]) assert.throws(() => createLocalization(value), /Invalid/);
  const f = fixture(t, {Hello:{message:'Bonjour'}}), notices = [];
  const service = createLocalization('fr', (...args) => notices.push(args)); service.add([f.item]);
  assert.equal(service.forOwner(f.item.id).bundle, undefined);
  assert.match(notices[0][1], /string translations/);
});
