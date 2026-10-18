'use strict';
const fs=require('node:fs'),{Parser}=require('./vs/platform/contextkey/common/contextkey.js');
const cases=JSON.parse(fs.readFileSync(0,'utf8'));
if(!Array.isArray(cases)||cases.length>2048)throw new Error('Oracle input exceeds bounds');
process.stdout.write(JSON.stringify(cases.map(({source,context})=>{
 const parser=new Parser({regexParsingWithErrorRecovery:false}),expr=parser.parse(source);
 return expr?{valid:true,value:expr.evaluate({getValue:key=>Object.hasOwn(context,key)?context[key]:undefined})}:{valid:false};
})));
