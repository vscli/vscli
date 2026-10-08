'use strict';

// Original compatibility types. Native Rust remains authoritative for edits.
const { fileURLToPath, pathToFileURL } = require('node:url');
const path = require('node:path');

class Position {
  constructor(line, character) {
    if (!Number.isInteger(line) || !Number.isInteger(character) || line < 0 || character < 0) {
      throw new RangeError('Position requires nonnegative integer coordinates');
    }
    this.line = line;
    this.character = character;
    Object.freeze(this);
  }
  compareTo(other) { return Math.sign(this.line - other.line || this.character - other.character); }
  isBefore(other) { return this.compareTo(other) < 0; }
  isBeforeOrEqual(other) { return this.compareTo(other) <= 0; }
  isAfter(other) { return this.compareTo(other) > 0; }
  isAfterOrEqual(other) { return this.compareTo(other) >= 0; }
  isEqual(other) { return this.compareTo(other) === 0; }
  translate(lineDelta = 0, characterDelta = 0) {
    if (typeof lineDelta === 'object') ({ lineDelta = 0, characterDelta = 0 } = lineDelta);
    return new Position(this.line + lineDelta, this.character + characterDelta);
  }
  with(line = this.line, character = this.character) {
    if (typeof line === 'object') ({ line = this.line, character = this.character } = line);
    return new Position(line, character);
  }
}

class Range {
  constructor(start, end, endLine, endCharacter) {
    if (typeof start === 'number') {
      start = new Position(start, end);
      end = new Position(endLine, endCharacter);
    } else {
      start = new Position(start.line, start.character);
      end = new Position(end.line, end.character);
    }
    if (start.isAfter(end)) [start, end] = [end, start];
    this.start = start;
    this.end = end;
  }
  get isEmpty() { return this.start.isEqual(this.end); }
  get isSingleLine() { return this.start.line === this.end.line; }
  contains(other) {
    return other.start ? this.contains(other.start) && this.contains(other.end)
      : this.start.isBeforeOrEqual(other) && this.end.isAfterOrEqual(other);
  }
  isEqual(other) { return this.start.isEqual(other.start) && this.end.isEqual(other.end); }
  intersection(other) {
    const start = this.start.isAfter(other.start) ? this.start : other.start;
    const end = this.end.isBefore(other.end) ? this.end : other.end;
    return start.isAfter(end) ? undefined : new Range(start, end);
  }
  union(other) {
    return new Range(this.start.isBefore(other.start) ? this.start : other.start,
      this.end.isAfter(other.end) ? this.end : other.end);
  }
  with(start = this.start, end = this.end) {
    if (start.start || start.end) ({ start = this.start, end = this.end } = start);
    return new Range(start, end);
  }
}

class Selection extends Range {
  constructor(anchor, active, activeLine, activeCharacter) {
    if (typeof anchor === 'number') {
      anchor = new Position(anchor, active);
      active = new Position(activeLine, activeCharacter);
    }
    super(anchor, active);
    this.anchor = new Position(anchor.line, anchor.character);
    this.active = new Position(active.line, active.character);
  }
  get isReversed() { return this.anchor.isAfter(this.active); }
}

class Uri {
  constructor(value) { this._url = new URL(value); }
  static parse(value) { return new Uri(value); }
  static file(value) { return new Uri(pathToFileURL(path.resolve(value))); }
  static from(parts) {
    const authority = parts.authority || '';
    const encodedPath = (parts.path || '').split('/').map(encodeURIComponent).join('/');
    const uri = new Uri(`${parts.scheme}:${authority || parts.scheme === 'file' ? `//${authority}` : ''}${encodedPath}`);
    return uri.with({ query: parts.query || '', fragment: parts.fragment || '' });
  }
  static joinPath(base, ...segments) { return base.with({ path: path.posix.join(base.path, ...segments) }); }
  get scheme() { return this._url.protocol.slice(0, -1); }
  get authority() { return this._url.host; }
  get path() { return decodeURIComponent(this._url.pathname); }
  get query() { return this._url.search.slice(1); }
  get fragment() { return decodeURIComponent(this._url.hash.slice(1)); }
  get fsPath() { return this.scheme === 'file' ? fileURLToPath(this._url) : this.path; }
  with(parts) {
    const result = new Uri(this.toString());
    if (parts.scheme !== undefined) result._url.protocol = `${parts.scheme}:`;
    if (parts.authority !== undefined) result._url.host = parts.authority;
    if (parts.path !== undefined) result._url.pathname = parts.path.split('/').map(encodeURIComponent).join('/');
    if (parts.query !== undefined) result._url.search = parts.query;
    if (parts.fragment !== undefined) result._url.hash = parts.fragment;
    return result;
  }
  toString(skipEncoding = false) { return skipEncoding ? decodeURI(this._url.href) : this._url.href; }
  toJSON() { return { scheme: this.scheme, authority: this.authority, path: this.path, query: this.query, fragment: this.fragment }; }
}

class Disposable {
  constructor(call) { this._call = call; }
  dispose() { const call = this._call; this._call = undefined; if (call) call(); }
  static from(...items) { return new Disposable(() => items.forEach(item => item.dispose())); }
}

class EventEmitter {
  constructor(onError = error => console.error(error)) {
    this._listeners = new Set();
    this._onError = onError;
    this.event = (listener, thisArg, disposables) => {
      const entry = { listener, thisArg };
      this._listeners.add(entry);
      const disposable = new Disposable(() => this._listeners.delete(entry));
      if (disposables) disposables.push(disposable);
      return disposable;
    };
  }
  fire(value) {
    for (const { listener, thisArg } of [...this._listeners]) {
      try { listener.call(thisArg, value); } catch (error) { this._onError(error); }
    }
  }
  dispose() { this._listeners.clear(); }
}

class TextDocument {
  constructor(snapshot) { this._update(snapshot); }
  _update(snapshot) {
    this._snapshot = snapshot;
    this._text = snapshot.text;
    this._lines = this._text.split(/\r\n|\r|\n/);
    this._starts = [0];
    for (const match of this._text.matchAll(/\r\n|\r|\n/g)) this._starts.push(match.index + match[0].length);
    this._uri = Uri.parse(snapshot.uri);
  }
  get uri() { return this._uri; }
  get fileName() { return this.uri.fsPath; }
  get languageId() { return this._snapshot.languageId; }
  get version() { return this._snapshot.version; }
  get isDirty() { return this._snapshot.isDirty; }
  get isClosed() { return !!this._snapshot.isClosed; }
  get isUntitled() { return this.uri.scheme === 'untitled'; }
  get eol() { return this._text.includes('\r\n') ? 2 : 1; }
  get lineCount() { return this._lines.length; }
  getText(range) {
    if (!range) return this._text;
    range = this.validateRange(range);
    return this._text.slice(this.offsetAt(range.start), this.offsetAt(range.end));
  }
  validatePosition(position) {
    const line = Math.max(0, Math.min(position.line, this.lineCount - 1));
    return new Position(line, Math.max(0, Math.min(position.character, this._lines[line].length)));
  }
  validateRange(range) { return new Range(this.validatePosition(range.start), this.validatePosition(range.end)); }
  offsetAt(position) {
    position = this.validatePosition(position);
    return this._starts[position.line] + position.character;
  }
  positionAt(offset) {
    offset = Math.max(0, Math.min(Math.floor(offset), this._text.length));
    let low = 0, high = this._starts.length;
    while (low + 1 < high) {
      const middle = (low + high) >>> 1;
      if (this._starts[middle] > offset) high = middle;
      else low = middle;
    }
    return new Position(low, Math.min(offset - this._starts[low], this._lines[low].length));
  }
  lineAt(position) {
    const line = typeof position === 'number' ? position : position.line;
    if (!Number.isInteger(line) || line < 0 || line >= this.lineCount) throw new RangeError('Invalid line number');
    const text = this._lines[line];
    const range = new Range(line, 0, line, text.length);
    const index = text.search(/\S/);
    return { lineNumber: line, text, range,
      rangeIncludingLineBreak: line + 1 < this.lineCount ? new Range(line, 0, line + 1, 0) : range,
      firstNonWhitespaceCharacterIndex: index < 0 ? text.length : index,
      isEmptyOrWhitespace: index < 0 };
  }
}

module.exports = { Position, Range, Selection, Uri, Disposable, EventEmitter, TextDocument };
