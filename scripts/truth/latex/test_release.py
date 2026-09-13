"""Limited release scope, denominator and immutable publication boundaries."""
from copy import deepcopy
from pathlib import Path
import tempfile
import unittest

from release import build_release, evidence_paths, write_immutable


def fixture():
    objects = [{'id':kind,'kind':kind,'labels':[],'spans':[{'start':0,'end':1}], 'region':[{'page':1,'rect':{'x_min':0,'y_min':0,'x_max':1,'y_max':1}}]} for kind in ('equation','statement','proof','algorithm')]
    entry={'id':'entry','printed_key':'1','spans':[{'start':2,'end':3}], 'field_labels':{'title':'A label','first_author':'An Author','year':'2020'},'source_members':[], 'field_provenance':{name:{'native_span':{'start':2,'end':3},'source_member':{'path':'main.bbl','start':0,'end':1},'source_role':name,'root_source_role':{'command':name,'source_command_start':0,'source_command_end':1,'value':'DO NOT PUBLISH RAW SOURCE'}} for name in ('title','first_author','year')}}
    base={'accepted':False,'metric_eligibility':{f'O{i}':False for i in range(1,12)},'source_inventory_sha256':'a'*64,'manual_assembly':{'candidate_sha256':'b'*64},'pdf_sha256':'c'*64,'source_sha256':'d'*64,'index':{'path':'index.json','sha256':'e'*64},'stratum':{'category':'cs.AI','year':2020}}
    math={**deepcopy(base),'arxiv_id':'2001.00001','paper_id':'0123456789abcdef','manual_object_overlay':{'metric_eligibility':{f'O{i}':True for i in range(3,8)},'objects':objects,'references':[{'source_link':0,'target':'statement','start':5,'end':6}], 'associated_content':[],'non_object_references':[],'reviewed_absent_kinds':['figure','table'],'evidence_hashes':{}},'manual_bibliography_overlay':{'metric_eligibility':{f'O{i}':True for i in range(8,11)},'entries':[entry],'mentions':[{'start':7,'end':8,'target':'entry'}],'citation_groups':[],'evidence_hashes':{}}}
    figure={**deepcopy(base),'arxiv_id':'2101.00001','paper_id':'fedcba9876543210','manual_figure_table_overlay':{'metric_eligibility':{'O1':True,'O2':True},'objects':[{'id':'figure','kind':'figure','spans':[]},{'id':'table','kind':'table','spans':[]}],'evidence_hashes':{}},'independent_panel':{'identities':[{'agent_identity':str(i)} for i in range(3)],'panelists':[['a','b','c'] for _ in range(3)],'valid_ids':['a','b','c'],**{key:'f'*64 for key in ('packet_sha256','prompt_sha256','review_sha256','scoring_policy_sha256')},'selection_policy':'Explicit manual pilot'}}
    papers=[math,figure]
    config={'version':'k1-limited-v1','build_date':'2026-09-13','target_papers':500,'coverage_followup':'https://github.com/tjmisko/Lysilogy/issues/97','selection_bias':'Two manually selected complete papers; no generalization','publication_deviation':'Limited all-kind release; original target remains unmet','papers':[{'arxiv_id':row['arxiv_id'],'paper_id':row['paper_id'],'candidate':'external/'+row['arxiv_id']+'.json'} for row in papers]}
    inputs={'papers':[{'arxiv_id':row['arxiv_id'],'version':1,'stratum':row['stratum'],'pdf':{'sha256':row['pdf_sha256']},'source':{'sha256':row['source_sha256']}} for row in papers]}
    return papers,config,inputs,[]


class ReleaseTests(unittest.TestCase):
    def test_should_retain_scope_and_negatives_when_every_kind_has_reviewed_positive_truth(self):
        release,bibliography=build_release(*fixture())
        coverage=release['coverage'];self.assertEqual(coverage['target_papers'],500);self.assertFalse(coverage['target_met'])
        self.assertEqual(coverage['denominators']['O1'],{'truth_objects':2,'papers':2,'negative_papers':1})
        self.assertEqual(coverage['denominators']['O2']['all_annotated_truth_objects'],2)
        self.assertEqual(coverage['denominators']['O11']['panelist_top3_opportunities'],9)
        self.assertEqual(len(bibliography['papers']),1)
        self.assertNotIn('DO NOT PUBLISH RAW SOURCE',str(release));self.assertFalse(coverage['system_acceptance_claimed'])

    def test_should_reject_missing_positive_kinds_when_negative_cohorts_cannot_fill_them(self):
        rows=fixture();rows[0][1]['manual_figure_table_overlay']['objects'].pop()
        with self.assertRaisesRegex(ValueError,'positive example'):build_release(*rows)

    def test_should_reject_a_reduced_target_or_changed_cohort_when_the_release_is_limited(self):
        for mutate in (lambda rows:rows[1].update(target_papers=2),lambda rows:rows[1]['papers'][0].update(paper_id='other'),lambda rows:rows[1].update(selection_bias='')):
            rows=fixture();mutate(rows)
            with self.subTest(mutate=mutate),self.assertRaises(ValueError):build_release(*rows)

    def test_should_retain_pinned_versions_when_release_links_are_projected(self):
        rows=fixture();release,bibliography=build_release(*rows)
        self.assertEqual(release['papers'][0]['arxiv_version'],1)
        self.assertEqual(bibliography['papers'][0]['arxiv_url'],'https://arxiv.org/abs/2001.00001v1')
        rows[0][0]['arxiv_version']=2
        with self.assertRaisesRegex(ValueError,'candidate version differs'):build_release(*rows)
        rows=fixture();rows[0][0]['stratum']=['invented',1900]
        with self.assertRaisesRegex(ValueError,'stratum differs'):build_release(*rows)

    def test_should_require_all_external_documents_when_a_bundle_is_pinned(self):
        config={'inputs':'inputs.json','indexes':'indexes.json','automatic_builds':[],'papers':[{'candidate':'candidate.json','object_bundle':'objects'}],'evidence_sha256':{'inputs.json':'a','indexes.json':'b','candidate.json':'c'}}
        with self.assertRaisesRegex(ValueError,'pin every'):evidence_paths(config)
        for name in ('packet.json','root-v1.json','independent-v1.json','independent-v1-receipt.json','reconciliation-independent-v1.json'):config['evidence_sha256']['objects/'+name]='d'
        self.assertEqual(len(evidence_paths(config)),8)

    def test_should_preserve_published_bytes_when_repeated_or_conflicting_releases_are_requested(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)/'k1-limited-v1';payloads={'objects.json':b'one','bibliography.json':b'two'}
            write_immutable(output,payloads);write_immutable(output,payloads)
            with self.assertRaisesRegex(ValueError,'immutable'):write_immutable(output,{'objects.json':b'changed','bibliography.json':b'two'})
            self.assertEqual((output/'objects.json').read_bytes(),b'one')

    def test_should_reject_redirected_ancestors_when_no_release_directory_exists_yet(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);outside=root/'outside';outside.mkdir()
            (root/'truth').symlink_to(outside,target_is_directory=True)
            with self.assertRaisesRegex(ValueError,'ancestor is a symlink'):
                write_immutable(root/'truth/k1-limited-v1',{'objects.json':b'one'})
            self.assertEqual(list(outside.iterdir()),[])

    def test_should_reject_symlink_publication_when_an_existing_release_is_redirected(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);target=root/'target';target.mkdir();(root/'release').symlink_to(target,target_is_directory=True)
            with self.assertRaises(ValueError):write_immutable(root/'release',{'objects.json':b'one'})


if __name__=='__main__':unittest.main()
