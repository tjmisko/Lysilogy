"""Limited release scope, denominator and immutable publication boundaries."""
from copy import deepcopy
from pathlib import Path
import tempfile
import unittest

from release import build_release, evidence_paths, write_immutable, validate_automatic_reports, automatic_source_identity
from annotations import canonical
from archive import sha256


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


def automatic_fixture(cache):
    inputs={'papers':[{'arxiv_id':'2001.00001','stratum':['cs.AI',2020],'version':1,'pdf':{'path':'pdf/paper.pdf','sha256':'c'*64},'source':{'sha256':'d'*64}}]}
    config={'inputs':'inputs.json','indexes':'indexes.json','automatic_builds':[]}
    indexes={'papers':[{'relative_path':'paper.pdf','paper_id':'0123456789abcdef','pdf_sha256':'c'*64,'index':{'path':'papers/actual/index.json','sha256':'e'*64}}]}
    (cache/'indexes.json').write_bytes(canonical(indexes));(cache/'inputs.json').write_bytes(canonical(inputs))
    evidence={'inputs.json':sha256(canonical(inputs)),'indexes.json':sha256(canonical(indexes))}
    sources={'archive.py':b'def read(): return 1\n','tex.py':b'def render(): return 1\n','parser.py':b'def parse(): return 1\n','align.py':b'def align(): return 1\n','builder.py':b'IMPLEMENTATION = ["new"]\ndef derive(): return 1\n'}
    for label,head in [('historical','1234567'),('current','abcdef0')]:
        directory=cache/label;(directory/'modules').mkdir(parents=True);(directory/'papers').mkdir()
        modules={}
        for name,raw in sources.items():
            if name=='builder.py':raw=raw.replace(b'new',b'old')
            if label=='historical' and name=='align.py':raw=raw.replace(b'1',b'0')
            (directory/'modules'/name).write_bytes(raw);modules[name]=sha256(raw)
        inventory={'objects':[]};metrics={f'O{i}':False for i in range(1,12)}
        candidate={'arxiv_id':'2001.00001','paper_id':'0123456789abcdef','pdf_sha256':'c'*64,'source_sha256':'d'*64,'index':indexes['papers'][0]['index'],'stratum':['cs.AI',2020],'accepted':False,'bibliography_eligible':False,'metric_eligibility':metrics,'source_inventory':inventory,'source_inventory_sha256':sha256(canonical(inventory))}
        candidate_raw=canonical(candidate);(directory/'papers/paper.json').write_bytes(candidate_raw)
        summary={'arxiv_id':'2001.00001','stratum':['cs.AI',2020],'accepted':False,'candidate_path':'papers/paper.json','candidate_sha256':sha256(candidate_raw),'source_inventory_sha256':candidate['source_inventory_sha256'],'metrics':metrics}
        report={'head':head,'input_sha256':evidence['inputs.json'],'index_map_sha256':evidence['indexes.json'],'papers':1,'counts':{'parsed':1,'accepted':0,'bibliography_eligible':0},'eligible_metrics':{key:0 for key in metrics},'network_calls':0,'model_calls':0,'external_cost_usd':0,'final_k1_publication':False,'wall_seconds':1,'peak_rss_kib':1}
        runner=b'inert runner receipt';launch={'head':head,'modules':modules,'runner_sha256':sha256(runner)}
        for name,raw in [('report.json',canonical(report)),('papers.jsonl',canonical(summary)+b'\n'),('launch.json',canonical(launch)),('run.py',runner)]:
            (directory/name).write_bytes(raw);evidence[label+'/'+name]=sha256(raw)
        config['automatic_builds'].append({'label':label,'tested_head':head,'path':label+'/report.json','paper_summaries':label+'/papers.jsonl','launch':label+'/launch.json','runner':label+'/run.py'})
    return cache,config,inputs,evidence,sources


class ReleaseTests(unittest.TestCase):
    def test_should_preserve_prior_papers_and_attached_notes_when_visual_negative_cohort_is_appended(self):
        from test_tranche_visual import fixture as visual_fixture, seal
        from tranche_visual import validate_visual
        rows, config, inputs, history = fixture()
        before, before_bibliography = build_release(rows, config, inputs, history)
        candidate, native, source, docs, images = visual_fixture()
        candidate['metric_eligibility'] = {f'O{i}': False for i in range(1, 12)}
        assembled = validate_visual(canonical(candidate), canonical(native), source, seal(candidate, docs), images,
                                    {r['retained_path']: r['sha256'] for r in docs['assignment_history']['records']})
        assembled['manual_assembly'] = {'candidate_sha256': sha256(canonical(candidate))}
        rows.append(assembled)
        config['version'] = 'k1-limited-v3'
        config['papers'].append({'arxiv_id': candidate['arxiv_id'], 'paper_id': candidate['paper_id'], 'candidate': 'external/new-candidate.json'})
        inputs['papers'].append({'arxiv_id': candidate['arxiv_id'], 'version': 1, 'stratum': candidate['stratum'],
                                 'pdf': {'sha256': candidate['pdf_sha256']}, 'source': {'sha256': candidate['source_sha256']}})
        after, bibliography = build_release(rows, config, inputs, history)
        self.assertEqual(canonical(after['papers'][:-1]), canonical(before['papers']))
        self.assertEqual(canonical(bibliography['papers']), canonical(before_bibliography['papers']))
        paper = after['papers'][-1]
        self.assertEqual(paper['associated_content'], assembled['manual_object_overlay']['associated_content'])
        self.assertEqual(len(paper['associated_content'][0]['spans']), 2)
        self.assertTrue(all(paper['metric_eligibility'][f'O{i}'] for i in range(1, 8)))
        self.assertFalse(any(paper['metric_eligibility'][f'O{i}'] for i in range(8, 12)))
        self.assertEqual(after['coverage']['denominators']['O2']['all_annotated_truth_objects'], 3)
        self.assertEqual(after['coverage']['target_papers'], 500)
        self.assertFalse(after['coverage']['target_met'])

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

    def test_should_pin_explicit_tranche_manifest_when_its_nested_evidence_is_selected(self):
        config={'inputs':'inputs.json','indexes':'indexes.json','automatic_builds':[],
                'papers':[{'candidate':'candidate.json','tranche_bundle':'tranche'}],
                'evidence_sha256':{'inputs.json':'a','indexes.json':'b','candidate.json':'c'}}
        with self.assertRaisesRegex(ValueError,'pin every'):evidence_paths(config)
        config['evidence_sha256']['tranche/manifest.json']='d'
        self.assertEqual(evidence_paths(config),set(config['evidence_sha256']))
        config['papers'][0]['tranche_bundle']='other-version'
        with self.assertRaisesRegex(ValueError,'pin every'):evidence_paths(config)

    def test_should_bind_current_automatic_coverage_when_only_fingerprint_lists_changed(self):
        with tempfile.TemporaryDirectory() as directory:
            arguments=automatic_fixture(Path(directory))
            history=validate_automatic_reports(*arguments)
            self.assertEqual(history[1]['summary']['papers'],1)
            self.assertEqual(history[1]['summary']['counts']['parsed'],1)
            self.assertEqual(automatic_source_identity('builder.py',b'IMPLEMENTATION=[]\ndef derive(): return 1'),automatic_source_identity('builder.py',b'IMPLEMENTATION=["manual"]\ndef derive(): return 1'))

    def test_should_reject_historical_report_relabeling_when_current_coverage_is_required(self):
        with tempfile.TemporaryDirectory() as directory:
            arguments=automatic_fixture(Path(directory));config=arguments[1]
            config['automatic_builds'][1]={**config['automatic_builds'][0],'label':'current'}
            with self.assertRaisesRegex(ValueError,'report identities must differ'):validate_automatic_reports(*arguments)

    def test_should_reject_incomplete_or_fabricated_counts_when_full_automatic_denominators_are_required(self):
        for mutation in (lambda report:report.update(papers=0),lambda report:report.update(eligible_metrics={}),lambda report:report['counts'].update(parsed=0),lambda report:report.update(head='9999999')):
            with self.subTest(mutation=mutation),tempfile.TemporaryDirectory() as directory:
                arguments=automatic_fixture(Path(directory));path=Path(directory)/'current/report.json'
                import json
                report=json.loads(path.read_bytes());mutation(report);path.write_bytes(canonical(report));arguments[3]['current/report.json']=sha256(path.read_bytes())
                with self.assertRaises(ValueError):validate_automatic_reports(*arguments)

    def test_should_reject_changed_derivation_or_candidates_when_current_receipts_claim_old_bytes(self):
        for target in ('source','candidate'):
            with self.subTest(target=target),tempfile.TemporaryDirectory() as directory:
                arguments=automatic_fixture(Path(directory))
                if target=='source':arguments[4]['align.py']=b'def align(): return 99\n'
                else:(Path(directory)/'current/papers/paper.json').write_bytes(b'changed candidate')
                with self.assertRaises(ValueError):validate_automatic_reports(*arguments)

    def test_should_reject_foreign_automatic_artifacts_when_resealed_summaries_keep_the_same_metrics(self):
        for field,value in [('paper_id','foreign-id'),('pdf_sha256','f'*64),('source_sha256','f'*64),('index',{'path':'foreign/index.json','sha256':'e'*64}),('arxiv_version',2)]:
            with self.subTest(field=field),tempfile.TemporaryDirectory() as directory:
                import json
                arguments=automatic_fixture(Path(directory));candidate_path=Path(directory)/'current/papers/paper.json'
                candidate=json.loads(candidate_path.read_bytes());candidate[field]=value;candidate_path.write_bytes(canonical(candidate))
                summary_path=Path(directory)/'current/papers.jsonl';summary=json.loads(summary_path.read_bytes());summary['candidate_sha256']=sha256(candidate_path.read_bytes());summary_path.write_bytes(canonical(summary)+b'\n')
                arguments[3]['current/papers.jsonl']=sha256(summary_path.read_bytes())
                with self.assertRaisesRegex(ValueError,'automatic candidate.*differs'):validate_automatic_reports(*arguments)

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
