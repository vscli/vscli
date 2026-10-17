'use strict';
const { bindCurrentListener } = require('./caller-context.cjs');

// Source-guided adaptation of VS Code 1.95's cancellation lifecycle (MIT).
// Copyright (c) Microsoft Corporation. All rights reserved.
// Primary commit: 912bb683695358a54ae0c670461738984cbb5b95
// https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/base/common/cancellation.ts
/*
MIT License

Copyright (c) 2015 - present Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/

const noopDisposable = Object.freeze({ dispose() {} });
function disposable(call) {
  return {
    dispose() {
      const current = call;
      call = undefined;
      if (current) current();
    },
  };
}
const noneEvent = () => noopDisposable;
// Match the pinned shortcut event: scheduled on the next timer turn, bound
// receiver, individually disposable, and no registration in the third argument.
const canceledEvent = Object.freeze(function (listener, thisArgs) {
  const timer = setTimeout(bindCurrentListener(listener).bind(thisArgs), 0);
  return disposable(() => clearTimeout(timer));
});

// Cancellation is single-shot. A Set iteration avoids a second listener cohort
// allocation and observes removal/disposal during synchronous delivery. The
// sequence boundary excludes subscribers added through a cached live event
// during fire, as the pinned emitter's fixed delivery-queue end does.
class CancellationSignal {
  constructor() {
    this.listeners = new Set();
    this.next = 0;
    this.disposed = false;
    this.event = (listener, thisArgs, disposables) => {
      if (this.disposed) return noopDisposable;
      const bound = bindCurrentListener(listener);
      const entry = { listener: thisArgs ? bound.bind(thisArgs) : bound, sequence: ++this.next };
      this.listeners.add(entry);
      const result = disposable(() => {
        this.listeners.delete(entry);
        entry.listener = undefined;
      });
      if (Array.isArray(disposables)) disposables.push(result);
      return result;
    };
  }
  fire() {
    const end = this.next;
    for (const entry of this.listeners) {
      if (this.disposed || entry.sequence > end) break;
      try { entry.listener(undefined); }
      catch (error) {
        // VS Code uses its onUnexpectedError handler; the isolated compatibility
        // process uses the existing host convention, reporting through stderr.
        console.error(error);
      }
    }
  }
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    for (const entry of this.listeners) entry.listener = undefined;
    this.listeners.clear();
  }
}

class MutableToken {
  constructor() {
    this._isCancelled = false;
    this._emitter = null;
  }
  get isCancellationRequested() { return this._isCancelled; }
  get onCancellationRequested() {
    if (this._isCancelled) return canceledEvent;
    if (!this._emitter) this._emitter = new CancellationSignal();
    return this._emitter.event;
  }
  cancel() {
    if (this._isCancelled) return;
    this._isCancelled = true;
    if (this._emitter) {
      this._emitter.fire();
      this.dispose();
    }
  }
  dispose() {
    if (this._emitter) {
      this._emitter.dispose();
      this._emitter = null;
    }
  }
}

// Internal structural helpers. CancellationToken is an interface in vscode.d.ts,
// not a public vscode namespace/constructor; only export the source on api.cjs.
const none = Object.freeze({ isCancellationRequested: false, onCancellationRequested: noneEvent });
const canceled = Object.freeze({ isCancellationRequested: true, onCancellationRequested: canceledEvent });
const CancellationToken = Object.freeze({
  None: none,
  Cancelled: canceled,
  isCancellationToken(value) {
    if (value === none || value === canceled || value instanceof MutableToken) return true;
    return !!value && typeof value === 'object'
      && typeof value.isCancellationRequested === 'boolean'
      && typeof value.onCancellationRequested === 'function';
  },
});

class CancellationTokenSource {
  constructor(parent) {
    this._token = undefined;
    this._parentListener = parent && parent.onCancellationRequested(this.cancel, this);
  }
  get token() {
    if (!this._token) this._token = new MutableToken();
    return this._token;
  }
  cancel() {
    if (!this._token) this._token = canceled;
    else if (this._token instanceof MutableToken) this._token.cancel();
  }
  dispose(cancel = false) {
    if (cancel) this.cancel();
    this._parentListener?.dispose();
    if (!this._token) this._token = none;
    else if (this._token instanceof MutableToken) this._token.dispose();
  }
}

module.exports = { CancellationTokenSource, CancellationToken };
