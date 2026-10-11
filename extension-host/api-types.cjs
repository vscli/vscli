'use strict';
const { es5ClassCompat } = require('./es5-class-compat.cjs');

// Coordinate/disposable overload semantics and event delivery order follow
// Microsoft VS Code 1.95.0 (912bb683695358a54ae0c670461738984cbb5b95)
// and stable 1.141.0 (2a59476c9bfcb90b3ddc372c36762471b7dfad1c):
// src/vs/workbench/api/common/extHostTypes.ts and src/vs/base/common/event.ts.
// Stable coordinates are split into extHostTypes/{position,range,selection}.ts.
// Copyright (c) Microsoft Corporation. All rights reserved.
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

// Compatibility value types. Native Rust remains authoritative for edits.
const { Uri } = require('./uri.cjs');

class Position {
  static Min(...positions) {
    if (!positions.length) throw new TypeError();
    return positions.reduce((result, value) => value.isBefore(result) ? value : result);
  }
  static Max(...positions) {
    if (!positions.length) throw new TypeError();
    return positions.reduce((result, value) => value.isAfter(result) ? value : result);
  }
  static isPosition(value) {
    return value instanceof Position || !!value && typeof value.line === 'number' && typeof value.character === 'number';
  }
  static of(value) {
    if (value instanceof Position) return value;
    if (Position.isPosition(value)) return new Position(value.line, value.character);
    throw new Error('Invalid argument, is NOT a position-like object');
  }
  constructor(line, character) {
    // Extension-facing values follow the upstream constructor. Wire consumers
    // must separately enforce finite integer document coordinates.
    if (line < 0) throw new Error('Illegal argument: line must be non-negative');
    if (character < 0) throw new Error('Illegal argument: character must be non-negative');
    this._line = line;
    this._character = character;
  }
  get line() { return this._line; }
  get character() { return this._character; }
  compareTo(other) {
    if (this._line < other._line) return -1;
    if (this._line > other.line) return 1;
    if (this._character < other._character) return -1;
    if (this._character > other._character) return 1;
    return 0;
  }
  isBefore(other) {
    if (this._line < other._line) return true;
    if (other._line < this._line) return false;
    return this._character < other._character;
  }
  isBeforeOrEqual(other) {
    if (this._line < other._line) return true;
    if (other._line < this._line) return false;
    return this._character <= other._character;
  }
  isAfter(other) { return !this.isBeforeOrEqual(other); }
  isAfterOrEqual(other) { return !this.isBefore(other); }
  isEqual(other) { return this._line === other._line && this._character === other._character; }
  translate(lineDelta = 0, characterDelta = 0) {
    if (lineDelta === null || characterDelta === null) throw new Error('Illegal argument');
    if (typeof lineDelta !== 'number') {
      const change = lineDelta;
      lineDelta = typeof change.lineDelta === 'number' ? change.lineDelta : 0;
      characterDelta = typeof change.characterDelta === 'number' ? change.characterDelta : 0;
    }
    if (lineDelta === 0 && characterDelta === 0) return this;
    return new Position(this.line + lineDelta, this.character + characterDelta);
  }
  with(lineOrChange, character = this.character) {
    if (lineOrChange === null || character === null) throw new Error('Illegal argument');
    let line;
    if (lineOrChange === undefined) line = this.line;
    else if (typeof lineOrChange === 'number') line = lineOrChange;
    else {
      const change = lineOrChange;
      line = typeof change.line === 'number' ? change.line : this.line;
      character = typeof change.character === 'number' ? change.character : this.character;
    }
    if (line === this.line && character === this.character) return this;
    return new Position(line, character);
  }
  toJSON() { return { line: this.line, character: this.character }; }
}

class Range {
  static isRange(value) {
    return value instanceof Range || !!value && typeof value === 'object'
      && Position.isPosition(value.start) && Position.isPosition(value.end);
  }
  static of(value) {
    if (value instanceof Range) return value;
    if (Range.isRange(value)) return new Range(value.start, value.end);
    throw new Error('Invalid argument, is NOT a range-like object');
  }
  constructor(start, end, endLine, endCharacter) {
    if ([start, end, endLine, endCharacter].every(value => typeof value === 'number')) {
      start = new Position(start, end);
      end = new Position(endLine, endCharacter);
    } else if (Position.isPosition(start) && Position.isPosition(end)) {
      start = Position.of(start);
      end = Position.of(end);
    } else {
      throw new Error('Invalid arguments');
    }
    if (!start.isBefore(end)) [start, end] = [end, start];
    this._start = start;
    this._end = end;
  }
  get start() { return this._start; }
  get end() { return this._end; }
  get isEmpty() { return this.start.isEqual(this.end); }
  get isSingleLine() { return this.start.line === this.end.line; }
  contains(other) {
    if (Range.isRange(other)) return this.contains(other.start) && this.contains(other.end);
    if (Position.isPosition(other)) {
      if (Position.of(other).isBefore(this.start)) return false;
      if (this.end.isBefore(other)) return false;
      return true;
    }
    return false;
  }
  isEqual(other) { return this.start.isEqual(other._start) && this.end.isEqual(other._end); }
  intersection(other) {
    const start = this.start.isAfter(other.start) ? this.start : other.start;
    const end = this.end.isBefore(other.end) ? this.end : other.end;
    return start.isAfter(end) ? undefined : new Range(start, end);
  }
  union(other) {
    if (this.contains(other)) return this;
    if (other.contains(this)) return other;
    return new Range(this.start.isBefore(other.start) ? this.start : other.start,
      this.end.isAfter(other.end) ? this.end : other.end);
  }
  with(start = this.start, end = this.end) {
    if (start === null || end === null) throw new Error('Illegal argument');
    if (!start) start = this.start;
    else if (!Position.isPosition(start)) {
      const change = start;
      start = change.start || this.start;
      end = change.end || this.end;
    }
    if (start.isEqual(this.start) && end.isEqual(this.end)) return this;
    return new Range(start, end);
  }
  toJSON() { return [this.start, this.end]; }
}

class Selection extends Range {
  static isSelection(value) {
    return value instanceof Selection || !!value && typeof value === 'object'
      && Range.isRange(value) && Position.isPosition(value.anchor)
      && Position.isPosition(value.active) && typeof value.isReversed === 'boolean';
  }
  constructor(anchor, active, activeLine, activeCharacter) {
    if ([anchor, active, activeLine, activeCharacter].every(value => typeof value === 'number')) {
      anchor = new Position(anchor, active);
      active = new Position(activeLine, activeCharacter);
    } else if (Position.isPosition(anchor) && Position.isPosition(active)) {
      anchor = Position.of(anchor);
      active = Position.of(active);
    } else {
      throw new Error('Invalid arguments');
    }
    super(anchor, active);
    this._anchor = anchor;
    this._active = active;
  }
  get anchor() { return this._anchor; }
  get active() { return this._active; }
  get isReversed() { return this.anchor === this.end; }
  toJSON() { return { start: this.start, end: this.end, active: this.active, anchor: this.anchor }; }
}

class Disposable {
  #call;
  constructor(call) { this.#call = call; }
  dispose() {
    if (typeof this.#call === 'function') {
      this.#call();
      this.#call = undefined;
    }
  }
  static from(...items) {
    let retained = items;
    return new Disposable(() => {
      if (!retained) return;
      for (const item of retained) {
        if (item && typeof item.dispose === 'function') item.dispose();
      }
      retained = undefined;
    });
  }
}

class EventEmitter {
  constructor(onError = error => console.error(error)) {
    this._listeners = new Set();
    this._onError = typeof onError === 'function' ? onError
      : onError?.onListenerError || (error => console.error(error));
    this._disposed = false;
    this._delivery = undefined;
    this.event = (listener, thisArg, disposables) => {
      if (this._disposed) return new Disposable(() => {});
      const entry = { listener, thisArg: thisArg || undefined };
      this._listeners.add(entry);
      const disposable = new Disposable(() => this._listeners.delete(entry));
      if (Array.isArray(disposables)) disposables.push(disposable);
      return disposable;
    };
  }
  _drain() {
    const delivery = this._delivery;
    if (!delivery) return;
    while (this._delivery === delivery && delivery.index < delivery.entries.length) {
      const entry = delivery.entries[delivery.index++];
      if (!this._listeners.has(entry)) continue;
      try { entry.listener.call(entry.thisArg, delivery.value); }
      catch (error) { this._onError(error); }
    }
    if (this._delivery === delivery) this._delivery = undefined;
  }
  fire(value) {
    // Complete the earlier emission before dispatching a reentrant one.
    this._drain();
    if (this._disposed || !this._listeners.size) return;
    this._delivery = { entries: [...this._listeners], index: 0, value };
    this._drain();
  }
  dispose() {
    this._disposed = true;
    this._listeners.clear();
    this._delivery = undefined;
  }
}

// Preserve only the classes decorated by pinned upstream.
for (const value of [Position, Range, Selection, Disposable]) es5ClassCompat(value);

const { TextDocument } = require('./document-types.cjs')
  .createDocumentTypes({ Position, Range, Uri });

module.exports = { Position, Range, Selection, Uri, Disposable, EventEmitter, TextDocument };
