'use strict';

// Public API objects follow upstream JavaScript semantics. Native wire authority
// is deliberately narrower: finite LSP uintegers and explicit protocol objects.
function position(value, description = 'Native') {
  const line = value?.line, character = value?.character;
  if (!Number.isInteger(line) || !Number.isInteger(character)
      || line < 0 || character < 0 || line > 0x7fffffff || character > 0x7fffffff) {
    throw new RangeError(`${description} coordinates require nonnegative LSP uintegers`);
  }
  return { line, character };
}
function range(value, description = 'Native') {
  const start = position(value?.start, description), end = position(value?.end, description);
  if (start.line > end.line || start.line === end.line && start.character > end.character) {
    throw new RangeError(`${description} range is reversed`);
  }
  return { start, end };
}

module.exports = { position, range };
