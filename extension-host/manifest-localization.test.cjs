'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),os=require('node:os'),path=require('node:path');
const {createDefaults}=require('./manifest-localization.cjs');
function fixture(run){const root=fs.mkdtempSync(path.join(os.tmpdir(),'vscli-manifest-nls-'));try{return run(root);}finally{fs.rmSync(root,{recursive:true,force:true});}}
test('upstream default normalizer replaces exact nested labels and command strings, preserving raw source and fallbacks',()=>fixture(root=>{
 const file=path.join(root,'package.nls.json'),bytes=Buffer.from(JSON.stringify({title:'Run default',label:{message:'Default label',comment:['context']},empty:''}));fs.writeFileSync(file,bytes);
 const raw={displayName:'%label%',contributes:{commands:[{title:'%title%',category:'%label%'}],themes:[{label:'%label%'}]},unknown:'%unknown%',empty:'%empty%',embedded:'before %title%',percent:'%%'},warnings=[];
 const {manifest}=createDefaults(0,value=>warnings.push(value)).localize(root,raw);
 assert.equal(manifest.displayName,'Default label');assert.equal(manifest.contributes.commands[0].title,'Run default');assert.equal(manifest.contributes.commands[0].category,'Default label');assert.equal(manifest.contributes.themes[0].label,'Default label');for(const key of ['unknown','empty','embedded','percent'])assert.equal(manifest[key],raw[key]);assert.equal(raw.displayName,'%label%');assert.deepEqual(fs.readFileSync(file),bytes);assert.deepEqual(warnings,[]);
}));
test('malformed/expanding translations reject the whole normalization without changing raw manifests',()=>fixture(root=>{
 const original={a:'%long%',b:'%long%'};
 for(const text of ['not JSON',JSON.stringify({long:'x'.repeat(600*1024)})]){fs.writeFileSync(path.join(root,'package.nls.json'),text);const warnings=[];assert.strictEqual(createDefaults(0,value=>warnings.push(value)).localize(root,original).manifest,original);assert.equal(warnings.length,1);}
}));
test('default manifest capture retains missing files and rejects occupied aggregate budgets and symlinks',()=>fixture(root=>{
 const original={label:'%label%'},file=path.join(root,'package.nls.json'),warnings=[];assert.strictEqual(createDefaults().localize(root,original).manifest,original);fs.writeFileSync(file,'{"label":"valid"}');assert.strictEqual(createDefaults(8*1024*1024,value=>warnings.push(value)).localize(root,original).manifest,original);assert.equal(warnings.length,1);
 if(process.platform!=='win32'){fs.renameSync(file,file+'.original');fs.symlinkSync(file+'.original',file);assert.strictEqual(createDefaults(0,value=>warnings.push(value)).localize(root,original).manifest,original);assert.equal(warnings.length,2);}
}));
