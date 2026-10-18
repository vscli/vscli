#!/usr/bin/env python3
"""Source-only exhaustive stable declaration inventory; no JS execution.
Tree-sitter supplies complete syntax traversal. Static presence is never parity.
"""
import collections, hashlib, json, re
from pathlib import Path
from tree_sitter import Language, Parser
import tree_sitter_typescript, tree_sitter_javascript
ROOT=Path(__file__).resolve().parents[1]
import sys
if len(sys.argv)>1:
 assert len(sys.argv)==3 and sys.argv[1]=='--root','usage: '+sys.argv[0]+' [--root AUDIT_DIRECTORY]'
 ROOT=Path(sys.argv[2]).resolve()
TS=Parser(Language(tree_sitter_typescript.language_typescript()))
JS=Parser(Language(tree_sitter_javascript.language()))
def sha(data):return hashlib.sha256(data).hexdigest()
def text(src,n):return src[n.start_byte:n.end_byte].decode()
def walk(n):
 yield n
 for child in n.named_children:yield from walk(child)
def location(file,node):return {'file':file,'line':node.start_point.row+1,'end_line':node.end_point.row+1,'start_byte':node.start_byte,'end_byte':node.end_byte}
def name_of(src,node):
 name=node.child_by_field_name('name') or node.child_by_field_name('key')
 if name is not None:return text(src,name)
 if node.type in ['identifier','type_identifier','property_identifier','shorthand_property_identifier','shorthand_property_identifier_pattern']:return text(src,node)
 if node.type=='index_signature':return '[index]'
 if node.type=='call_signature':return '[call]'
 return '[anonymous]'
def declaration_inventory(file):
 src=(ROOT/file).read_bytes();tree=TS.parse(src);assert not tree.root_node.has_error
 declarations=[];members=[];inline=[];seen=set();containers={};serial=collections.Counter()
 module=next(n for n in walk(tree.root_node) if n.type=='module')
 def member_record(node,owner,category):
  name=name_of(src,node); key=owner+'.'+name;serial[key]+=1
  rec={'id':key,'overload':serial[key],'owner':owner,'name':name,'syntax':node.type,'category':category,
   'signature':text(src,node).strip(),'location':location(file,node)}
  rec['static']=bool(re.match(r'^\s*static\b',rec['signature']))
  rec['readonly']=bool(re.match(r'^\s*(static\s+)?readonly\b',rec['signature']))
  rec['optional']=bool(re.match(r'^\s*(?:readonly\s+)?[^(:?]+\?',rec['signature']))
  members.append(rec);seen.add((node.start_byte,node.end_byte));return rec
 def visit_body(body,scope):
  for outer in body.named_children:
   node=outer.child_by_field_name('declaration') if outer.type=='export_statement' else outer
   if node is None or node.type=='comment':continue
   if node.type=='expression_statement' and len(node.named_children)==1 and node.named_children[0].type=='internal_module':node=node.named_children[0]
   kind=node.type
   if kind in ['internal_module','interface_declaration','class_declaration','enum_declaration','type_alias_declaration']:
    name=name_of(src,node);qualified=scope+'.'+name;body2=node.child_by_field_name('body')
    rec={'id':qualified,'name':name,'parent':scope,'kind':kind,'explicit_export':outer.type=='export_statement','location':location(file,node),
     'header':text(src,node).split('{',1)[0].strip(),'sha256':sha(src[node.start_byte:node.end_byte])}
    rec['deprecated']='@deprecated' in text(src,outer.prev_named_sibling) if outer.prev_named_sibling and outer.prev_named_sibling.type=='comment' else False
    declarations.append(rec);containers[(node.start_byte,node.end_byte)]=qualified
    if kind=='internal_module':visit_body(body2,qualified)
    elif kind in ['class_declaration','interface_declaration']:
     for child in body2.named_children:
      if child.type in ['method_signature','property_signature','public_field_definition','call_signature','index_signature','construct_signature']:
       member_record(child,qualified,'class_member' if kind=='class_declaration' else 'interface_member')
      elif child.type!='comment':raise AssertionError(('unknown body declaration',qualified,child.type,text(src,child)))
    elif kind=='enum_declaration':
     for child in body2.named_children:
      if child.type=='comment':continue
      value=child.child_by_field_name('value')
      key=child.child_by_field_name('name') or child
      member_record(child,qualified,'enum_member')['name']=text(src,key)
      members[-1]['id']=qualified+'.'+text(src,key)
      members[-1]['value']=text(src,value) if value else None
    elif kind=='type_alias_declaration':pass
   elif kind=='function_signature':
    name=name_of(src,node);rec=member_record(node,scope,'namespace_function')
    declarations.append({'id':scope+'.'+name,'name':name,'parent':scope,'kind':kind,'explicit_export':outer.type=='export_statement','location':location(file,node),'signature':rec['signature']})
   elif kind=='lexical_declaration':
    for variable in node.named_children:
     if variable.type!='variable_declarator':continue
     name=name_of(src,variable);rec=member_record(variable,scope,'exported_value')
     declarations.append({'id':scope+'.'+name,'name':name,'parent':scope,'kind':kind,'explicit_export':outer.type=='export_statement','location':location(file,variable),'signature':rec['signature']})
   else:raise AssertionError(('unknown namespace declaration',kind,text(src,node)[:200]))
 visit_body(module.child_by_field_name('body'),'vscode')
 # Include every anonymous nested object signature, not just named declarations.
 for node in walk(module):
  if node.type not in ['method_signature','property_signature','call_signature','index_signature','construct_signature'] or (node.start_byte,node.end_byte) in seen:continue
  ancestor=node.parent;owner=None
  while ancestor is not None:
   if (ancestor.start_byte,ancestor.end_byte) in containers:
    owner=containers[(ancestor.start_byte,ancestor.end_byte)];break
   ancestor=ancestor.parent
  if owner is None:
   # Inline members in namespace functions/values need exact nearest export.
   ancestor=node.parent
   while ancestor is not None:
    if ancestor.type in ['function_signature','variable_declarator']:
     owner='vscode.$inline.'+name_of(src,ancestor);break
    ancestor=ancestor.parent
  rec=member_record(node,(owner or 'vscode')+f'.$inline@{node.start_point.row+1}:{node.start_point.column}','inline_type_member')
  inline.append(rec['id'])
 globals_=[{'name':name_of(src,n),'syntax':n.type,'location':location(file,n),'declaration':text(src,n)} for n in tree.root_node.named_children if n.type=='interface_declaration']
 expected=dict(collections.Counter(n.type for n in walk(module) if n.type in ['internal_module','interface_declaration','class_declaration','enum_declaration','type_alias_declaration','function_signature','variable_declarator']))
 actual=dict(collections.Counter(d['kind'] for d in declarations));actual['variable_declarator']=actual.pop('lexical_declaration',0)
 assert expected==actual,(expected,actual)
 return {'source':file,'source_sha256':sha(src),'parse_errors':False,'syntax_coverage':expected,'ambient_globals':globals_,'declarations':declarations,'members':members,'inline_member_ids':inline}

def js_inventory():
 classes={}; constants={}; objects=[]; source_errors=[]
 for file in sorted((ROOT/'current/extension-host').glob('*.cjs')):
  if file.name.endswith('.test.cjs'):continue
  src=file.read_bytes();tree=JS.parse(src);assert not tree.root_node.has_error,file
  relative=str(file.relative_to(ROOT))
  def evidence(node):return location(relative,node)
  for node in walk(tree.root_node):
   if node.type=='class_declaration':
    name=name_of(src,node);entry={'name':name,'location':evidence(node),'members':{},'extends':None}
    heritage=next((c for c in node.named_children if c.type=='class_heritage'),None)
    if heritage:entry['extends']=text(src,heritage).removeprefix('extends ').strip()
    def add(n,k,mode):
     if not k.startswith('_'):entry['members'].setdefault(k,[]).append({'mode':mode,'location':evidence(n)})
    for child in node.child_by_field_name('body').named_children:
     if child.type=='method_definition':add(child,name_of(src,child),'method_or_accessor')
    for child in walk(node.child_by_field_name('body')):
     if child.type=='assignment_expression':
      left=child.child_by_field_name('left')
      if left and left.type=='member_expression':
       obj=left.child_by_field_name('object');prop=left.child_by_field_name('property')
       if obj and text(src,obj)=='this' and prop:add(child,text(src,prop),'instance_assignment')
     if child.type=='call_expression':
      fun=child.child_by_field_name('function');args=child.child_by_field_name('arguments')
      if fun and text(src,fun)=='Object.assign' and len(args.named_children)>=2 and text(src,args.named_children[0])=='this':
       for obj in args.named_children[1:]:
        if obj.type=='object':
         for prop in obj.named_children:
          if prop.type in ['pair','shorthand_property_identifier']:add(prop,name_of(src,prop) if prop.type=='pair' else text(src,prop),'assigned_object_field')
    classes[name]=entry
   elif node.type=='variable_declarator':
    name=name_of(src,node);value=node.child_by_field_name('value')
    if value and value.type=='call_expression':
     args=value.child_by_field_name('arguments');fn=value.child_by_field_name('function')
     if args and args.named_children and fn:
      first=args.named_children[0]
      if text(src,fn)=='enumeration' and first.type=='array':
       constants[name]={'location':evidence(node),'members':[text(src,n)[1:-1] for n in first.named_children if n.type=='string']}
      elif text(src,fn)=='Object.freeze' and first.type=='object':
       constants[name]={'location':evidence(node),'members':[text(src,c.child_by_field_name('key')) for c in first.named_children if c.type=='pair']}
   # Complete static supported facade keys, including methods explicitly rejecting.
   elif node.type=='call_expression':
    fn=node.child_by_field_name('function');args=node.child_by_field_name('arguments')
    if fn and text(src,fn)=='supported' and args and len(args.named_children)==2:
     label,obj=args.named_children
     if label.type=='string' and obj.type=='object':
      keys=[];spreads=[]
      for prop in obj.named_children:
       if prop.type in ['pair','method_definition','shorthand_property_identifier']:
        key=prop.child_by_field_name('key') or prop.child_by_field_name('name')
        keys.append({'name':text(src,key) if key else text(src,prop),'location':evidence(prop)})
       elif prop.type=='spread_element':spreads.append(text(src,prop))
      objects.append({'name':text(src,label)[1:-1],'location':evidence(node),'keys':keys,'spreads':spreads})
 return {'classes':classes,'constants':constants,'supported_facades':objects,'scope':'Static syntax only; no constructor/provider or wire execution.'}

def main():
 stable=declaration_inventory('upstream/vscode.d.ts');old=declaration_inventory('upstream/vscode-1.95.0.d.ts');host=js_inventory()
 for name,value in [('stable-api-inventory.json',stable),('baseline-1.95-api-inventory.json',old),('host-static-inventory.json',host)]:
  (ROOT/name).write_text(json.dumps(value,indent=2,ensure_ascii=False,sort_keys=True)+'\n')
 current_ids={d['id'] for d in stable['declarations']};old_ids={d['id'] for d in old['declarations']}
 current_members={m['id'] for m in stable['members'] if m['category']!='inline_type_member'};old_members={m['id'] for m in old['members'] if m['category']!='inline_type_member'}
 delta={'new_declarations':sorted(current_ids-old_ids),'removed_declarations':sorted(old_ids-current_ids),'new_named_members':sorted(current_members-old_members),'removed_named_members':sorted(old_members-current_members),'note':'Signatures are retained in complete inventories; this ID delta is not a semantic compatibility comparison.'}
 (ROOT/'stable-vs-1.95-delta.json').write_text(json.dumps(delta,indent=2,sort_keys=True)+'\n')
 print(json.dumps({'stable_declarations':len(stable['declarations']),'stable_members_including_overloads_inline':len(stable['members']),'stable_kinds':dict(collections.Counter(d['kind'] for d in stable['declarations'])),'stable_member_kinds':dict(collections.Counter(m['category'] for m in stable['members'])),'host_classes':len(host['classes']),'host_facades':len(host['supported_facades']),'new_declaration_ids':len(delta['new_declarations'])},indent=2))
if __name__=='__main__':main()
