'use strict';
const { EventEmitter, Uri } = require('./api-types.cjs');
function reviveFileOperation(frame, session) {
  if (!frame || frame.session !== session || !Number.isSafeInteger(frame.sequence) || frame.sequence <= 0 ||
      !['create','rename','delete'].includes(frame.kind) || !Array.isArray(frame.files) || frame.files.length > 128 ||
      Object.keys(frame).some(key=>!['session','sequence','kind','files'].includes(key))) throw new Error('Invalid native file operation event');
  let bytes = 0;
  return frame.files.map(file => {
    if (!file || typeof file.target !== 'string' || !file.target || file.target.length > 16384 ||
        (frame.kind === 'rename' ? typeof file.source !== 'string' || !file.source || file.source.length > 16384 : file.source !== undefined) ||
        Object.keys(file).some(key=>!['source','target'].includes(key))) throw new Error('Invalid native file operation resource');
    bytes += Buffer.byteLength(file.target) + (file.source ? Buffer.byteLength(file.source) : 0);
    if (bytes > 1024*1024) throw new Error('Native file operation event exceeds byte budget');
    return frame.kind === 'rename' ? {oldUri:Uri.parse(file.source),newUri:Uri.parse(file.target)} : Uri.parse(file.target);
  });
}
function createFileOperations(session) {
  const emitters = {create:new EventEmitter(),rename:new EventEmitter(),delete:new EventEmitter()};
  let sequence = 0;
  const api = {onDidCreateFiles:emitters.create.event,onDidRenameFiles:emitters.rename.event,onDidDeleteFiles:emitters.delete.event};
  function publish(frame) {
    const files = reviveFileOperation(frame, session);
    if (frame.sequence <= sequence) throw new Error('Invalid native file operation event');
    sequence = frame.sequence;
    emitters[frame.kind].fire(Object.freeze({files}));
  }
  return {api,publish};
}
module.exports = {createFileOperations,reviveFileOperation};
