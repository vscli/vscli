'use strict';
// Fixture evidence: existence of the final path proves a complete publication.
// Each fixture is an isolated extension package, so keep this helper local.
const fs = require('node:fs');
const path = require('node:path');
let sequence = 0;
module.exports = function publishEvidence(destination, text, io = fs) {
  if (typeof text !== 'string' || Buffer.byteLength(text, 'utf8') > 64 * 1024) {
    throw new RangeError('Fixture evidence must be a string of at most 64 KiB');
  }
  if (!Number.isSafeInteger(sequence + 1)) throw new RangeError('Fixture evidence sequence exhausted');
  const temporary = path.join(path.dirname(destination),
    `.${path.basename(destination)}.fixture-${process.pid}-${++sequence}.tmp`);
  let descriptor, owned = false;
  try {
    descriptor = io.openSync(temporary, 'wx', 0o600);
    owned = true;
    io.writeFileSync(descriptor, text, { encoding: 'utf8' });
    io.closeSync(descriptor);
    descriptor = undefined;
    // Never remove or truncate an existing final record before replacement.
    io.renameSync(temporary, destination);
    owned = false;
  } finally {
    if (descriptor !== undefined) {
      try { io.closeSync(descriptor); } catch { /* Preserve the original failure. */ }
    }
    if (owned) {
      try { io.unlinkSync(temporary); } catch { /* Preserve the original failure. */ }
    }
  }
};
