#!/usr/bin/env python3
"""Conservative static mapping; presence never grants behavioral qualification."""
import collections,json,re
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
import sys
if len(sys.argv)>1:
 assert len(sys.argv)==3 and sys.argv[1]=='--root','usage: '+sys.argv[0]+' [--root AUDIT_DIRECTORY]'
 ROOT=Path(sys.argv[2]).resolve()
I=json.loads((ROOT/'stable-api-inventory.json').read_text());H=json.loads((ROOT/'host-static-inventory.json').read_text())
entries={};callbacks={};conditions={}
def evidence(file,needle):
 path=ROOT/'current/extension-host'/file;src=path.read_text();index=src.find(needle)
 assert index>=0,(file,needle)
 return {'file':str(path.relative_to(ROOT)),'line':src[:index].count('\n')+1,'needle':needle}
def add(owner,fields,file,condition=None,status='source_present_unqualified'):
 for name in fields:
  key='vscode.'+owner+'.'+name if owner else 'vscode.'+name
  entries[key]={'status':status,'evidence':[evidence(file,name)],'condition':condition}
# The extension-scoped facade, not the narrower unowned internal API.
for f in H['supported_facades']:
 if f['name'] in ['window','workspace','commands','extensions','TextEditor','TextEditorEdit','Extension','ExtensionContext']:
  for key in f['keys']:
   if not key['name'].startswith('_'):entries['vscode.'+f['name']+'.'+key['name']]={'status':'source_present_unqualified','evidence':[key['location']],'condition':None}
add('window',['showQuickPick','showInputBox'],'prompts.cjs')
add('window',['createOutputChannel','createStatusBarItem','createTreeView','registerTreeDataProvider'],'surfaces.cjs')
for registration,callback in re.findall(r"\['(register\w+)', '(provide\w+)'\]",(ROOT/'current/extension-host/providers.cjs').read_text()):
 add('languages',[registration],'providers.cjs','sessionOptions.languageProviders must be enabled')
 # Interface name differs for DocumentFormattingEditProvider.
 interface=registration.removeprefix('register')
 callbacks['vscode.'+interface+'.'+callback]={'status':'source_handled_callback_unqualified','evidence':[evidence('providers.cjs',registration)],'condition':'Opt-in language-provider host; result normalization and native consumers impose limits.'}
add('languages',['createDiagnosticCollection','getDiagnostics','onDidChangeDiagnostics'],'diagnostics.cjs')
callbacks['vscode.CompletionItemProvider.resolveCompletionItem']={'status':'source_handled_callback_unqualified','evidence':[evidence('providers.cjs','resolveCompletionItem')]}
callbacks['vscode.CodeActionProvider.resolveCodeAction']={'status':'source_handled_callback_unqualified','evidence':[evidence('providers.cjs','resolveCodeAction')]}
for name in ['getChildren','getTreeItem','onDidChangeTreeData']:
 callbacks['vscode.TreeDataProvider.'+name]={'status':'source_handled_callback_unqualified','evidence':[evidence('surfaces.cjs',name)],'condition':'Single-selection bounded native contributed trees.'}
add('ExtensionContext',['extension'],'api.cjs')
add('ExtensionContext',['globalState','workspaceState'],'memento.cjs')
add('Memento',['get','keys','update'],'memento.cjs')
add('WorkspaceConfiguration',['get','has','inspect','update'],'configuration.cjs')
add('OutputChannel',['name','append','appendLine','replace','clear','show','hide','dispose'],'surfaces.cjs')
add('StatusBarItem',['id','alignment','priority','name','text','tooltip','color','backgroundColor','command','show','hide','dispose'],'surfaces.cjs')
add('TreeView',['visible','selection','onDidChangeVisibility','onDidExpandElement','onDidCollapseElement','onDidChangeSelection','reveal','title','description','message','dispose'],'surfaces.cjs')
add('DiagnosticCollection',['name','set','delete','clear','get','has','forEach','dispose'],'diagnostics.cjs')
add('TextLine',['lineNumber','text','range','rangeIncludingLineBreak','firstNonWhitespaceCharacterIndex','isEmptyOrWhitespace'],'api-types.cjs')
add('CancellationToken',['isCancellationRequested','onCancellationRequested'],'cancellation.cjs')
add('WorkspaceFolder',['uri','name','index'],'api.cjs')
for key in ['vscode.WorkspaceConfiguration.update','vscode.TreeView.reveal']:
 entries[key]['status']='explicitly_unavailable';entries[key]['condition']='Exposed method rejects/throws instead of implementing this API.'
for key in ['vscode.WorkspaceEdit.createFile','vscode.WorkspaceEdit.renameFile','vscode.WorkspaceEdit.deleteFile']:
 entries[key]={'status':'explicitly_unavailable','evidence':[evidence('code-action-types.cjs',key.rsplit('.',1)[1])],'condition':'Records unsupported resource operation; consumer rejects the action.'}
# Only these modules are spread into the exposed public top-level object.
exported_types=set()
for file in ['api-types.cjs','provider-types.cjs','diagnostic-types.cjs']:
 src=(ROOT/'current/extension-host'/file).read_text()
 for export in re.findall(r'module\.exports\s*=\s*\{([^}]+)\}',src,re.S):
  exported_types.update(re.findall(r'\b[A-Za-z][A-Za-z0-9_]*\b',export))
exported_types.update(['StatusBarAlignment','TreeItemCollapsibleState','ThemeColor','TreeItem','EndOfLine','ViewColumn','ExtensionMode'])
exported_types.discard('TextDocument') # Exposed extra constructor is not public vscode.d.ts.
for name in exported_types:
 file=next((f for f in ['api-types.cjs','provider-types.cjs','diagnostic-types.cjs','surfaces.cjs','api.cjs'] if re.search(r'\b'+name+r'\b',(ROOT/'current/extension-host'/f).read_text())),None)
 entries['vscode.'+name]={'status':'source_present_unqualified','evidence':[evidence(file,name)],'condition':'Surface presence only; signatures, static properties, DTO consumption and overloads remain individually unqualified.'}
entries['vscode.version']={'status':'source_present_unqualified','evidence':[evidence('api.cjs',"version: '1.95.0'")],'condition':'Reports 1.95.0; stable audit targets 1.141.0. Do not raise this claim before compatibility policy is reviewed.'}
# AST class members plus inherited prototype members. Assignment is not result consumption.
def class_entry(name,member):
 cls=H['classes'].get(name)
 if not cls:return None
 found=cls['members'].get(member)
 if found:return {'status':'source_present_unqualified','evidence':[f['location'] for f in found],'condition':'Static member/property presence; no execution or overload qualification.'}
 return class_entry(cls['extends'],member) if cls['extends'] else None
# Class static kind constants are assigned through a property-definition loop.
kind_src=(ROOT/'current/extension-host/code-action-types.cjs').read_text()
for name in re.findall(r'\b(\w+):\s*\x27[^\x27]*\x27',kind_src.split('Object.defineProperty(CodeActionKind',1)[0].split('Object.entries({',1)[1]):
 entries['vscode.CodeActionKind.'+name]={'status':'source_present_unqualified','evidence':[evidence('code-action-types.cjs',name+':')],'condition':'Static CodeActionKind constant; no latest behavioral qualification.'}
# Top-level enum constants from frozen native facade.
for owner in exported_types:
 obj=H['constants'].get(owner)
 if obj:
  for name in obj['members']:
   entries['vscode.'+owner+'.'+name]={'status':'source_present_unqualified','evidence':[obj['location']],'condition':'Enumerated source member only; values/alias/deprecation not runtime-tested.'}
for owner,names in {'EndOfLine':['LF','CRLF'],'ViewColumn':['Active','Beside','One','Two','Three','Four','Five','Six','Seven','Eight','Nine'],'ExtensionMode':['Production','Development','Test'],'StatusBarAlignment':['Left','Right'],'TreeItemCollapsibleState':['None','Collapsed','Expanded']}.items():add(owner,names,'api.cjs' if owner in ['EndOfLine','ViewColumn','ExtensionMode'] else 'surfaces.cjs')
namespace_ids={d['id'] for d in I['declarations'] if d['kind']=='internal_module'}
exposed_namespaces={'vscode.window','vscode.workspace','vscode.commands','vscode.languages','vscode.extensions'}
concrete_interfaces={'vscode.TextDocument','vscode.TextLine','vscode.TextEditor','vscode.TextEditorEdit','vscode.ExtensionContext','vscode.Extension','vscode.Memento','vscode.WorkspaceConfiguration','vscode.OutputChannel','vscode.StatusBarItem','vscode.TreeView','vscode.DiagnosticCollection','vscode.CancellationToken','vscode.WorkspaceFolder'}
# Callback interfaces are public extension-created values, not runtime constructors.
callback_interfaces={key.rsplit('.',1)[0] for key in callbacks}
class_ids={d['id'] for d in I['declarations'] if d['kind']=='class_declaration'}
rows=[]
def status_for(record,is_declaration):
 key=record['id'];owner=record.get('owner',record.get('parent'));name=record['name']
 if key in entries:return entries[key]
 if key in callbacks:return callbacks[key]
 if is_declaration:
  kind=record['kind']
  if kind=='internal_module':return {'status':'namespace_partial_surface' if key in exposed_namespaces else 'missing_namespace'}
  if kind in ['interface_declaration','type_alias_declaration']:return {'status':'type_only_contract','condition':'Stable declaration retained; does not imply a runtime export. Related factory/provider may be absent.'}
  return {'status':'missing_runtime_surface'}
 if owner in namespace_ids or owner=='vscode':return {'status':'missing_runtime_surface'}
 if record['category']=='inline_type_member':return {'status':'nested_type_contract_unqualified'}
 if owner in concrete_interfaces or owner in class_ids:
  cls=owner.removeprefix('vscode.');found=class_entry(cls,name)
  if found:return found
  if owner in class_ids and name in H['classes'].get(cls,{}).get('members',{}):return class_entry(cls,name)
  if owner in class_ids and owner in entries and record['syntax']=='public_field_definition':
   return {'status':'dto_field_unqualified','condition':'Extensible host object can receive this property; no evidence of initialization or acceptance/consumption. Inspect normalizer before implementation claims.'}
  return {'status':'missing_concrete_member'}
 if owner in callback_interfaces:return {'status':'callback_contract_unhandled_or_unqualified','condition':'Provider type has partial handling; this declared member has no mapped callback consumption.'}
 if record['category']=='enum_member':return {'status':'missing_runtime_surface'}
 return {'status':'type_only_contract','condition':'Extension-supplied or event/options result shape; needs a concrete owning API contract before coverage.'}
for kind,items in [('declaration',I['declarations']),('member',I['members'])]:
 for item in items:
  rec={'inventory_kind':kind,'id':item['id'],'overload':item.get('overload'),**status_for(item,kind=='declaration')}
  # Every overload remains separately unqualified, never inferred from a name.
  rec['qualified']=False
  rows.append(rec)
result={'contract':'vscli-extension-api-static-audit-v1','version':'1.141.0','source_sha256':I['source_sha256'],'status_meaning':'Static source evidence only. Every qualified=false; no compatibility percentage is defined. Missing/type-only/DTO callbacks are distinct. Unsupported overloads are not granted by name presence.','status_counts':dict(collections.Counter(r['status'] for r in rows)),'rows':rows,'mapping_entries':entries,'callback_entries':callbacks,'exposed_namespace_names':sorted(exposed_namespaces),'missing_namespace_names':sorted(namespace_ids-exposed_namespaces),'extra_nonstandard_top_level_exports':['TextDocument']}
(ROOT/'coverage.json').write_text(json.dumps(result,indent=2,ensure_ascii=False,sort_keys=True)+'\n')
namespace_summary=[]
for ns in sorted(namespace_ids):
 ms=[r for r in rows if r['inventory_kind']=='member' and r['id'].rsplit('.',1)[0]==ns]
 namespace_summary.append({'namespace':ns,'declaration_count_including_overloads':len(ms),'statuses':dict(collections.Counter(r['status'] for r in ms))})
(ROOT/'namespace-summary.json').write_text(json.dumps(namespace_summary,indent=2,sort_keys=True)+'\n')
print(json.dumps(result['status_counts'],indent=2));print(json.dumps(namespace_summary,indent=2))
