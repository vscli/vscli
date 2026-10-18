"use strict";
const MAX_BYTES=1024*1024, wellFormed=String.prototype.isWellFormed;
function isClipboardText(value) {return typeof value==='string'&&Reflect.apply(wellFormed,value,[])&&Buffer.byteLength(value)<=MAX_BYTES;}
function createClipboard(options) {
 return {forOwner(owner) {
  const guard=owner ? options.captureCaller(owner) : ()=>{throw new Error('Clipboard requires an extension owner');};
  const check=()=>{if(options.enabled!==true)throw new Error('VSCLI clipboard is not implemented by this native host');guard();};
  return Object.freeze({
   async readText() {
    check();const result=await options.request(owner,'nativeClipboardRead',null);guard();
    if(!isClipboardText(result))throw new Error('Invalid native clipboard text receipt');
    return result;
   },
   async writeText(value) {
    check();if(typeof value!=='string')throw new TypeError('Clipboard text must be a string');
    if(!Reflect.apply(wellFormed,value,[]))throw new Error('Clipboard text contains unpaired UTF-16 surrogates');
    if(Buffer.byteLength(value)>MAX_BYTES)throw new Error('Clipboard text exceeds 1 MiB');
    const result=await options.request(owner,'nativeClipboardWrite',{text:value});guard();
    if(result!==null)throw new Error('Invalid native clipboard write receipt');
   },
  });
 }};
}
module.exports={createClipboard,isClipboardText};
