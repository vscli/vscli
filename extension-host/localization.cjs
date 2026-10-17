'use strict';
/*
 * Localization formatting adapted from Microsoft VS Code at
 * 2a59476c9bfcb90b3ddc372c36762471b7dfad1c:
 * extHostLocalizationService.ts, extHost.api.impl.ts and base/common/strings.ts.
 * Copyright (c) Microsoft Corporation. MIT License.
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */
const fs = require('node:fs');
const path = require('node:path');
const { Uri } = require('./uri.cjs');
const MAX_FILE = 1024 * 1024, MAX_TOTAL = 8 * MAX_FILE;
function language(value) {
  if (typeof value !== 'string' || value.length > 64 || !/^[a-zA-Z]{2,8}(?:-[a-zA-Z0-9]{1,8})*$/.test(value)) throw new Error('Invalid preferred extension language');
  return value;
}
function format2(template, values) {
  if (Object.keys(values).length === 0) return template;
  return template.replace(/{([^}]+)}/g, (match, group) => values[group] ?? match);
}
function readBundle(file) {
  if (!fs.lstatSync(file).isFile()) throw new Error('Localization bundle must be a regular file');
  const fd = fs.openSync(file, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW || 0) | (fs.constants.O_NONBLOCK || 0));
  try {
    const before = fs.fstatSync(fd);
    if (!before.isFile() || before.size > MAX_FILE) throw new Error('Localization bundle exceeds 1 MiB or is not a regular file');
    const buffer = Buffer.alloc(MAX_FILE + 1);
    let size = 0, count;
    while (size < buffer.length && (count = fs.readSync(fd, buffer, size, buffer.length-size, null))) size += count;
    const after = fs.fstatSync(fd);
    if (size > MAX_FILE || size !== before.size || before.size !== after.size || before.mtimeMs !== after.mtimeMs) throw new Error('Localization bundle changed or exceeds 1 MiB');
    const value = JSON.parse(buffer.toString('utf8', 0, size));
    if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).length > 16384 || Object.values(value).some(item => typeof item !== 'string')) throw new Error('Localization bundle requires at most 16384 string translations');
    return { contents: value, size };
  } finally { fs.closeSync(fd); }
}
function createLocalization(locale = 'en', diagnostic = () => {}) {
  locale = language(locale);
  const bundles = new Map(), selected = new Set();
  let bytes = 0;
  function add(packages) {
    for (const item of packages) {
      if (selected.has(item.id)) continue;
      if (selected.size >= 128) throw new Error('Localization selected package limit exceeded');
      selected.add(item.id);
      if (locale === 'en' || !item.manifest.l10n) continue;
      try {
        const relative = item.manifest.l10n;
        if (typeof relative !== 'string' || path.isAbsolute(relative) || relative.includes('\0')) throw new Error('Localization folder must be relative');
        const root = fs.realpathSync(item.folder);
        const file = fs.realpathSync(path.join(root, relative, 'bundle.l10n.' + locale + '.json'));
        const child = path.relative(root, file);
        if (child === '..' || child.startsWith('..' + path.sep) || path.isAbsolute(child)) throw new Error('Localization bundle escapes its selected package');
        const bundle = readBundle(file);
        if (bundle.size > MAX_TOTAL - bytes) throw new Error('Localization bundles exceed 8 MiB for this session');
        bytes += bundle.size;
        bundles.set(item.id, { contents: bundle.contents, uri: Uri.file(file) });
      } catch (error) {
        diagnostic(item.id, 'Localization unavailable; using original messages: ' + String(error.message || error).slice(0, 1024));
      }
    }
  }
  function message(owner, details) {
    const { message, args, comment } = details;
    if (locale === 'en') return format2(message, args ?? {});
    let key = message;
    if (comment && comment.length > 0) key += '/' + (Array.isArray(comment) ? comment.join('') : comment);
    const value = bundles.get(owner)?.contents[key];
    return format2(value ?? message, args ?? {});
  }
  function forOwner(owner) {
    return {
      t(...params) {
        if (typeof params[0] === 'string') {
          const key = params.shift();
          const args = typeof params[0] !== 'object' ? params : params[0];
          return message(owner, { message: key, args });
        }
        return message(owner, params[0]);
      },
      get bundle() { return bundles.get(owner)?.contents; },
      get uri() { return bundles.get(owner)?.uri; },
    };
  }
  return { add, forOwner };
}
module.exports = { createLocalization, language };
