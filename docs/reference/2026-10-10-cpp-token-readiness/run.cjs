'use strict';
const path=require('node:path');
const {supervise}=require('../../tests/vscode-reference/supervisor.cjs');
async function main(){for(const mode of ['none','keep','brackets','advanced','full'])await supervise(path.join(__dirname,'worker.cjs'),[mode],{timeoutMs:30000,graceMs:1000});}
main().catch(error=>{console.error(error);process.exitCode=1;});
