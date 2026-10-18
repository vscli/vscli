'use strict';
// Source: microsoft/vscode 1.141.0, immutable commit
// 2a59476c9bfcb90b3ddc372c36762471b7dfad1c,
// src/vs/workbench/api/common/extHostTypes/es5ClassCompat.ts.
// Copyright (c) Microsoft Corporation. All rights reserved.
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
// Only classes explicitly decorated in pinned upstream may use this helper.
function es5ClassCompat(target) {
  return Object.assign(target, {
    apply(...args) {
      if (args.length === 0) return Reflect.construct(target, []);
      const argsList = args.length === 1 ? [] : args[1];
      return Reflect.construct(target, argsList, args[0].constructor);
    },
    call(...args) {
      if (args.length === 0) return Reflect.construct(target, []);
      const [receiver, ...rest] = args;
      return Reflect.construct(target, rest, receiver.constructor);
    },
  });
}
module.exports = { es5ClassCompat };
