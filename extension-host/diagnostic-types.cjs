'use strict';
const { Range } = require('./api-types.cjs');
const { Location } = require('./provider-types.cjs');
const DiagnosticSeverity = Object.freeze({ Error: 0, Warning: 1, Information: 2, Hint: 3 });
const DiagnosticTag = Object.freeze({ Unnecessary: 1, Deprecated: 2 });
class Diagnostic {
  constructor(range, message, severity = DiagnosticSeverity.Error) {
    if (!(range instanceof Range) || typeof message !== 'string' || !Number.isInteger(severity) || severity < 0 || severity > 3) throw new TypeError('Invalid Diagnostic range, message, or severity');
    this.range = range; this.message = message; this.severity = severity;
  }
}
class DiagnosticRelatedInformation {
  constructor(location, message) {
    if (!(location instanceof Location) || typeof message !== 'string') throw new TypeError('Invalid diagnostic related information');
    this.location = location; this.message = message;
  }
}
module.exports = { Diagnostic, DiagnosticSeverity, DiagnosticTag, DiagnosticRelatedInformation };
