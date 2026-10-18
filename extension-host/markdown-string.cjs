'use strict';
// Source-derived value semantics from Microsoft VS Code stable 1.141.0,
// commit 2a59476c9bfcb90b3ddc372c36762471b7dfad1c:
// extHostTypes/markdownString.ts, base/common/htmlContent.ts and iconLabels.ts.
// Copyright (c) Microsoft Corporation. All rights reserved.
// MIT License
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
const { es5ClassCompat } = require('./es5-class-compat.cjs');
const iconsRegex = /\$\([A-Za-z0-9-]+(?:~[A-Za-z]+)?\)/g;
const escapedIconsRegex = new RegExp(`(\\\\)?${iconsRegex.source}`, 'g');
function escapeIcons(text) {
  return text.replace(escapedIconsRegex, (match, escaped) => escaped ? match : `\\${match}`);
}
function escapeMarkdownSyntaxTokens(text) {
  return text.replace(/[\\`*_{}[\]()#+!~]/g, '\\$&').replace(/^([ \t]*)-/gm, '$1\\-');
}
function appendEscapedMarkdownCodeBlockFence(code, language) {
  const longestFenceLength = code.match(/^`+/gm)?.reduce((a,b) => a.length > b.length ? a : b).length ?? 0;
  const desiredFenceLength = longestFenceLength >= 3 ? longestFenceLength + 1 : 3;
  return [`${'`'.repeat(desiredFenceLength)}${language}`, code, `${'`'.repeat(desiredFenceLength)}`].join('\n');
}
// The extension class delegates directly; subclasses' public accessors cannot
// redirect its private value mutations. Public values have no native wire cap.
class BaseMarkdownString {
  constructor(value = '', options = false) {
    this.value = value;
    if (typeof this.value !== 'string') throw new Error('Illegal argument: value');
    if (typeof options === 'boolean') {
      this.isTrusted = options; this.supportThemeIcons = false; this.supportHtml = false; this.supportAlertSyntax = false;
    } else {
      this.isTrusted = options.isTrusted ?? undefined;
      this.supportThemeIcons = options.supportThemeIcons ?? false;
      this.supportHtml = options.supportHtml ?? false;
      this.supportAlertSyntax = options.supportAlertSyntax ?? false;
    }
  }
  appendText(value) {
    this.value += escapeMarkdownSyntaxTokens(this.supportThemeIcons ? escapeIcons(value) : value)
      .replace(/([ \t]+)/g, (_match,g1) => '&nbsp;'.repeat(g1.length))
      .replace(/\>/gm, '\\>')
      .replace(/\n/g, '\n\n');
    return this;
  }
  appendMarkdown(value) { this.value += value; return this; }
  appendCodeblock(language, code) { this.value += `\n${appendEscapedMarkdownCodeBlockFence(code,language)}\n`; return this; }
}
class MarkdownString {
  #delegate;
  static isMarkdownString(thing) {
    if (thing instanceof MarkdownString) return true;
    if (!thing || typeof thing !== 'object') return false;
    return thing.appendCodeblock && thing.appendMarkdown && thing.appendText && thing.value !== undefined;
  }
  constructor(value, supportThemeIcons = false) {
    this.#delegate = new BaseMarkdownString(value, {supportThemeIcons});
  }
  get value() { return this.#delegate.value; }
  set value(value) { this.#delegate.value = value; }
  get isTrusted() { return this.#delegate.isTrusted; }
  set isTrusted(value) { this.#delegate.isTrusted = value; }
  get supportThemeIcons() { return this.#delegate.supportThemeIcons; }
  set supportThemeIcons(value) { this.#delegate.supportThemeIcons = value; }
  get supportHtml() { return this.#delegate.supportHtml; }
  set supportHtml(value) { this.#delegate.supportHtml = value; }
  get supportAlertSyntax() { return this.#delegate.supportAlertSyntax; }
  set supportAlertSyntax(value) { this.#delegate.supportAlertSyntax = value; }
  get baseUri() { return this.#delegate.baseUri; }
  set baseUri(value) { this.#delegate.baseUri = value; }
  appendText(value) { this.#delegate.appendText(value); return this; }
  appendMarkdown(value) { this.#delegate.appendMarkdown(value); return this; }
  appendCodeblock(value, language) { this.#delegate.appendCodeblock(language ?? '',value); return this; }
}
es5ClassCompat(MarkdownString);
module.exports = { MarkdownString };
