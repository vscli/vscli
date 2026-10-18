/*---------------------------------------------------------------------------------------------
 * Copyright (c) Microsoft Corporation. All rights reserved.
 * Licensed under the MIT License. Source: immutable stable VS Code 1.141.0
 * https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypes.ts
 * These constructors intentionally preserve upstream parameter-property values
 * without validation, coercion, cloning or runtime freezing.
 *--------------------------------------------------------------------------------------------*/
'use strict';
class TextTabInput { constructor(uri) {this.uri=uri;} }
class TextDiffTabInput { constructor(original,modified) {this.original=original;this.modified=modified;} }
class CustomEditorTabInput { constructor(uri,viewType) {this.uri=uri;this.viewType=viewType;} }
class WebviewEditorTabInput { constructor(viewType) {this.viewType=viewType;} }
class NotebookEditorTabInput { constructor(uri,notebookType) {this.uri=uri;this.notebookType=notebookType;} }
class NotebookDiffEditorTabInput { constructor(original,modified,notebookType) {this.original=original;this.modified=modified;this.notebookType=notebookType;} }
class TerminalEditorTabInput { constructor() {} }
module.exports={TabInputText:TextTabInput,TabInputTextDiff:TextDiffTabInput,TabInputCustom:CustomEditorTabInput,TabInputWebview:WebviewEditorTabInput,TabInputNotebook:NotebookEditorTabInput,TabInputNotebookDiff:NotebookDiffEditorTabInput,TabInputTerminal:TerminalEditorTabInput};
