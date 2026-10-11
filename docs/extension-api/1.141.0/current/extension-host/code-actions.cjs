'use strict';
const { Uri, Selection } = require('./api-types.cjs');
const { CodeActionKind, snapshotWorkspaceEdit } = require('./code-action-types.cjs');
function array(value,max) {
  if (!Array.isArray(value)) throw new Error('Code action result requires an array'); const count = value.length;
  if (!Number.isSafeInteger(count) || count < 0 || count > max) throw new Error(`Code action array exceeds ${max} entries`);
  const copied = []; for(let index=0;index<count;index++) copied.push(value[index]);
  if(value.length!==count) throw new Error('Code action array changed during normalization'); return copied;
}
function captureWorkspace(params, options) {
  const captured = new Map();
  for (const target of array(params.workspace,128)) {
    const document = options.document(target.document);
    if (!document || document.isClosed || document.version !== target.version || document._snapshot.uri !== target.uri || captured.has(target.uri)) throw new Error('Code action workspace snapshot became stale');
    captured.set(target.uri,{document,version:target.version,uri:target.uri});
  }
  return captured;
}
function assertWorkspace(workspace, options) {
  for (const target of workspace.values()) if (target.document.isClosed || target.document.version !== target.version || options.document(target.document._snapshot.id) !== target.document || target.document._snapshot.uri !== target.uri) throw new Error('Code action workspace changed or closed');
}
function createActions(options) {
  const cached = new Map(); let nextHandle = 0;
  function purge() { for(const [id,item]of cached) if(Date.now()-item.started>=6000 || !options.current(item)) cached.delete(id); }
  function retire(origin,owner) { for(const [id,item]of cached) if(item.origin===origin && item.entry.owner===owner) cached.delete(id); }
  function clear() { cached.clear(); }
  function args(params,call) {
    call.workspace = captureWorkspace(params,options);
    const range = options.range(params.range,call.document), selection = params.selection;
    const selected = selection ? new Selection(options.position(selection.anchor,call.document),options.position(selection.active,call.document)) : new Selection(range.start,range.end);
    if(JSON.stringify(options.range(selected,call.document))!==JSON.stringify(range)) throw new Error('Code action selection differs from requested range');
    const context = params.actionContext || {};
    if(Object.keys(context).some(key=>!['only','triggerKind'].includes(key)) || ![1,2].includes(context.triggerKind??1)) throw new Error('Unsupported code action context');
    const only = context.only===undefined ? undefined : new CodeActionKind(options.text(context.only,128));
    const diagnostics = options.diagnostics ? options.diagnostics(call.document,selected) : [];
    return [call.document,selected,{diagnostics,only,triggerKind:context.triggerKind??1},call.source.token];
  }
  function workspaceEdit(value,workspace) {
    const snapshot = snapshotWorkspaceEdit(value);
    if(snapshot.unsupported.length) throw new Error(`Unsupported workspace resource operation: ${snapshot.unsupported[0]}`);
    const documentChanges = []; let count = 0,bytes = 0;
    for(const resource of snapshot.resources) {
      const target=workspace.get(resource.key);
      if(!target) throw new Error('Code action target is outside the captured mirrored documents');
      const uri=Uri.parse(resource.key);
      if(!['file','untitled'].includes(uri.scheme)||uri.query||uri.fragment||uri.authority&&uri.authority!=='localhost') throw new Error('Unsupported workspace edit URI');
      const edits=[];
      for(const candidate of resource.values) {
        if(++count>4096) throw new Error('Code action exceeds4096 text edits');
        if(Array.isArray(candidate)||candidate?.annotationId!==undefined) throw new Error('Annotated workspace text edits are unsupported');
        if(!candidate||typeof candidate!=='object'||Object.keys(candidate).some(key=>!['range','newText'].includes(key))) throw new Error('Unsupported workspace text edit');
        const newText=options.text(candidate.newText,1024*1024); bytes+=Buffer.byteLength(JSON.stringify(newText))+128; if(bytes>1024*1024) throw new Error('Code action exceeds1 MiB provider result budget');
        edits.push({range:options.range(candidate.range,target.document),newText});
      }
      documentChanges.push({textDocument:{uri:resource.key,version:target.version},edits});
    }
    if(snapshot.current()!==snapshot.generation) throw new Error('Workspace edit changed during normalization');
    return {documentChanges};
  }
  function row(value,call,only) {
    if(!value||typeof value!=='object') throw new Error('Invalid code action');
    const result={title:options.text(value.title,1024)};
    const kind=value.kind,isPreferred=value.isPreferred,disabled=value.disabled,command=value.command,edit=value.edit;
    if(kind!==undefined) { if(!(kind instanceof CodeActionKind)) throw new Error('Code action kind requires CodeActionKind'); result.kind=kind.value; }
    if(only && (!kind||!only.contains(kind))) return null;
    if(isPreferred!==undefined) {if(typeof isPreferred!=='boolean')throw new Error('Invalid code action preferred flag');result.isPreferred=isPreferred;}
    if(disabled!==undefined) result.disabled={reason:options.text(disabled.reason,4096)};
    if(command!==undefined) result.disabled={reason:edit!==undefined?'Combined extension edit-and-command actions are unsupported':'Extension code action commands are unsupported'};
    else if(edit!==undefined) {
      try {result.edit=workspaceEdit(edit,call.workspace);}catch(error){result.disabled={reason:String(error.message).slice(0,4096)};}
    } else if(!call.entry.resolves) result.disabled={reason:'Code action contains no supported edit'};
    return result;
  }
  function normalize(values,call,context) {
    const original=values===undefined||values===null?[]:array(values,300), result=[], originals=[]; let bytes=0;
    for(const value of original) {
      const before=result.length;
      try { const normalized=row(value,call,context?.only); if(normalized){result.push(normalized);originals.push(value);} }
      catch(error) { result.push({title:'Unsupported code action',disabled:{reason:String(error.message).slice(0,4096)}}); originals.push(value); }
      if(result.length>before)bytes+=Buffer.byteLength(JSON.stringify(result.at(-1)));
      if(bytes>1024*1024)throw new Error('Code action source exceeds1 MiB');
    }
    return {result,originals};
  }
  function retain(call,originals,result,origin) {
    purge(); for(const [id,item]of cached)if(item.entry===call.entry)cached.delete(id);
    const eligible=result.map((row,index)=>({row,index})).filter(({row})=>call.entry.resolves&&!row.edit&&!row.disabled);
    if(cached.size+eligible.length>2400)throw new Error('Code action handle limit exceeded2400');
    let bytes=0;for(const item of cached.values())bytes+=item.bytes;
    for(const {row}of eligible)bytes+=Buffer.byteLength(JSON.stringify(row));
    if(bytes>2*1024*1024)throw new Error('Code action cache exceeds2 MiB');
    const staged=[];
    for(const {row,index}of eligible) {if(nextHandle>=Number.MAX_SAFE_INTEGER)throw new Error('Code action handle exhausted');const handle=++nextHandle;
      staged.push([handle,{entry:call.entry,document:call.document,version:call.version,epoch:call.epoch,workspace:call.workspace,origin,original:originals[index],snapshot:JSON.stringify(row),bytes:Buffer.byteLength(JSON.stringify(row)),started:Date.now()}]);row._vscliCodeActionHandle=handle;}
    for(const pair of staged)cached.set(...pair);
  }
  function lookup(params,entry) {
    purge();const item=cached.get(params.handle);
    if(!item||item.entry!==entry||item.origin!==params.origin||item.version!==params.version||item.document!==options.document(params.document))throw new Error('Stale code action resolve handle');return item;
  }
  function resolved(value,item,call,handle) {
    // VS Code resolves edit/command; displayed metadata remains the cached row.
    const result=JSON.parse(item.snapshot),candidate=value??item.original,command=candidate.command,edit=candidate.edit;
    if(command!==undefined)result.disabled={reason:edit!==undefined?'Combined extension edit-and-command actions are unsupported':'Extension code action commands are unsupported'};
    else if(edit!==undefined){try{result.edit=workspaceEdit(edit,call.workspace);}catch(error){result.disabled={reason:String(error.message).slice(0,4096)};}}
    else result.disabled={reason:'Resolved code action contains no supported edit'};
    result._vscliCodeActionHandle=handle;return result;
  }
  return {args,normalize,retain,lookup,resolved,retire,clear,has:(id,item)=>{purge();return cached.get(id)===item;},purge,count:()=>{purge();return cached.size;},assertWorkspace};
}
module.exports={createActions,array,assertWorkspace};
