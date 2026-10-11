#!/usr/bin/env python3
import copy,json,unittest
from pathlib import Path
from verify import ROOT,validate,verify_package
class AuditIntegrity(unittest.TestCase):
 def setUp(self):
  self.i=json.loads((ROOT/'stable-api-inventory.json').read_text());self.c=json.loads((ROOT/'coverage.json').read_text());self.p=json.loads((ROOT/'provenance.json').read_text())
  self.assertTrue(validate(self.i,self.c,self.p))
 def test_complete_positive_baseline(self):
  self.assertEqual(verify_package()['version'],'1.141.0')
  namespaces={d['id'] for d in self.i['declarations'] if d['kind']=='internal_module'}
  self.assertEqual(len(namespaces),16);self.assertIn('vscode.lm',namespaces)
  self.assertTrue(any(m['category']=='inline_type_member' for m in self.i['members']))
  self.assertTrue(any(m['name']=='registerMcpServerDefinitionProvider' for m in self.i['members']))
  self.assertEqual(self.i['ambient_globals'][0]['name'],'Thenable')
 def test_missing_late_member_rejected(self):
  self.c['rows'].pop()
  with self.assertRaisesRegex(AssertionError,'coverage inventory differs'):validate(self.i,self.c,self.p)
 def test_duplicate_row_rejected(self):
  self.c['rows'].append(copy.deepcopy(self.c['rows'][0]))
  with self.assertRaisesRegex(AssertionError,'coverage inventory differs'):validate(self.i,self.c,self.p)
 def test_unknown_status_rejected(self):
  self.c['rows'][0]['status']='implemented'
  with self.assertRaisesRegex(AssertionError,'unknown status'):validate(self.i,self.c,self.p)
 def test_presence_cannot_claim_overload_parity(self):
  row=next(r for r in self.c['rows'] if r['status']=='source_present_unqualified');row['qualified']=True
  with self.assertRaisesRegex(AssertionError,'claimed parity'):validate(self.i,self.c,self.p)
 def test_presence_requires_evidence(self):
  row=next(r for r in self.c['rows'] if r['status']=='source_present_unqualified');row['evidence']=[]
  with self.assertRaisesRegex(AssertionError,'lacks source proof'):validate(self.i,self.c,self.p)
 def test_source_mismatch_rejected(self):
  self.i['source_sha256']='0'*64
  with self.assertRaisesRegex(AssertionError,'declaration/source proof'):validate(self.i,self.c,self.p)
 def test_latest_stable_pin_mismatch_rejected(self):
  self.p['stable_update_response']['version']='0'*40
  with self.assertRaisesRegex(AssertionError,'immutable stable pin'):validate(self.i,self.c,self.p)
 def test_absent_namespace_stays_absent_not_type_erasure(self):
  self.assertEqual(next(r for r in self.c['rows'] if r['inventory_kind']=='declaration' and r['id']=='vscode.debug')['status'],'missing_namespace')
  self.assertEqual(next(r for r in self.c['rows'] if r['inventory_kind']=='member' and r['id']=='vscode.WorkspaceConfiguration.update')['status'],'explicitly_unavailable')
  self.assertEqual(next(r for r in self.c['rows'] if r['inventory_kind']=='declaration' and r['id']=='vscode.TabGroups')['status'],'type_only_contract')
if __name__=='__main__':unittest.main()
