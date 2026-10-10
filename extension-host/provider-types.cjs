'use strict';
// Original, bounded compatibility values; native code validates wire results.
const { Position, Range, Disposable, EventEmitter } = require('./api-types.cjs');
function enumeration(names) { return Object.freeze(Object.fromEntries(names.map((name, index) => [name, index]))); }
const CompletionItemKind = enumeration(['Text','Method','Function','Constructor','Field','Variable','Class','Interface','Module','Property','Unit','Value','Enum','Keyword','Snippet','Color','File','Reference','Folder','EnumMember','Constant','Struct','Event','Operator','TypeParameter']);
const SymbolKind = enumeration(['File','Module','Namespace','Package','Class','Method','Property','Field','Constructor','Enum','Interface','Function','Variable','Constant','String','Number','Boolean','Array','Object','Key','Null','EnumMember','Struct','Event','Operator','TypeParameter']);
const SymbolTag = Object.freeze({ Deprecated: 1 });
class MarkdownString {
  constructor(value = '', supportThemeIcons = false) { this.value = value; this.supportThemeIcons = supportThemeIcons; this.isTrusted = false; this.supportHtml = false; }
  appendText(value) { this.value += String(value).replace(/[\\`*_{}\[\]()<>#+.!-]/g, '\\$&'); return this; }
  appendMarkdown(value) { this.value += value; return this; }
  appendCodeblock(value, language = '') { this.value += `\n\n\`\`\`${language}\n${value}\n\`\`\`\n`; return this; }
}
class SnippetString {
  constructor(value = '') { this.value = value; this._tabstop = 1; }
  appendText(value) { this.value += String(value).replace(/[\\$}]/g, '\\$&'); return this; }
  appendTabstop(number = this._tabstop++) { this.value += `$${number}`; return this; }
  appendPlaceholder(value, number = this._tabstop++) {
    if (typeof value !== 'string') throw new Error('VSCLI does not implement callback snippet placeholders');
    this.value += `\${${number}:${value.replace(/[\\$}]/g, '\\$&')}}`; return this;
  }
}
class CompletionItem { constructor(label, kind) { this.label = label; if (kind !== undefined) this.kind = kind; } }
class CompletionList { constructor(items = [], isIncomplete = false) { this.items = items; this.isIncomplete = isIncomplete; } }
class Hover { constructor(contents, range) { this.contents = Array.isArray(contents) ? contents : [contents]; if (range !== undefined) this.range = range; } }
class Location { constructor(uri, rangeOrPosition) { this.uri = uri; this.range = rangeOrPosition instanceof Position ? new Range(rangeOrPosition, rangeOrPosition) : rangeOrPosition; } }
class TextEdit {
  constructor(range, newText) { this.range = range; this.newText = newText; }
  static replace(range, newText) { return new TextEdit(range, newText); }
  static insert(position, newText) { return new TextEdit(new Range(position, position), newText); }
  static delete(range) { return new TextEdit(range, ''); }
}
class DocumentSymbol {
  constructor(name, detail, kind, range, selectionRange) {
    Object.assign(this, { name, detail, kind, range, selectionRange, children: [] });
    DocumentSymbol.validate(this);
  }
  static validate(candidate) {
    let count = 0;
    function visit(value, depth) {
      if (++count > 512 || depth > 16) throw new Error('DocumentSymbol count or depth limit exceeded');
      const name = value?.name;
      if (typeof name !== 'string' || !name || Buffer.byteLength(name) > 4096) throw new Error('DocumentSymbol requires a nonempty name of at most 4 KiB');
      const range = value.range, selection = value.selectionRange;
      if (!(range instanceof Range) || !(selection instanceof Range) || !range.contains(selection)) throw new Error('selectionRange must be contained in fullRange');
      const children = value.children;
      if (children === undefined) return;
      if (!Array.isArray(children)) throw new Error('DocumentSymbol children must be an array');
      const length = children.length;
      if (!Number.isSafeInteger(length) || length < 0 || length > 512) throw new Error('DocumentSymbol child count exceeded');
      for (let index = 0; index < length; index++) visit(children[index], depth + 1);
      if (children.length !== length) throw new Error('DocumentSymbol children changed during validation');
    }
    visit(candidate, 0);
  }
}
class SymbolInformation {
  constructor(name, kind, rangeOrContainer, locationOrUri, containerName) {
    this.name = name; this.kind = kind; this.containerName = undefined;
    if (rangeOrContainer instanceof Range) {
      this.location = new Location(locationOrUri, rangeOrContainer);
      this.containerName = containerName;
    } else {
      this.containerName = rangeOrContainer;
      // Preserve the early compatibility overload while supporting the public
      // Location overload and the standard Range/Uri overload above.
      this.location = containerName && locationOrUri instanceof Range ? new Location(containerName, locationOrUri) : locationOrUri;
    }
    SymbolInformation.validate(this);
  }
  static validate(candidate) {
    const name = candidate?.name;
    if (typeof name !== 'string' || !name || Buffer.byteLength(name) > 4096) throw new Error('SymbolInformation requires a nonempty name of at most 4 KiB');
  }
}
class ParameterInformation { constructor(label, documentation) { this.label = label; if (documentation !== undefined) this.documentation = documentation; } }
class SignatureInformation { constructor(label, documentation) { this.label = label; this.parameters = []; if (documentation !== undefined) this.documentation = documentation; } }
class SignatureHelp { constructor() { this.signatures = []; this.activeSignature = 0; this.activeParameter = 0; } }
class CancellationTokenSource {
  constructor(parent) {
    const emitter = new EventEmitter();
    let canceled = false;
    const event = (listener, thisArg, disposables) => {
      if (!canceled) return emitter.event(listener, thisArg, disposables);
      const timer = setTimeout(() => listener.call(thisArg, undefined), 0);
      const result = new Disposable(() => clearTimeout(timer));
      if (disposables) disposables.push(result);
      return result;
    };
    this.token = Object.freeze({ get isCancellationRequested() { return canceled; }, onCancellationRequested: event });
    this.cancel = () => { if (!canceled) { canceled = true; emitter.fire(undefined); emitter.dispose(); } };
    const parentListener = parent?.onCancellationRequested(this.cancel);
    if (parent?.isCancellationRequested) this.cancel();
    this.dispose = (cancel = false) => { if (cancel) this.cancel(); parentListener?.dispose(); emitter.dispose(); };
  }
}
const CompletionTriggerKind = Object.freeze({ Invoke: 0, TriggerCharacter: 1, TriggerForIncompleteCompletions: 2 });
const SignatureHelpTriggerKind = Object.freeze({ Invoke: 1, TriggerCharacter: 2, ContentChange: 3 });

const { CodeAction, CodeActionKind, CodeActionTriggerKind, WorkspaceEdit } = require('./code-action-types.cjs');
module.exports = { CodeAction, CodeActionKind, CodeActionTriggerKind, WorkspaceEdit, CompletionItemKind, SymbolKind, SymbolTag, MarkdownString, SnippetString, CompletionItem, CompletionList, Hover, Location, TextEdit, DocumentSymbol, SymbolInformation, ParameterInformation, SignatureInformation, SignatureHelp, CancellationTokenSource, CompletionTriggerKind, SignatureHelpTriggerKind };
