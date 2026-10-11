'use strict';
const { WorkspaceEdit, snapshotWorkspaceEdit } = require('./edit-values.cjs');
class CodeActionKind {
  constructor(value) { if (typeof value !== 'string' || Buffer.byteLength(value) > 128) throw new TypeError('Invalid code action kind'); this.value = value; Object.freeze(this); }
  append(part) { return new CodeActionKind(this.value ? `${this.value}.${part}` : part); }
  contains(other) { return this.value === '' || this.value === other.value || other.value.startsWith(`${this.value}.`); }
  intersects(other) { return this.contains(other) || other.contains(this); }
}
for (const [name, value] of Object.entries({ Empty:'', QuickFix:'quickfix', Refactor:'refactor', RefactorExtract:'refactor.extract', RefactorInline:'refactor.inline', RefactorMove:'refactor.move', RefactorRewrite:'refactor.rewrite', Source:'source', SourceOrganizeImports:'source.organizeImports', SourceFixAll:'source.fixAll', Notebook:'notebook' })) Object.defineProperty(CodeActionKind, name, { value: new CodeActionKind(value), enumerable: true });
const CodeActionTriggerKind = Object.freeze({ Invoke:1, Automatic:2 });
class CodeAction { constructor(title, kind) { this.title = title; if (kind !== undefined) this.kind = kind; } }
module.exports = { CodeAction,CodeActionKind,CodeActionTriggerKind,WorkspaceEdit,snapshotWorkspaceEdit };
