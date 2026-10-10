'use strict';
const fs=require('node:fs'),os=require('node:os'),path=require('node:path'),{createHash}=require('node:crypto');
const {runTests}=require('../../tests/vscode-reference/node_modules/@vscode/test-electron');
const {workerLifetime}=require('../../tests/vscode-reference/supervisor.cjs');
workerLifetime();
async function main(){
 const mode=process.argv[2];process.env.VSCLI_REFERENCE_INDENT_MODE=mode;
 const session=fs.mkdtempSync(path.join(os.tmpdir(),`vscli-fixed-indent-${mode}-`)),profile=path.join(session,'profile'),extensions=path.join(session,'extensions');
 fs.mkdirSync(path.join(profile,'User'),{recursive:true});fs.mkdirSync(extensions);
 const settings=JSON.stringify({'telemetry.telemetryLevel':'off','update.mode':'none','extensions.autoUpdate':false,'extensions.autoCheckUpdates':false,'security.workspace.trust.enabled':false,'workbench.startupEditor':'none','editor.autoIndent':mode,'[cpp]':{'editor.autoIndent':mode},'editor.tabSize':4,'editor.insertSpaces':true,'editor.detectIndentation':false,'editor.autoClosingQuotes':'languageDefined','editor.autoClosingBrackets':'languageDefined','editor.parameterHints.enabled':false,'editor.quickSuggestions':false});
 fs.writeFileSync(path.join(profile,'User/settings.json'),settings);process.env.VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256=createHash('sha256').update(settings).digest('hex');
 try{await runTests({vscodeExecutablePath:path.resolve(__dirname,'../vscode-reference/cache/vscode-linux-x64-1.95.0/code'),extensionDevelopmentPath:__dirname,extensionTestsPath:path.join(__dirname,'suite.cjs'),launchArgs:['--user-data-dir',profile,'--extensions-dir',extensions,'--locale=en','--skip-welcome','--skip-release-notes','--disable-workspace-trust','--disable-gpu','--no-sandbox']});}finally{fs.rmSync(session,{recursive:true,force:true});}
}
main().then(()=>process.send({type:'complete',ok:true}),error=>{console.error(error);process.send({type:'complete',ok:false,error:String(error?.stack||error)});});
