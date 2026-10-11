'use strict';

// Compatibility builder; parsing and native insertion remain separate services.
// Compared with microsoft/vscode 2a59476c9bfcb90b3ddc372c36762471b7dfad1c
// src/vs/workbench/api/common/extHostTypes/snippetString.ts.
class SnippetString {
  constructor(value) { this.value = value || ''; this._tabstop = 1; }
  static isSnippetString(value) {
    return value instanceof SnippetString || !!value && typeof value === 'object' && typeof value.value === 'string';
  }
  static _escape(value) { return value.replace(/[$}\\]/g, '\\$&'); }
  appendText(value) { this.value += SnippetString._escape(value); return this; }
  appendTabstop(number = this._tabstop++) { this.value += `$${number}`; return this; }
  appendPlaceholder(value, number = this._tabstop++) {
    if (typeof value === 'function') {
      const nested = new SnippetString();
      nested._tabstop = this._tabstop;
      value(nested);
      this._tabstop = nested._tabstop;
      value = nested.value;
    } else value = SnippetString._escape(value);
    this.value += `\${${number}:${value}}`;
    return this;
  }
  appendChoice(values, number = this._tabstop++) {
    const value = values.map(item => item.replace(/[|\\,]/g, '\\$&')).join(',');
    this.value += `\${${number}|${value}|}`;
    return this;
  }
  appendVariable(name, defaultValue) {
    if (typeof defaultValue === 'function') {
      const nested = new SnippetString();
      nested._tabstop = this._tabstop;
      defaultValue(nested);
      this._tabstop = nested._tabstop;
      defaultValue = nested.value;
    } else if (typeof defaultValue === 'string') defaultValue = defaultValue.replace(/[$}]/g, '\\$&');
    this.value += `\${${name}${defaultValue ? `:${defaultValue}` : ''}}`;
    return this;
  }
}

module.exports = { SnippetString };
