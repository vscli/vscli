'use strict';
// Bounded capture around the pinned upstream default-message normalizer.
const fs=require('node:fs'),path=require('node:path');
const {localizeManifest}=require('./upstream-manifest-nls.cjs');
const MAX=1024*1024;
function createDefaults(initialBytes=0, diagnostic=()=>{}) {
  let bytes=initialBytes;
  function localize(folder, original) {
    const file=path.join(folder,'package.nls.json');
    let size=0;
    try {
      let stat;
      try{stat=fs.lstatSync(file);}catch(error){if(error.code==='ENOENT')return {manifest:original,bytes:0};throw error;}
      if(!stat.isFile()||stat.size>MAX)throw new Error('package.nls.json must be a regular file at most 1 MiB');
      const fd=fs.openSync(file,fs.constants.O_RDONLY|(fs.constants.O_NOFOLLOW||0)|(fs.constants.O_NONBLOCK||0));
      let messages;
      try {
        const before=fs.fstatSync(fd);
        if(!before.isFile()||before.size>MAX)throw new Error('package.nls.json changed type or exceeds 1 MiB');
        const remaining=Math.max(0,8*MAX-bytes),buffer=Buffer.alloc(Math.min(MAX,remaining)+1);
        let count;
        while(size<buffer.length&&(count=fs.readSync(fd,buffer,size,buffer.length-size,null)))size+=count;
        bytes+=size;
        if(size>MAX||size>remaining)throw new Error('Manifest translations exceed the 8 MiB batch / 1 MiB file budget');
        const after=fs.fstatSync(fd);
        if(size!==before.size||before.size!==after.size||before.mtimeMs!==after.mtimeMs)throw new Error('package.nls.json changed during capture');
        messages=JSON.parse(buffer.toString('utf8',0,size));
      }finally{fs.closeSync(fd);}
      if(!messages||typeof messages!=='object'||Array.isArray(messages)||Object.keys(messages).length>16384)throw new Error('package.nls.json requires an object with at most 16384 keys');
      const normalized=JSON.parse(JSON.stringify(original));
      let normalizedSize=Buffer.byteLength(JSON.stringify(original));
      function budget(value,depth=0){
        if(depth>64)throw new Error('Manifest localization exceeds 64 levels');
        if(typeof value==='string'&&value.length>1&&value.startsWith('%')&&value.endsWith('%')){
          const key=value.slice(1,-1),entry=Object.hasOwn(messages,key)?messages[key]:undefined;
          const message=typeof entry==='string'?entry:entry?.message;
          if(typeof message==='string'&&message){normalizedSize+=Buffer.byteLength(JSON.stringify(message))-Buffer.byteLength(JSON.stringify(value));if(normalizedSize>MAX)throw new Error('Localized manifest exceeds 1 MiB');}
        }else if(value&&typeof value==='object')for(const child of Object.values(value))budget(child,depth+1);
      }
      if(normalizedSize>MAX)throw new Error('Localized manifest exceeds 1 MiB');
      budget(normalized);
      localizeManifest({warn:()=>{},error:error=>{throw new Error(String(error));}},normalized,messages);
      return {manifest:normalized,bytes:size};
    }catch(error){diagnostic('Manifest localization unavailable; original labels retained: '+String(error.message||error).slice(0,1024));return {manifest:original,bytes:size};}
  }
  return {localize};
}
module.exports={createDefaults};
