'use strict';

// Derived from Microsoft VS Code 1.141.0, commit
// 2a59476c9bfcb90b3ddc372c36762471b7dfad1c:
// extHostDocumentData.ts, mirrorTextModel.ts, wordHelper.ts and strings.ts.
// Copyright (c) Microsoft Corporation. All rights reserved.
// Licensed under the MIT License; see document-types.LICENSE.txt.
// Native Rust remains authoritative for text, encoding and successful saves.

function createDocumentTypes({ Position, Range, Uri }) {
  const usualSeparators = '`~!@#$%^&*()-=+[{]}\\|;:\'",.<>/?';
  let defaultSource = '(-?\\d*\\.\\d\\w*)|([^';
  for (const separator of usualSeparators) defaultSource += '\\' + separator;
  const defaultWord = new RegExp(defaultSource + '\\s]+)', 'g');

  function validWordDefinition(value) {
    let result = defaultWord;
    if (value && value instanceof RegExp) {
      result = value;
      if (!value.global) {
        let flags = 'g';
        if (value.ignoreCase) flags += 'i';
        if (value.multiline) flags += 'm';
        if (value.unicode) flags += 'u';
        result = new RegExp(value.source, flags);
      }
    }
    result.lastIndex = 0;
    return result;
  }

  function endless(regexp) {
    if (['^', '^$', '$', '^\\s*$'].includes(regexp.source)) return false;
    const match = regexp.exec('');
    return !!(match && regexp.lastIndex === 0);
  }

  function wordAt(column, definition, text, textOffset = 0) {
    const regexp = validWordDefinition(definition);
    // This is the upstream 1,000-UTF16-unit window, not a native source scan.
    if (text.length > 1000) {
      let start = column - 500;
      if (start < 0) start = 0;
      else textOffset += start;
      return wordAt(column, regexp, text.substring(start, column + 500), textOffset);
    }
    const startTime = Date.now(), position = column - 1 - textOffset;
    let previous = -1, match = null;
    for (let index = 1; ; index++) {
      if (Date.now() - startTime >= 150) break;
      const regexIndex = position - 15 * index;
      regexp.lastIndex = Math.max(0, regexIndex);
      let found = null, candidate;
      while ((candidate = regexp.exec(text))) {
        const at = candidate.index || 0;
        if (at <= position && regexp.lastIndex >= position) { found = candidate; break; }
        if (previous > 0 && at > previous) break;
        // Native adaptation: a zero-width result outside the caret must not
        // spin forever. Upstream's empty-string probe misses e.g. /(?=x)/g.
        if (candidate[0].length === 0) throw new Error('Word regexp made no progress');
        // Soft budget between calls; cannot preempt one expensive regexp.exec.
        if (Date.now() - startTime >= 150) return null;
      }
      if (!found && match) break;
      match = found;
      if (regexIndex <= 0) break;
      previous = regexIndex;
    }
    if (!match) return null;
    regexp.lastIndex = 0;
    return { start: textOffset + match.index, end: textOffset + match.index + match[0].length };
  }

  class TextLine {
    constructor(line, text, last) { this._line = line; this._text = text; this._last = last; }
    get lineNumber() { return this._line; }
    get text() { return this._text; }
    get range() { return new Range(this._line, 0, this._line, this._text.length); }
    get rangeIncludingLineBreak() {
      return this._last ? this.range : new Range(this._line, 0, this._line + 1, 0);
    }
    get firstNonWhitespaceCharacterIndex() { return /^(\s*)/.exec(this._text)[1].length; }
    get isEmptyOrWhitespace() { return this.firstNonWhitespaceCharacterIndex === this._text.length; }
  }

  class TextDocument {
    constructor(snapshot, hooks = {}) {
      // Hooks belong to the runtime, never extension-controlled snapshot data.
      this._hooks = hooks;
      this._update(snapshot);
    }
    _update(snapshot) {
      const text = Object.hasOwn(snapshot, 'text') ? snapshot.text : this._sourceText;
      if (typeof text !== 'string') throw new Error('A new document requires its text snapshot');
      const merged = { ...this._snapshot, ...snapshot };
      const eol = merged.eol === 2 || merged.eol === '\r\n' ? '\r\n'
        : merged.eol === 1 || merged.eol === '\n' ? '\n'
        : text === this._sourceText ? this._eol : text.includes('\r\n') ? '\r\n' : '\n';
      if (merged.eol !== undefined && ![1, 2, '\n', '\r\n'].includes(merged.eol)) throw new Error('Invalid document EOL snapshot');
      if (merged.encoding !== undefined && typeof merged.encoding !== 'string') throw new Error('Invalid document encoding snapshot');
      // Validate metadata and URI before publishing any update to this identity.
      const uri = merged.uri === this._snapshot?.uri ? this._uri : Uri.parse(merged.uri);
      if (text !== this._sourceText || eol !== this._eol) {
        const lines = text.split(/\r\n|\r|\n/), starts = [];
        let offset = 0;
        for (const line of lines) { starts.push(offset); offset += line.length + eol.length; }
        this._sourceText = text;
        this._lines = lines;
        this._starts = starts;
        this._eol = eol;
        this._text = undefined;
      }
      this._snapshot = merged;
      this._uri = uri;
    }
    get uri() { return this._uri; }
    get fileName() { return this.uri.fsPath; }
    get languageId() { return this._snapshot.languageId; }
    get version() { return this._snapshot.version; }
    get isDirty() { return !this.isClosed && !!this._snapshot.isDirty; }
    get isClosed() { return !!this._snapshot.isClosed; }
    get isUntitled() { return this.uri.scheme === 'untitled'; }
    // Current native file decoding/writing is UTF-8; further encoding support
    // must publish real metadata and use the native persistence pipeline.
    get encoding() { return this._snapshot.encoding ?? 'utf8'; }
    get eol() { return this._eol === '\n' ? 1 : 2; }
    get lineCount() { return this._lines.length; }
    save() {
      if (this.isClosed) return Promise.reject(new Error('Document has been closed'));
      if (typeof this._hooks.save !== 'function') return Promise.reject(new Error('Native document save acknowledgement gateway is not configured'));
      return Promise.resolve().then(() => this._hooks.save(this)).then(value => {
        if (typeof value !== 'boolean') throw new Error('Invalid native document save acknowledgement');
        return value;
      });
    }
    getText(range) {
      if (!range) {
        if (this._text === undefined) this._text = this._lines.join(this._eol);
        return this._text;
      }
      range = this.validateRange(range);
      if (range.isEmpty) return '';
      if (range.isSingleLine) return this._lines[range.start.line].substring(range.start.character, range.end.character);
      const lines = [this._lines[range.start.line].substring(range.start.character)];
      for (let line = range.start.line + 1; line < range.end.line; line++) lines.push(this._lines[line]);
      lines.push(this._lines[range.end.line].substring(0, range.end.character));
      return lines.join(this._eol);
    }
    validatePosition(position) {
      if (!(position instanceof Position)) throw new Error('Invalid argument');
      let { line, character } = position;
      if (line < 0) { line = 0; character = 0; }
      else if (line >= this.lineCount) { line = this.lineCount - 1; character = this._lines[line].length; }
      else if (character < 0) character = 0;
      else if (character > this._lines[line].length) character = this._lines[line].length;
      return line === position.line && character === position.character ? position : new Position(line, character);
    }
    validateRange(range) {
      if (!(range instanceof Range)) throw new Error('Invalid argument');
      const start = this.validatePosition(range.start), end = this.validatePosition(range.end);
      return start === range.start && end === range.end ? range : new Range(start.line, start.character, end.line, end.character);
    }
    offsetAt(position) {
      position = this.validatePosition(position);
      return this._starts[position.line] + position.character;
    }
    positionAt(offset) {
      offset = Math.max(0, Math.floor(offset));
      // Reproduce prefix-sum boundary behavior, including EOL interiors and
      // nonfinite numeric inputs; all wire-coordinate admission is separate.
      let low = 0, high = this._lines.length - 1, middle = 0, start = 0;
      while (low <= high) {
        middle = (low + ((high - low) / 2)) | 0;
        start = this._starts[middle];
        const stop = start + this._lines[middle].length + this._eol.length;
        if (offset < start) high = middle - 1;
        else if (offset >= stop) low = middle + 1;
        else break;
      }
      return new Position(middle, Math.min(offset - start, this._lines[middle].length));
    }
    lineAt(value) {
      const line = value instanceof Position ? value.line : typeof value === 'number' ? value : undefined;
      if (typeof line !== 'number' || line < 0 || line >= this.lineCount || Math.floor(line) !== line) throw new Error('Illegal value for `line`');
      return new TextLine(line, this._lines[line], line === this.lineCount - 1);
    }
    getWordRangeAtPosition(value, regexp) {
      const position = this.validatePosition(value);
      if (!regexp) regexp = this._hooks.wordDefinition?.(this.languageId);
      else if (endless(regexp)) throw new Error(`[getWordRangeAtPosition]: ignoring custom regexp '${regexp.source}' because it matches the empty string.`);
      const result = wordAt(position.character + 1, validWordDefinition(regexp), this._lines[position.line]);
      return result ? new Range(position.line, result.start, position.line, result.end) : undefined;
    }
  }
  return { TextDocument };
}
module.exports = { createDocumentTypes };
