'use strict';
// Source-derived values: microsoft/vscode 1.141.0,
// 2a59476c9bfcb90b3ddc372c36762471b7dfad1c,
// extHostTypes/codeActionKind.ts and extHostTypes.ts.
// Copyright (c) Microsoft Corporation. All rights reserved.
// Licensed under the MIT License; see filesystem.LICENSE.txt.
const { es5ClassCompat } = require('./es5-class-compat.cjs');
const { WorkspaceEdit, snapshotWorkspaceEdit } = require('./edit-values.cjs');
// Runtime value semantics follow the pinned classes. Native wire limits belong
// to provider registration and result serialization, not these constructors.
class CodeActionKind {
  static sep = '.';
  constructor(value) { this.value = value; }
  append(parts) { return new CodeActionKind(this.value ? this.value + CodeActionKind.sep + parts : parts); }
  intersects(other) { return this.contains(other) || other.contains(this); }
  contains(other) { return this.value === other.value || other.value.startsWith(this.value + CodeActionKind.sep); }
}
class CodeAction { constructor(title, kind) { this.title = title; this.kind = kind; } }
// Preserve only the classes decorated by pinned upstream.
for (const value of [CodeActionKind, CodeAction]) es5ClassCompat(value);
CodeActionKind.Empty = new CodeActionKind('');
CodeActionKind.QuickFix = CodeActionKind.Empty.append('quickfix');
CodeActionKind.Refactor = CodeActionKind.Empty.append('refactor');
CodeActionKind.RefactorExtract = CodeActionKind.Refactor.append('extract');
CodeActionKind.RefactorInline = CodeActionKind.Refactor.append('inline');
CodeActionKind.RefactorMove = CodeActionKind.Refactor.append('move');
CodeActionKind.RefactorRewrite = CodeActionKind.Refactor.append('rewrite');
CodeActionKind.Source = CodeActionKind.Empty.append('source');
CodeActionKind.SourceOrganizeImports = CodeActionKind.Source.append('organizeImports');
CodeActionKind.SourceFixAll = CodeActionKind.Source.append('fixAll');
CodeActionKind.Notebook = CodeActionKind.Empty.append('notebook');
const CodeActionTriggerKind = Object.freeze({ Invoke:1, Automatic:2 });
module.exports = { CodeAction,CodeActionKind,CodeActionTriggerKind,WorkspaceEdit,snapshotWorkspaceEdit };
