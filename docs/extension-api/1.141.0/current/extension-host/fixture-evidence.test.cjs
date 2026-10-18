'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const fixturePaths = ['primary', 'lazy'].map(name =>
  path.join(__dirname, '..', 'tests', 'fixtures', `code-action-provider-${name}`, 'publish-evidence.cjs'));
const oldRecord = '{"id":7,"text":"old 猫\\r\\n"}';
const newRecord = JSON.stringify({ id: 19, text: 'new 猫🙂\r\n', original: true });
function directory(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-fixture-evidence-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return root;
}
function reader(destination, expected) {
  if (expected === undefined) {
    assert.equal(fs.existsSync(destination), false);
  } else {
    assert.equal(fs.readFileSync(destination, 'utf8'), expected);
    assert.deepEqual(JSON.parse(fs.readFileSync(destination, 'utf8')), JSON.parse(expected));
  }
}
for (const helperPath of fixturePaths) {
  const publish = require(helperPath);
  const owner = path.basename(path.dirname(helperPath));
  for (const initial of [undefined, oldRecord]) {
    test(`${owner}: staging exposes ${initial === undefined ? 'no final path' : 'the complete previous JSON'} until rename`, t => {
      const root = directory(t), destination = path.join(root, 'evidence.json'), phases = [];
      if (initial !== undefined) fs.writeFileSync(destination, initial);
      const io = {
        ...fs,
        openSync(...args) {
          const fd = fs.openSync(...args);
          reader(destination, initial); phases.push('open');
          return fd;
        },
        writeFileSync(fd, text) {
          const bytes = Buffer.from(text, 'utf8'), half = Math.floor(bytes.length / 2);
          // Positively expose a partial STAGED file, with a synchronous reader
          // at that exact boundary; the destination must still be absent/old.
          fs.writeSync(fd, bytes.subarray(0, half));
          reader(destination, initial); phases.push('partial');
          fs.writeSync(fd, bytes.subarray(half));
          reader(destination, initial); phases.push('complete-stage');
        },
        closeSync(fd) { fs.closeSync(fd); reader(destination, initial); phases.push('closed'); },
        renameSync(source, target) {
          assert.equal(path.dirname(source), path.dirname(target));
          assert.equal(fs.readFileSync(source, 'utf8'), newRecord);
          reader(destination, initial); phases.push('before-rename');
          fs.renameSync(source, target);
          reader(destination, newRecord); phases.push('published');
        },
      };
      publish(destination, newRecord, io);
      reader(destination, newRecord);
      assert.deepEqual(phases, ['open', 'partial', 'complete-stage', 'closed', 'before-rename', 'published']);
      assert.deepEqual(fs.readdirSync(root), ['evidence.json']);
    });
  }
  for (const failure of ['write', 'rename']) {
    test(`${owner}: ${failure} failure propagates once, preserves final JSON and removes its staged file`, t => {
      const root = directory(t), destination = path.join(root, 'evidence.json');
      fs.writeFileSync(destination, oldRecord);
      const sentinel = new Error(`controlled ${failure} failure`);
      let failedCalls = 0, renamed = 0;
      const io = {
        ...fs,
        writeFileSync(fd, text, options) {
          if (failure === 'write') {
            fs.writeSync(fd, Buffer.from(text).subarray(0, 3));
            reader(destination, oldRecord); failedCalls++; throw sentinel;
          }
          fs.writeFileSync(fd, text, options);
        },
        renameSync() {
          renamed++; reader(destination, oldRecord); failedCalls++; throw sentinel;
        },
      };
      assert.throws(() => publish(destination, newRecord, io), error => error === sentinel);
      assert.equal(failedCalls, 1);
      assert.equal(renamed, failure === 'rename' ? 1 : 0);
      reader(destination, oldRecord);
      assert.deepEqual(fs.readdirSync(root), ['evidence.json']);
    });
  }
  test(`${owner}: refused exclusive staging preserves a foreign file without publication or retries`, t => {
    const root = directory(t), destination = path.join(root, 'evidence.json');
    let temporary, opens = 0;
    const sentinel = Object.assign(new Error('controlled occupied stage'), { code: 'EEXIST' });
    const io = {
      ...fs,
      openSync(candidate, flags) {
        temporary = candidate; opens++;
        assert.equal(flags, 'wx');
        fs.writeFileSync(candidate, 'foreign owner');
        throw sentinel;
      },
    };
    assert.throws(() => publish(destination, newRecord, io), error => error === sentinel);
    assert.equal(opens, 1);
    assert.equal(fs.existsSync(destination), false);
    assert.equal(fs.readFileSync(temporary, 'utf8'), 'foreign owner');
    assert.equal(fs.readdirSync(root).length, 1);
  });
}
test('package-local publishers have identical source so both isolated fixture packages share the tested publication contract', () => {
  assert.equal(fs.readFileSync(fixturePaths[0], 'utf8'), fs.readFileSync(fixturePaths[1], 'utf8'));
});
