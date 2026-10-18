'use strict';
const assert=require('node:assert/strict'),v=require('vscode');
exports.activate=context=>{
 const manifest=v.extensions.getExtension('fixture.default-manifest').packageJSON;
 assert.equal(manifest.displayName,'Default manifest fixture');assert.equal(manifest.contributes.commands[0].title,'Apply localized edit');assert.equal(manifest.contributes.commands[0].category,'Default category');assert.equal(manifest.contributes.themes[0].label,'Default translated theme');
 context.subscriptions.push(v.commands.registerCommand('fixture.defaultManifest.edit',async()=>{const editor=v.window.activeTextEditor;assert(editor);assert.equal(await editor.edit(edit=>edit.insert(new v.Position(0,0),'localized ')),true);return v.window.showInformationMessage('Default manifest edit applied');}));
};
