/*---------------------------------------------------------------------------------------------
 * Copyright (c) Microsoft Corporation. All rights reserved.
 * Licensed under the MIT License. Source-guided public tab object lifecycle:
 * https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostEditorTabs.ts
 * VSCLI adaptations: bounded full snapshots, original caller/private membership
 * proofs and deferred events after all native mirrors have been published.
 *--------------------------------------------------------------------------------------------*/
'use strict';
const {Uri,EventEmitter}=require('./api-types.cjs');
// Cache runtime constructors before exposing their mutable public exports.
// Atomic native publication must not call extension-replaced constructors.
const {TabInputText,TabInputTextDiff,TabInputCustom,TabInputWebview,TabInputNotebook,TabInputNotebookDiff,TabInputTerminal}=require('./tab-inputs.cjs');
const parseUri=Uri.parse.bind(Uri);
const MAX_GROUPS=4,MAX_TABS=128,MAX_METADATA=2*1024*1024;
function createEditorTabs({enabled=false,captureCaller,bindEvent,call}) {
  const groups=new Map(),tabs=new Map(),brands=new WeakMap(),facades=new Map(),plans=new WeakMap();
  const groupEvents=new EventEmitter(),tabEvents=new EventEmitter();
  let ordered=[],revision={},initialized=false;
  function prepare(dto){
    let bytes=0,total=0;
    function string(value,max,name){
      if(typeof value!=='string'||!value||value.length>max)throw new TypeError(`Invalid native tab ${name}`);
      const size=Buffer.byteLength(value);if(size>max||(bytes+=size+32)>MAX_METADATA)throw new RangeError('Native tab metadata budget exceeded');return value;
    }
    function bool(value){if(typeof value!=='boolean')throw new TypeError('Invalid native tab boolean');return value;}
    function input(value){
      if(!value||typeof value!=='object')throw new TypeError('Invalid native tab input');
      const kind=value.kind,result={kind};
      const uri=key=>{const serialized=string(value[key],4096,'URI');parseUri(serialized,true);result[key]=serialized;};
      switch(kind){
        case 'text':uri('uri');break;
        case 'textDiff':uri('original');uri('modified');break;
        case 'custom':uri('uri');result.viewType=string(value.viewType,128,'view type');break;
        case 'webview':result.viewType=string(value.viewType,128,'view type');break;
        case 'notebook':uri('uri');result.notebookType=string(value.notebookType,128,'notebook type');break;
        case 'notebookDiff':uri('original');uri('modified');result.notebookType=string(value.notebookType,128,'notebook type');break;
        case 'terminal':break;
        default:throw new TypeError('Unsupported native tab input kind');
      }
      return result;
    }
    if(!dto||!Array.isArray(dto.groups)||dto.groups.length<1||dto.groups.length>MAX_GROUPS)throw new RangeError('Native tab group count must be 1..4');
    const seenGroups=new Set(),seenTabs=new Set(),seenColumns=new Set();let activeGroups=0;
    const normalized=dto.groups.map(group=>{
      const id=string(group.id,128,'group ID'),viewColumn=group.viewColumn,isActive=bool(group.isActive);
      if(seenGroups.has(id)||seenColumns.has(viewColumn)||!Number.isInteger(viewColumn)||viewColumn<1||viewColumn>4)throw new Error('Invalid native tab group identity/column');
      seenGroups.add(id);seenColumns.add(viewColumn);activeGroups+=Number(isActive);
      if(!Array.isArray(group.tabs)||group.tabs.length>MAX_TABS||(total+=group.tabs.length)>512)throw new RangeError('Native tab count exceeds bounds');
      let activeTabs=0;
      const normalizedTabs=group.tabs.map(tab=>{
        const tabId=string(tab.id,128,'ID');if(seenTabs.has(tabId))throw new Error('Duplicate native tab ID');seenTabs.add(tabId);
        // Empty labels are valid API presentation; native IDs and resources are not.
        const label=tab.label;if(typeof label!=='string'||label.length>1024||Buffer.byteLength(label)>1024)throw new TypeError('Invalid native tab label');
        bytes+=Buffer.byteLength(label)+128;if(bytes>MAX_METADATA)throw new RangeError('Native tab metadata budget exceeded');
        const value={id:tabId,label,isActive:bool(tab.isActive),isDirty:bool(tab.isDirty),isPinned:bool(tab.isPinned),isPreview:bool(tab.isPreview),input:input(tab.input)};
        activeTabs+=Number(value.isActive);return value;
      });
      if(activeTabs!==(normalizedTabs.length?1:0))throw new Error('Native tab group active member is invalid');
      return {id,viewColumn,isActive,tabs:normalizedTabs};
    });
    if(activeGroups!==1)throw new Error('Native tab active group is invalid');
    const plan={};plans.set(plan,{revision,groups:normalized});return plan;
  }
  function makeInput(value){
    const uri=key=>parseUri(value[key],true);
    switch(value.kind){
      case 'text':return new TabInputText(uri('uri'));
      case 'textDiff':return new TabInputTextDiff(uri('original'),uri('modified'));
      case 'custom':return new TabInputCustom(uri('uri'),value.viewType);
      case 'webview':return new TabInputWebview(value.viewType);
      case 'notebook':return new TabInputNotebook(uri('uri'),value.notebookType);
      case 'notebookDiff':return new TabInputNotebookDiff(uri('original'),uri('modified'),value.notebookType);
      case 'terminal':return new TabInputTerminal();
    }
  }
  function groupRecord(data){
    const record={kind:'groups',id:data.id,data,alive:true,tabs:[],proof:{}};
    record.public=Object.freeze({get isActive(){return ordered.some(group=>group.id===record.id&&group.data.isActive);},get viewColumn(){return record.data.viewColumn;},
      get activeTab(){return record.tabs.find(tab=>tab.data.isActive)?.public;},get tabs(){return Object.freeze(record.tabs.map(tab=>tab.public));}});
    brands.set(record.public,record);return record;
  }
  function tabRecord(data,parent){
    const record={kind:'tabs',id:data.id,data,parent,alive:true,proof:{},input:makeInput(data.input)};
    record.public=Object.freeze({get isActive(){return record.parent.tabs.find(tab=>tab.data.isActive)?.id===record.id;},get label(){return record.data.label;},get input(){return record.input;},
      get isDirty(){return record.data.isDirty;},get isPinned(){return record.data.isPinned;},get isPreview(){return record.data.isPreview;},get group(){return record.parent.public;}});
    brands.set(record.public,record);return record;
  }
  function publish(plan){
    const prepared=plans.get(plan);if(!prepared||prepared.revision!==revision)throw new Error('Native tab snapshot plan retired');plans.delete(plan);
    const nextGroups=new Map(),nextTabs=new Map(),openedGroups=[],closedGroups=[],changedGroups=[],openedTabs=[],closedTabs=[],changedTabs=[];
    // Decode inputs before mutating any live object. Source DTOs are private
    // primitives, so this stage cannot evaluate extension-created getters.
    const stages=prepared.groups.map(data=>{
      const old=groups.get(data.id),record=old||groupRecord(data);
      const members=data.tabs.map(tab=>{
        const previous=tabs.get(tab.id),retained=previous?.parent===record?previous:undefined;
        const changed=!!retained&&JSON.stringify(retained.data)!==JSON.stringify(tab);
        return {data:tab,record:retained||tabRecord(tab,record),retained,changed,input:changed?makeInput(tab.input):undefined};
      });
      return {data,record,old,members,changed:!!old&&JSON.stringify(old.data)!==JSON.stringify(data)};
    });
    for(const stage of stages){
      const {data,record,old,members,changed}=stage;
      if(!old)openedGroups.push(record.public);else if(changed)changedGroups.push(record.public);
      // Close authority follows membership lifetime, independently of ordinary
      // dirty/label/focus metadata updates. Group batches retain ordered members.
      if(old&&JSON.stringify(old.tabs.map(tab=>tab.id))!==JSON.stringify(members.map(member=>member.data.id)))record.proof={};
      record.data=data;record.alive=true;
      record.tabs=members.map(member=>{
        const tab=member.record;if(!member.retained)openedTabs.push(tab.public);else if(member.changed)changedTabs.push(tab.public);
        tab.data=member.data;tab.alive=true;if(member.input)tab.input=member.input;nextTabs.set(tab.id,tab);return tab;
      });
      nextGroups.set(record.id,record);
    }
    for(const [id,old]of tabs)if(nextTabs.get(id)!==old){old.alive=false;old.proof={};closedTabs.push(old.public);}
    for(const [id,old]of groups)if(nextGroups.get(id)!==old){old.alive=false;old.proof={};closedGroups.push(old.public);}
    groups.clear();for(const [id,record]of nextGroups)groups.set(id,record);
    tabs.clear();for(const [id,record]of nextTabs)tabs.set(id,record);
    ordered=stages.map(stage=>stage.record);initialized=true;revision={};
    // Upstream freezes the event record, while its three arrays remain mutable.
    const event=(opened,closed,changed)=>Object.freeze({opened,closed,changed});
    const groupEvent=event(openedGroups,closedGroups,changedGroups),tabEvent=event(openedTabs,closedTabs,changedTabs);
    let fired=false;
    return ()=>{if(fired)return;fired=true;
      if(groupEvent.opened.length||groupEvent.closed.length||groupEvent.changed.length)groupEvents.fire(groupEvent);
      if(tabEvent.opened.length||tabEvent.closed.length||tabEvent.changed.length)tabEvents.fire(tabEvent);
    };
  }
  function forOwner(owner){
    if(facades.has(owner))return facades.get(owner);
    const original=captureCaller(owner);
    const requireState=()=>{if(!enabled||!initialized)throw new Error('Native tab inventory is not initialized');};
    const ownedEvent=source=>{
      const bound=bindEvent(owner,source);
      return function(...args){original();if(!enabled)throw new Error('Native tab inventory is not initialized');return Reflect.apply(bound,this,args);};
    };
    const facade=Object.freeze({get all(){requireState();return Object.freeze(ordered.map(group=>group.public));},
      get activeTabGroup(){requireState();return ordered.find(group=>group.data.isActive).public;},
      onDidChangeTabs:ownedEvent(tabEvents.event),onDidChangeTabGroups:ownedEvent(groupEvents.event),
      async close(value,preserveFocus){
        original();requireState();const guard=captureCaller(owner);
        // Capture the complete bounded private cohort before array element
        // access. A getter-triggered publication cannot borrow a replacement.
        const snapshot=new Map();for(const record of [...groups.values(),...tabs.values()])snapshot.set(record,{proof:record.proof,parent:record.parent});
        let selected;
        if(Array.isArray(value)){
          const length=value.length;if(!Number.isInteger(length)||length<0||length>512)throw new RangeError('Tab close count exceeds 512');
          selected=[];for(let index=0;index<length;index++)selected.push(value[index]);
        }else selected=[value];
        if(selected.length===0){guard();return true;}
        let kind;const ids=[],records=[];
        for(const object of selected){
          const record=brands.get(object),captured=record&&snapshot.get(record);
          if(!record||!captured||!record.alive||(kind&&kind!==record.kind))throw new Error('Invalid, retired or mixed tab close target');
          kind=record.kind;ids.push(record.id);records.push(record);
        }
        if(preserveFocus!==undefined&&typeof preserveFocus!=='boolean')throw new TypeError('Tab close preserveFocus must be boolean');
        guard();for(const record of records){const before=snapshot.get(record);
          if(!record.alive||record.proof!==before.proof||record.parent!==before.parent||(record.kind==='tabs'?tabs:groups).get(record.id)!==record)throw new Error('Tab close source changed during capture');
        }
        const result=await call(owner,'nativeTabsClose',{kind,ids,preserveFocus:preserveFocus??false});
        guard();
        if(!result||typeof result!=='object'||Array.isArray(result)||Object.keys(result).length!==3||
          !Object.hasOwn(result,'closed')||!Object.hasOwn(result,'kind')||!Object.hasOwn(result,'ids')||typeof result.closed!=='boolean'||result.kind!==kind||
          !Array.isArray(result.ids)||result.ids.length!==ids.length||result.ids.some((id,index)=>id!==ids[index]))throw new Error('Invalid native tab close acknowledgement');
        return result.closed;
      }});
    facades.set(owner,facade);return facade;
  }
  return {prepare,publish,forOwner,disposeOwner(owner){facades.delete(owner);}};
}
module.exports={createEditorTabs};
