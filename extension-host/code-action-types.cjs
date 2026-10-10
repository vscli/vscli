'use strict';
const { Uri, Range } = require('./api-types.cjs');
class CodeActionKind {
  constructor(value) { if (typeof value !== 'string' || Buffer.byteLength(value) > 128) throw new TypeError('Invalid code action kind'); this.value = value; Object.freeze(this); }
  append(part) { return new CodeActionKind(this.value ? `${this.value}.${part}` : part); }
  contains(other) { return this.value === '' || this.value === other.value || other.value.startsWith(`${this.value}.`); }
  intersects(other) { return this.contains(other) || other.contains(this); }
}
for (const [name, value] of Object.entries({ Empty:'', QuickFix:'quickfix', Refactor:'refactor', RefactorExtract:'refactor.extract', RefactorInline:'refactor.inline', RefactorMove:'refactor.move', RefactorRewrite:'refactor.rewrite', Source:'source', SourceOrganizeImports:'source.organizeImports', SourceFixAll:'source.fixAll', Notebook:'notebook' })) Object.defineProperty(CodeActionKind, name, { value: new CodeActionKind(value), enumerable: true });
const CodeActionTriggerKind = Object.freeze({ Invoke:1, Automatic:2 });
class CodeAction { constructor(title, kind) { this.title = title; if (kind !== undefined) this.kind = kind; } }
const edits = new WeakMap();
function key(uri) { if (!(uri instanceof Uri)) throw new TypeError('WorkspaceEdit requires URI'); const result = uri.toString(); if (Buffer.byteLength(result) > 4096) throw new Error('Workspace edit URI exceeds budget'); return result; }
function array(values, max) {
  if (!Array.isArray(values)) throw new TypeError('Workspace edits require an array'); const count = values.length;
  if (!Number.isSafeInteger(count) || count < 0 || count > max) throw new Error('Workspace edit count exceeds 4096');
  const result = []; for (let index = 0; index < count; index++) result.push(values[index]);
  if (values.length !== count) throw new Error('Workspace edit array changed during capture'); return result;
}
class WorkspaceEdit {
  constructor() { edits.set(this, { resources:new Map(), unsupported:[], generation:0 }); }
  get size() { const state = edits.get(this); return state.resources.size + state.unsupported.length; }
  set(uri, values) {
    const state = edits.get(this), expected = state.generation, uriKey = key(uri), copied = array(values,4096), next = new Map(state.resources);
    next.set(uriKey,{uri,key:uriKey,values:copied});
    if (next.size > 128 || [...next.values()].reduce((count, entry) => count + entry.values.length,0) > 4096) throw new Error('Workspace edit resource/count budget exceeded');
    if (state.generation !== expected) throw new Error('Workspace edit changed during capture'); state.resources = next; state.generation++;
  }
  replace(uri, range, newText, metadata) { this._add(uri,new (require('./provider-types.cjs').TextEdit)(range,newText),metadata); }
  insert(uri, position, newText, metadata) { this.replace(uri,new Range(position,position),newText,metadata); }
  delete(uri, range, metadata) { this.replace(uri,range,'',metadata); }
  _add(uri, edit, metadata) {
    const state = edits.get(this), expected = state.generation, uriKey = key(uri), existing = state.resources.get(uriKey)?.values || [], values = [...existing,metadata === undefined ? edit : [edit,metadata]], next = new Map(state.resources);
    next.set(uriKey,{uri,key:uriKey,values});
    if(next.size>128 || [...next.values()].reduce((count,entry)=>count+entry.values.length,0)>4096)throw new Error('Workspace edit resource/count budget exceeded');
    if(state.generation!==expected)throw new Error('Workspace edit changed during URI capture');state.resources=next;state.generation++;
  }
  has(uri) { return edits.get(this).resources.has(key(uri)); }
  get(uri) { return (edits.get(this).resources.get(key(uri))?.values || []).slice(); }
  entries() { return [...edits.get(this).resources.values()].map(entry => [entry.uri,entry.values.slice()]); }
  _unsupported(operation) { const state = edits.get(this); if (state.unsupported.length >= 128) throw new Error('Workspace resource operation count exceeds 128'); state.unsupported.push(operation); state.generation++; }
  createFile() { this._unsupported('file creation'); }
  renameFile() { this._unsupported('file rename'); }
  deleteFile() { this._unsupported('file deletion'); }
}
function snapshotWorkspaceEdit(value) {
  const state = edits.get(value); if (!state) throw new TypeError('Code action edit requires WorkspaceEdit');
  return { generation:state.generation, resources:[...state.resources.values()].map(entry => ({...entry,values:entry.values.slice()})), unsupported:state.unsupported.slice(), current:() => state.generation };
}
module.exports = { CodeAction,CodeActionKind,CodeActionTriggerKind,WorkspaceEdit,snapshotWorkspaceEdit };
