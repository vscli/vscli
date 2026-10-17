'use strict';
/*---------------------------------------------------------------------------------------------
 * Copyright (c) Microsoft Corporation. All rights reserved.
 * Licensed under the MIT License. See https://github.com/microsoft/vscode/blob/
 * 2a59476c9bfcb90b3ddc372c36762471b7dfad1c/LICENSE.txt.
 *--------------------------------------------------------------------------------------------*/
// String-pattern adaptation of VS Code's glob.ts, stable 1.141.0:
// https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/base/common/glob.ts
// starsToRegExp, splitGlobAware, parseRegExp and the five string fast paths
// preserve source matching, including its malformed-pattern behavior. Relative
// bases, expression/sibling clauses and traversal-only exclusions belong to the
// caller. No process-wide pattern cache or filesystem access is introduced.
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
const { sep } = require('node:path');
const PATH_REGEX = '[/\\\\]', NO_PATH_REGEX = '[^/\\\\]';
const MAX_PATTERN_BYTES = 4096, MAX_REGEX_LENGTH = 65536;
const MAX_PARSE_WORK = 65536, MAX_EXPANSIONS = 512, MAX_DEPTH = 32;
class GlobLimitError extends RangeError {}
const NEVER = () => false;
function escapeRegExpCharacters(value) {
  return value.replace(/[\\\{\}\*\+\?\|\^\$\.\[\]\(\)]/g, '\\$&');
}
// Preserve strings.ts UTF-16 substring bounds when lowercasing expands a code
// unit (e.g. capital dotted I); lowering both whole strings and comparing them
// would differ from the pinned basename/path optimizations.
function compareSubstring(a, b, aStart, aEnd, bStart, bEnd) {
  for (; aStart < aEnd && bStart < bEnd; aStart++, bStart++) {
    const left = a.charCodeAt(aStart), right = b.charCodeAt(bStart);
    if (left < right) return -1;
    if (left > right) return 1;
  }
  return Math.sign((aEnd - aStart) - (bEnd - bStart));
}
function compareSubstringIgnoreCase(a, b, aStart = 0, aEnd = a.length, bStart = 0, bEnd = b.length) {
  for (; aStart < aEnd && bStart < bEnd; aStart++, bStart++) {
    let left = a.charCodeAt(aStart), right = b.charCodeAt(bStart);
    if (left === right) continue;
    if (left >= 128 || right >= 128) return compareSubstring(a.toLowerCase(), b.toLowerCase(), aStart, aEnd, bStart, bEnd);
    if (left >= 97 && left <= 122) left -= 32;
    if (right >= 97 && right <= 122) right -= 32;
    if (left !== right) return left - right;
  }
  return Math.sign((aEnd - aStart) - (bEnd - bStart));
}
function equalsIgnoreCase(a, b) { return a.length === b.length && compareSubstringIgnoreCase(a, b) === 0; }
function endsWithIgnoreCase(value, candidate) {
  const start = value.length - candidate.length;
  return start >= 0 && compareSubstringIgnoreCase(value, candidate, start, value.length) === 0;
}
function starsToRegExp(count, last) {
  if (count === 0) return '';
  if (count === 1) return `${NO_PATH_REGEX}*?`;
  return `(?:${PATH_REGEX}|${NO_PATH_REGEX}+${PATH_REGEX}${last ? `|${PATH_REGEX}${NO_PATH_REGEX}+` : ''})*?`;
}
function splitGlobAware(pattern, separator) {
  if (!pattern) return [];
  const segments = [];
  let inBraces = false, inBrackets = false, current = '';
  for (const char of pattern) {
    if (char === separator) {
      if (!inBraces && !inBrackets) { segments.push(current); current = ''; continue; }
    } else if (char === '{') inBraces = true;
    else if (char === '}') inBraces = false;
    else if (char === '[') inBrackets = true;
    else if (char === ']') inBrackets = false;
    current += char;
  }
  if (current) segments.push(current);
  return segments;
}
function chargeParse(pattern, budget, depth) {
  if (depth > MAX_DEPTH || ++budget.expansions > MAX_EXPANSIONS || pattern.length > MAX_PARSE_WORK - budget.work)
    throw new GlobLimitError('Watcher glob expansion/work limit exceeded');
  budget.work += pattern.length;
}
function parseRegExp(pattern, budget, depth) {
  chargeParse(pattern, budget, depth);
  if (!pattern) return '';
  const segments = splitGlobAware(pattern, '/');
  if (segments.every(segment => segment === '**')) return '.*';
  let regex = '', previousGlobStar = false;
  function append(value) {
    if (value.length > MAX_REGEX_LENGTH - regex.length) throw new GlobLimitError('Watcher glob compiled size limit exceeded');
    regex += value;
  }
  segments.forEach((segment, index) => {
    if (segment === '**') {
      if (previousGlobStar) return;
      append(starsToRegExp(2, index === segments.length - 1));
    } else {
      let inBraces = false, braceValue = '', inBrackets = false, bracketValue = '';
      for (const char of segment) {
        if (char !== '}' && inBraces) { braceValue += char; continue; }
        if (inBrackets && (char !== ']' || !bracketValue)) {
          let value;
          if (char === '-') value = char;
          else if ((char === '^' || char === '!') && !bracketValue) value = '^';
          else if (char === '/') value = '';
          else value = escapeRegExpCharacters(char);
          bracketValue += value;
          continue;
        }
        switch (char) {
          case '{': inBraces = true; continue;
          case '[': inBrackets = true; continue;
          case '}': {
            const choices = splitGlobAware(braceValue, ',');
            append(`(?:${choices.map(choice => parseRegExp(choice, budget, depth + 1)).join('|')})`);
            inBraces = false; braceValue = ''; break;
          }
          case ']': append('[' + bracketValue + ']'); inBrackets = false; bracketValue = ''; break;
          case '?': append(NO_PATH_REGEX); continue;
          case '*': append(starsToRegExp(1)); continue;
          default: append(escapeRegExpCharacters(char));
        }
      }
      if (index < segments.length - 1 && (segments[index + 1] !== '**' || index + 2 < segments.length)) append(PATH_REGEX);
    }
    previousGlobStar = segment === '**';
  });
  return regex;
}
const T1 = /^\*\*\/\*\.[\w\.-]+$/;
const T2 = /^\*\*\/([\w\.-]+)\/?$/;
const T3 = /^{\*\*\/\*?[\w\.-]+\/?(,\*\*\/\*?[\w\.-]+\/?)*}$/;
const T4 = /^\*\*((\/[\w\.-]+)+)\/?$/;
const T5 = /^([\w\.-]+(\/[\w\.-]+)*)\/?$/;
function parsePattern(pattern, options, budget, depth) {
  chargeParse(pattern, budget, depth);
  if (!pattern) return NEVER;
  pattern = pattern.trim();
  const equals = options.ignoreCase ? equalsIgnoreCase : (a, b) => a === b;
  const endsWith = options.ignoreCase ? endsWithIgnoreCase : (a, b) => a.endsWith(b);
  let match;
  if (T1.test(pattern)) {
    const suffix = pattern.substring(4);
    return path => endsWith(path, suffix);
  }
  if ((match = T2.exec(pattern))) {
    const base = match[1];
    return path => equals(path, base) || endsWith(path, '/' + base) || endsWith(path, '\\' + base);
  }
  if (T3.test(pattern)) {
    // The upstream aggregate also retains basename metadata for callers. A
    // boolean-only matcher needs only the identical bounded union of children.
    const children = pattern.slice(1, -1).split(',').map(value => parsePattern(value, options, budget, depth + 1));
    return path => children.some(child => child(path));
  }
  const tail = T4.exec(pattern);
  if (tail || (match = T5.exec(pattern))) {
    const target = tail ? tail[1].substring(1) : match[1];
    const usingPosix = sep === '/', native = usingPosix ? target : target.replace(/\//g, sep);
    if (tail) return path => equals(path, native) || endsWith(path, sep + native) || !usingPosix && (equals(path, target) || endsWith(path, '/' + target));
    return path => equals(path, native) || !usingPosix && equals(path, target);
  }
  let regex;
  try { regex = new RegExp(`^${parseRegExp(pattern, budget, depth + 1)}$`, options.ignoreCase ? 'i' : undefined); }
  catch (error) { if (error instanceof GlobLimitError) throw error; return NEVER; }
  return path => { regex.lastIndex = 0; return regex.test(path); };
}
function compileGlob(pattern, {ignoreCase = false} = {}) {
  if (typeof pattern !== 'string') throw new TypeError('Watcher glob must be a string');
  // Refuse explicitly before trimming/scanning or building expanded regexes.
  if (pattern.length > MAX_PATTERN_BYTES || Buffer.byteLength(pattern) > MAX_PATTERN_BYTES || pattern.includes('\0'))
    throw new GlobLimitError('Watcher glob exceeds 4096 UTF-8 bytes or contains NUL');
  if (typeof ignoreCase !== 'boolean') throw new TypeError('Watcher ignoreCase must be a boolean');
  const parsed = parsePattern(pattern, {ignoreCase}, {work:0,expansions:0}, 0);
  return path => typeof path === 'string' && parsed(path);
}
module.exports = { compileGlob };
