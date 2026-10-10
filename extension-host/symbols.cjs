'use strict';
const { Uri } = require('./api-types.cjs');

function array(value) {
  if (!Array.isArray(value)) throw new TypeError('Provider symbols must be an array');
  const count = value.length;
  if (!Number.isSafeInteger(count) || count < 0 || count > 512) throw new Error('Provider symbol array exceeds 512 entries');
  const result = [];
  for (let index = 0; index < count; index++) result.push(value[index]);
  if (value.length !== count) throw new Error('Provider symbol array changed during validation');
  return result;
}
function normalize(value, document, options) {
  if (value === undefined || value === null) return null;
  let count = 0, bytes = 0;
  function text(value) {
    const result = options.text(value, 4096);
    bytes += Buffer.byteLength(result);
    if (bytes > 65536) throw new Error('Provider symbol display metadata exceeds 64 KiB');
    return result;
  }
  function convert(value, depth) {
    if (++count > 512 || depth > 16) throw new Error('Provider symbol count or depth limit exceeded');
    if (!value || typeof value !== 'object') throw new TypeError('Invalid provider symbol');
    const name = value.name, kind = value.kind, location = value.location, rawTags = value.tags, rawDeprecated = value.deprecated;
    if (typeof name !== 'string' || !name) throw new Error('Provider symbol name must be nonempty');
    const result = {name:text(name), kind:options.kind(kind,25)};
    if (rawTags !== undefined) {
      if (!Array.isArray(rawTags)) throw new Error('Invalid provider symbol tags');
      const length = rawTags.length;
      if (!Number.isSafeInteger(length) || length < 0 || length > 16) throw new Error('Provider symbol tag budget exceeded');
      const tags = [];
      for (let index = 0; index < length; index++) {
        const tag = rawTags[index];
        if (tag !== 1) throw new Error('Unsupported provider symbol tag');
        tags.push(tag);
      }
      if (rawTags.length !== length) throw new Error('Provider symbol tags changed during validation');
      result.tags = tags;
    }
    if (rawDeprecated !== undefined) {
      if (typeof rawDeprecated !== 'boolean') throw new Error('Invalid provider symbol deprecated flag');
      result.deprecated = rawDeprecated;
    }
    if (location !== undefined) {
      const children = value.children, containerName = value.containerName;
      if (!location || typeof location !== 'object' || children !== undefined) throw new Error('Location-based symbol children are unsupported');
      const rawUri = location.uri, rawRange = location.range;
      if (!(rawUri instanceof Uri)) throw new Error('Document symbols require a document URI');
      const uri = rawUri.toString(), expected = document.uri.toString();
      if (uri !== expected || !['file','untitled'].includes(rawUri.scheme) || rawUri.query || rawUri.fragment) throw new Error('Document symbol URI does not match its mirrored document');
      return {...result,containerName:text(containerName === undefined ? '' : containerName),location:{uri:options.text(uri,4096),range:options.range(rawRange,document)}};
    }
    const rawDetail = value.detail, rawRange = value.range, rawSelection = value.selectionRange, rawChildren = value.children;
    const outer = options.range(rawRange,document), selection = options.range(rawSelection,document);
    if (selection.start.isBefore(outer.start) || selection.end.isAfter(outer.end)) throw new Error('Symbol selectionRange lies outside its range');
    return {...result,detail:text(rawDetail === undefined ? '' : rawDetail),range:outer,selectionRange:selection,
      children:array(rawChildren === undefined ? [] : rawChildren).map(child => convert(child,depth+1))};
  }
  return array(value).map(value => convert(value,0));
}
function createSymbols(options) {
  let occupied;
  function reserve(call, request) {
    if (occupied) throw new Error('A document symbol callback is already running');
    occupied = {call,request};
  }
  function released(params, call) {
    // A duplicate-ID rejection is not proof that the admitted callback settled.
    if (!call?.symbolWork && occupied?.request === params.request) return;
    if (call && occupied?.call === call) occupied = undefined;
    if (params.session === options.session && Number.isSafeInteger(params.request) && params.request > 0 && typeof params.owner === 'string' && Number.isSafeInteger(params.provider) && params.provider > 0) {
      options.notify('symbolsReleased',{session:options.session,owner:params.owner,provider:params.provider,request:params.request});
    }
  }
  return {reserve,released,pending:()=>!!occupied};
}
module.exports = {normalize,createSymbols};
