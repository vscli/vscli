#!/usr/bin/env python3
"""Pure artifact verification; Python stdlib only; never executes source facades."""
import collections,hashlib,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
STATUSES={'source_present_unqualified','type_only_contract','missing_runtime_surface','missing_namespace','namespace_partial_surface','missing_concrete_member','dto_field_unqualified','callback_contract_unhandled_or_unqualified','source_handled_callback_unqualified','explicitly_unavailable','nested_type_contract_unqualified'}
def validate(inventory,coverage,provenance):
 assert inventory['source_sha256']==provenance['source_bindings'][inventory['source']]['sha256'],'declaration/source proof'
 assert inventory['parse_errors'] is False,'parser error'
 expected=collections.Counter(('declaration',d['id'],None) for d in inventory['declarations'])
 expected.update(('member',m['id'],m['overload']) for m in inventory['members'])
 actual=collections.Counter((r['inventory_kind'],r['id'],r['overload']) for r in coverage['rows'])
 assert actual==expected,'coverage inventory differs (missing/duplicate/unknown row)'
 assert all(r['status'] in STATUSES for r in coverage['rows']),'unknown status'
 assert all(r['qualified'] is False for r in coverage['rows']),'unqualified static audit claimed parity'
 for row in coverage['rows']:
  if row['status'] in {'source_present_unqualified','source_handled_callback_unqualified','explicitly_unavailable'}:
   assert row.get('evidence'),'presence/rejection lacks source proof'
   for proof in row['evidence']:
    assert proof['file'].startswith('current/extension-host/'),'evidence outside copied host'
    assert isinstance(proof['line'],int) and proof['line']>0,'invalid evidence location'
 assert dict(collections.Counter(r['status'] for r in coverage['rows']))==coverage['status_counts'],'count proof'
 assert provenance['commit']==provenance['stable_update_response']['version']==provenance['tag_ref_response']['object']['sha'],'immutable stable pin'
 assert provenance['version']==provenance['stable_update_response']['productVersion'],'stable release identity'
 return True
def verify_package(root=ROOT):
 provenance=json.loads((root/'provenance.json').read_text())
 for path,receipt in provenance['source_bindings'].items():
  data=(root/path).read_bytes()
  assert len(data)==receipt['bytes'] and hashlib.sha256(data).hexdigest()==receipt['sha256'],'upstream bytes changed: '+path
 inventory=json.loads((root/'stable-api-inventory.json').read_text());coverage=json.loads((root/'coverage.json').read_text())
 validate(inventory,coverage,provenance)
 for row in coverage['rows']:
  for proof in row.get('evidence',[]):
   lines=(root/proof['file']).read_text().splitlines()
   assert proof['line']<=len(lines),'evidence outside file'
   if 'needle' in proof:assert proof['needle'] in lines[proof['line']-1],'source needle missing'
 manifest_path=root/'MANIFEST.json'
 if manifest_path.exists():
  manifest=json.loads(manifest_path.read_text())
  for item in manifest['files']:
   path=item['path'];assert '..' not in Path(path).parts and not Path(path).is_absolute(),'unsafe manifest path'
   data=(root/path).read_bytes()
   assert len(data)==item['bytes'] and hashlib.sha256(data).hexdigest()==item['sha256'],'package bytes changed: '+path
 return {'declarations':len(inventory['declarations']),'members':len(inventory['members']),'qualified_rows':0,'version':provenance['version'],'commit':provenance['commit']}
if __name__=='__main__':print(json.dumps(verify_package(),indent=2))
