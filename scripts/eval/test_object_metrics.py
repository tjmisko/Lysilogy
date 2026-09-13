"""Offline independent arithmetic/provenance fixtures; no production score publication."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import struct
import unittest
from unittest.mock import patch
from types import SimpleNamespace

spec=importlib.util.spec_from_file_location('object_metrics',Path(__file__).with_name('object-metrics.py'))
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)


def rectangle(x0=0,y0=0,x1=10,y1=10):return dict(x_min=x0,y_min=y0,x_max=x1,y_max=y1)


def fixture():
    caption='Figure 1. An independently authored caption.'
    text=caption+' The text later mentions Figure 1.'
    n=len(caption)
    index={'text':text,'pages':[{'number':1,'start':0,'end':len(text),'width':100,'height':100,'provenance':'native'}]}
    paper={'paper_id':'1'*16,'arxiv_id':'2104.01511','arxiv_version':1,'pdf_sha256':'a'*64,'index':{'sha256':'b'*64},'metric_eligibility':{'O1':True,'O2':True},'reviewed_absent_kinds':['table'],'counts':{'figure':1},'objects':[{'id':'truth:1','kind':'figure','printed_label':'1','spans':[{'start':0,'end':n}],'region':[{'page':1,'rect':rectangle()}]}]}
    prediction={'id':'fig-1','kind':'figure','page':1,'label':'Figure 1','anchor':{'page':1,'start':0,'end':n},'region':rectangle(),'text':caption}
    artifact={'paper_id':paper['paper_id'],'reading_index_generation':'"'+'b'*64+'"','objects':[prediction]}
    return paper,artifact,index


class ObjectMetricTests(unittest.TestCase):
    def should_select_the_original_version_when_current_parser_and_detector_sources_have_changed(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);source=root/'src/source_index/figures.rs'
            source.parent.mkdir(parents=True);source.write_bytes(b'original detector')
            truth={'schema_version':1,'truth_set':'K1','origin':'arxiv-latex','version':'k1-limited-v1',
                   'provenance':{'implementation':{'src/source_index/figures.rs':m.digest(source.read_bytes())}}}
            calls=[]
            def replay(*args):
                calls.append(args);return truth,{'version':truth['version']},{'reproduced':True}
            adapter=SimpleNamespace(replay=replay)
            with patch.object(m,'truth_verifier',return_value=adapter):
                result=m.validate_truth(root,root/'cache',root/'corpus',root/'data',m.canonical(truth)+b'\n',True)
            self.assertEqual(calls[0][-1],'k1-limited-v1');self.assertTrue(result[2]['reproduced'])
            # No current parser file exists: the historical verifier owns replay.
            source.write_bytes(b'new detector')
            with patch.object(m,'truth_verifier',return_value=adapter):
                repeated=m.validate_truth(root,root/'cache',root/'corpus',root/'data',m.canonical(truth)+b'\n')
            self.assertEqual(repeated,(truth,{'version':truth['version']}));self.assertEqual(len(calls),2)
            # Actual prediction generations are checked separately by validate_derivation.

    def should_reject_foreign_labels_when_versioned_replay_returns_another_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);source=root/'src/source_index/figures.rs'
            source.parent.mkdir(parents=True);source.write_bytes(b'detector')
            truth={'schema_version':1,'truth_set':'K1','origin':'arxiv-latex','version':'k1-limited-v1',
                   'provenance':{'implementation':{'src/source_index/figures.rs':m.digest(source.read_bytes())}}}
            adapter=SimpleNamespace(replay=lambda *args:({**truth,'papers':['foreign']},{},{}))
            with patch.object(m,'truth_verifier',return_value=adapter):
                with self.assertRaisesRegex(ValueError,'labels or complete cohort differ'):
                    m.validate_truth(root,root/'cache',root/'corpus',root/'data',m.canonical(truth)+b'\n')

    def should_fingerprint_retained_modules_when_the_collector_uses_a_versioned_verifier(self):
        files=m.implementation_files(m.ROOT)
        self.assertIn('scripts/truth/latex/versioned.py',files)
        self.assertIn('eval/implementations/k1-limited-v1/manifest.json',files)
        self.assertIn('eval/implementations/k1-limited-v1/parser.py',files)
    def should_bind_graphics_receipts_when_the_source_factory_retains_traces(self):
        with tempfile.TemporaryDirectory() as directory:
            cache=Path(directory);tool=cache/'tool';tool.write_bytes(b'independent tool bytes')
            mocked=patch.object(m.shutil,'which',return_value=str(tool));mocked.start();self.addCleanup(mocked.stop)
            paper,artifact,index=fixture();raw=b'<synthetic trace />';sha=m.digest(raw)
            path=cache/'object-graphics-traces'/(sha+'.xml');path.parent.mkdir();path.write_bytes(raw)
            evidence={'version':1,'native_generation':artifact['reading_index_generation'],'pdf_sha256':paper['pdf_sha256'],
                'tool_sha256':m.digest(tool.read_bytes()),'tool_path':str(tool),'cache_key':'','generation':'','pages':[{'page':1,'status':'complete','trace_sha256':sha,'images':[rectangle()],'unsupported_images':0}]}
            evidence['cache_key']=m.digest(m.canonical([1,evidence['native_generation'],evidence['pdf_sha256'],evidence['tool_sha256']]))
            def seal(e):
                e['generation']='';basis=m.canonical(e).decode();e['generation']=m.digest(basis.encode());return basis
            basis=seal(evidence);artifact['graphics']=evidence
            row={'graphics_basis_json':basis,'graphics_traces':[{'page':1,'sha256':sha,'path':str(path)}]}
            # Pure receipt validation does not execute a tool or fetch a resource.
            self.assertEqual(m.validate_graphics(row,artifact,paper,index,cache)['trace_hashes'],{str(path):sha})
            for field,value in [('pdf_sha256','c'*64),('native_generation','foreign'),('cache_key','wrong'),('tool_path','/not-a-tool')]:
                changed=copy.deepcopy(artifact);changed['graphics'][field]=value
                changed_row={**row,'graphics_basis_json':seal(changed['graphics'])}
                with self.subTest(field=field),self.assertRaises(ValueError):m.validate_graphics(changed_row,changed,paper,index,cache)
            for changed_row in [{**row,'graphics_traces':[]},{**row,'graphics_traces':row['graphics_traces']*2},
                {**row,'graphics_traces':[{**row['graphics_traces'][0],'path':str(cache/'foreign.xml')}]}]:
                with self.assertRaises(ValueError):m.validate_graphics(changed_row,artifact,paper,index,cache)
            path.write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError,'trace bytes differ'):m.validate_graphics(row,artifact,paper,index,cache)

    def should_reject_graphics_inventory_drift_when_rehashed_page_evidence_is_incoherent(self):
        with tempfile.TemporaryDirectory() as directory,patch.object(m.shutil,'which',return_value=None):
            cache=Path(directory);paper,artifact,index=fixture()
            base={'version':1,'native_generation':artifact['reading_index_generation'],'pdf_sha256':paper['pdf_sha256'],
                'tool_sha256':None,'tool_path':None,'cache_key':m.digest(m.canonical([1,artifact['reading_index_generation'],paper['pdf_sha256'],None])),
                'generation':'','pages':[{'page':1,'status':'tool_unavailable','trace_sha256':None,'images':[],'unsupported_images':0}]}
            for mutate in [lambda e:e['pages'].append(e['pages'][0]),lambda e:e['pages'].clear(),
                lambda e:e['pages'][0].update(images=[rectangle()]),lambda e:e['pages'][0].update(status='complete'),
                lambda e:e['pages'][0].update(unsupported_images=-1)]:
                evidence=copy.deepcopy(base);mutate(evidence);basis=m.canonical(evidence).decode();evidence['generation']=m.digest(basis.encode());artifact['graphics']=evidence
                with self.assertRaises(ValueError):m.validate_graphics({'graphics_basis_json':basis,'graphics_traces':[]},artifact,paper,index,cache)

    def should_reject_changed_native_basis_when_a_new_detector_generation_is_measured(self):
        _,artifact,index=fixture()
        index.update({'schema_version':6,'tokens':[{'text':'independent','rects':[rectangle()],'provenance':'native'}],'objects':{'paragraph':[]},'gaps':[],'figures':[{'legacy':'ignored'}]})
        artifact['figure_detector_version']=m.DETECTOR_VERSION
        artifact['figure_detector_generation']=m.digest(('figures:'+str(m.DETECTOR_VERSION)+':'+artifact['reading_index_generation']).encode())
        row={'index_sha256':'b'*64,'native_schema_version':6,'native_basis_format':m.NATIVE_BASIS_FORMAT,'native_basis_sha256':m.native_basis_digest(index)}
        self.assertEqual(m.validate_derivation(row,artifact,index)['version'],m.DETECTOR_VERSION)
        for key in ['text','pages','tokens','objects','gaps']:
            altered=copy.deepcopy(index);altered[key]=None
            with self.subTest(key=key),self.assertRaisesRegex(ValueError,'native text, tokens, geometry or provenance differs'):
                m.validate_derivation(row,artifact,altered)
        for changes in [{'figure_detector_version':1},{'figure_detector_version':True},{'figure_detector_generation':'forged'}]:
            with self.subTest(changes=changes),self.assertRaises(ValueError):m.validate_derivation(row,{**artifact,**changes},index)
        for changes in [{'native_basis_format':'unknown'},{'native_schema_version':7}]:
            with self.subTest(changes=changes),self.assertRaises(ValueError):m.validate_derivation({**row,**changes},artifact,index)
        for changes in [{'schema_version':7},{'new_field':None}]:
            with self.subTest(changes=changes),self.assertRaises(ValueError):m.native_basis_digest({**index,**changes})
        changed_legacy={**index,'figures':[{'changed':'old predictions are not inputs'}]}
        self.assertEqual(m.native_basis_digest(index),m.native_basis_digest(changed_legacy))

    def should_match_independent_protocol_vectors_when_native_values_use_typed_floats(self):
        def expand(value):
            if type(value)is dict:
                if set(value)=={'$f32_bits'}:return struct.unpack('>f',bytes.fromhex(value['$f32_bits']))[0]
                return {key:expand(child) for key,child in value.items()}
            if type(value)is list:return [expand(child) for child in value]
            return value
        vectors=json.loads((m.ROOT/'eval/native-basis-vectors.json').read_bytes())['vectors']
        for row in vectors:
            with self.subTest(name=row['name']):self.assertEqual(m.native_value_digest(expand(row['value'])),row['sha256'])
        self.assertNotEqual(m.native_value_digest(0.0),m.native_value_digest(-0.0))
        for value in [float('nan'),float('inf'),1e300,0.1234567890123]:
            with self.subTest(value=value),self.assertRaises(ValueError):m.native_value_digest(value,normalize=True)
        for value in [1<<64,-(1<<63)-1]:
            with self.assertRaises(ValueError):m.native_value_digest(value)
        for value in [0.1,83.339,0.0000001]:
            self.assertEqual(m.native_value_digest(value,normalize=True),m.native_value_digest(struct.unpack('>f',struct.pack('>f',value))[0]))

    def should_use_cargo_selected_artifact_when_inherited_targets_point_elsewhere(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);source=root/'examples/object_metrics.rs';source.parent.mkdir();source.write_text('source')
            executable=root/'target/debug/examples/object_metrics';executable.parent.mkdir(parents=True);executable.write_bytes(b'executable')
            record={'reason':'compiler-artifact','target':{'name':'object_metrics','kind':['example'],'src_path':str(source)},'executable':str(executable)}
            observed=[]
            def compile_call(command,**kwargs):
                observed.append((command,kwargs));return SimpleNamespace(stdout=m.canonical(record)+b'\n',stderr=b'')
            with patch.object(m,'implementation_files',return_value=['examples/object_metrics.rs']),patch.object(m.Path,'home',return_value=root),patch.object(m.subprocess,'run',side_effect=compile_call),patch.dict(m.os.environ,{'CARGO_TARGET_DIR':'/foreign','CARGO_BUILD_TARGET':'other'}):
                m.build_bridge(root)
            command,kwargs=observed[0];self.assertEqual(kwargs['env']['CARGO_TARGET_DIR'],str(root/'target'));self.assertNotIn('CARGO_BUILD_TARGET',kwargs['env']);self.assertIn('--offline',command)
            receipt=m.document((root/'target/object-metrics-build.json').read_bytes());self.assertEqual(receipt['executable_sha256'],m.digest(b'executable'))
            original=(root/'target/object-metrics-build.json').read_bytes();record['executable']='/foreign/object_metrics'
            with patch.object(m,'implementation_files',return_value=['examples/object_metrics.rs']),patch.object(m.Path,'home',return_value=root),patch.object(m.subprocess,'run',side_effect=compile_call):
                with self.assertRaisesRegex(ValueError,'another executable'):m.build_bridge(root)
            self.assertEqual((root/'target/object-metrics-build.json').read_bytes(),original)

    def should_measure_actual_scoring_path_when_independent_regions_overlap_by_one_third(self):
        p,a,i=fixture();a['objects'][0]['region']=rectangle(x0=5,x1=15)
        self.assertEqual(m.evaluate_paper(p,a,i)['region_values'],[1/3])

    def should_preserve_roman_identities_when_tables_use_printed_numerals(self):
        for roman in ['I','II','III','IV','V']:
            self.assertEqual(m.label('Table '+roman,'table'),roman.lower())
        self.assertNotEqual(m.label('Table I','table'),m.label('Table 1','table'))
        p,a,i=fixture();p['objects'][0].update(kind='table',printed_label='IV');p['counts']={'table':1};p['reviewed_absent_kinds']=['figure'];a['objects'][0].update(kind='table',label='Table IV')
        self.assertEqual(m.evaluate_paper(p,a,i)['tp'],1)
        a['objects'][0]['label']='Table 4';self.assertEqual(m.evaluate_paper(p,a,i)['tp'],0)

    def should_measure_exact_independent_iou_when_rectangles_overlap(self):
        self.assertEqual(m.iou([(0,0,10,10)],(5,0,15,10)),1/3)
        self.assertEqual(m.union_area([(0,0,10,10),(5,0,15,10)]),150)
        self.assertEqual(m.iou([(0,0,10,10),(5,0,15,10)],(0,0,15,10)),1)

    def should_penalize_duplicates_when_predictions_share_the_same_identity(self):
        p,a,i=fixture();a['objects']*=3
        r=m.evaluate_paper(p,a,i)
        self.assertEqual((r['tp'],r['fp'],r['fn']),(1,2,0))
        self.assertEqual(r['unmatched_prediction_positions'],[1,2])

    def should_keep_misses_in_iou_denominator_when_no_prediction_exists(self):
        p,a,i=fixture();a['objects']=[];r=m.evaluate_paper(p,a,i)
        self.assertEqual((r['tp'],r['fp'],r['fn']),(0,0,1))
        self.assertEqual(r['region_values'],[0]);self.assertEqual(r['matched_region_values'],[])

    def should_ignore_region_quality_for_matching_when_duplicate_captions_tie(self):
        p,a,i=fixture();a['objects'].append(copy.deepcopy(a['objects'][0]));a['objects'][0]['region']=None
        r=m.evaluate_paper(p,a,i)
        self.assertEqual(r['outcomes'][0]['prediction_position'],0)
        self.assertEqual(r['region_values'],[0]);self.assertEqual(r['fp'],1)

    def should_penalize_false_positives_when_the_complete_paper_is_negative(self):
        p,a,i=fixture();p.update(objects=[],counts={},reviewed_absent_kinds=['figure','table'])
        r=m.evaluate_paper(p,a,i);self.assertEqual((r['tp'],r['fp'],r['fn']),(0,1,0))
        self.assertEqual(r['region_values'],[])

    def should_reject_missing_inventory_when_negative_review_is_absent(self):
        p,a,i=fixture();p['reviewed_absent_kinds']=[]
        with self.assertRaisesRegex(ValueError,'negative-kind'):m.evaluate_paper(p,a,i)
        p['reviewed_absent_kinds']=['table'];p['counts']['figure']=2
        with self.assertRaisesRegex(ValueError,'inventory'):m.evaluate_paper(p,a,i)

    def should_leave_tied_truth_unmatched_when_caption_identity_is_ambiguous(self):
        p,a,i=fixture();p['objects'].append(copy.deepcopy(p['objects'][0]));p['objects'][1]['id']='truth:2';p['counts']['figure']=2
        r=m.evaluate_paper(p,a,i);self.assertEqual((r['tp'],r['fp'],r['fn']),(0,1,2));self.assertEqual(r['ambiguous_prediction_positions'],[0])

    def should_require_caption_membership_when_printed_identity_and_region_match(self):
        p,a,i=fixture();a['objects'][0]['anchor'].update(start=len(i['text'])-9,end=len(i['text'])-1)
        r=m.evaluate_paper(p,a,i);self.assertEqual((r['tp'],r['fp'],r['fn']),(0,1,1))

    def should_require_kind_and_label_when_source_membership_matches(self):
        for field,value in [('kind','table'),('label','Figure 2'),('page',2)]:
            p,a,i=fixture();a['objects'][0][field]=value
            r=m.evaluate_paper(p,a,i);self.assertEqual((r['tp'],r['fp'],r['fn']),(0,1,1))

    def should_use_zero_for_invalid_geometry_when_detection_is_correct(self):
        for region in [None,{},rectangle(x1=0),rectangle(x0=-1),rectangle(x1=101),rectangle(x1=float('nan')),rectangle(y1=float('inf'))]:
            p,a,i=fixture();a['objects'][0]['region']=region
            r=m.evaluate_paper(p,a,i);self.assertEqual(r['tp'],1);self.assertEqual(r['region_values'],[0])

    def should_keep_unknown_truth_coverage_when_regions_were_not_annotated(self):
        p,a,i=fixture();p['objects'][0].pop('region');p['metric_eligibility']['O2']=False
        r=m.evaluate_paper(p,a,i);self.assertEqual(r['tp'],1);self.assertEqual(r['unknown_truth_regions'],1);self.assertEqual(r['region_values'],[])

    def should_never_substitute_caption_boxes_when_body_geometry_differs(self):
        p,a,i=fixture();a['objects'][0]['region']=rectangle(y0=20,y1=25)
        self.assertEqual(m.evaluate_paper(p,a,i)['region_values'],[0])

    def should_reject_invalid_truth_geometry_when_the_annotation_is_malformed(self):
        p,a,i=fixture();p['objects'][0]['region'][0]['rect']=rectangle(x1=0)
        with self.assertRaisesRegex(ValueError,'truth geometry'):m.evaluate_paper(p,a,i)

    def should_include_all_pages_in_region_union_when_truth_spans_multiple_pages(self):
        p,a,i=fixture();i['pages'].append({'number':2,'start':len(i['text']),'end':len(i['text']),'width':100,'height':100,'provenance':'native'})
        p['objects'][0]['region'].append({'page':2,'rect':rectangle()})
        self.assertEqual(m.evaluate_paper(p,a,i)['region_values'],[.5])

    def should_reject_wrong_generation_when_pdf_index_or_paper_identity_differs(self):
        for field,value in [('paper_id','2'*16),('reading_index_generation','"other"')]:
            p,a,i=fixture();a[field]=value
            with self.assertRaisesRegex(ValueError,'generation'):m.evaluate_paper(p,a,i)

    def should_penalize_inconsistent_anchor_pages_when_prediction_page_and_offsets_agree(self):
        p,a,i=fixture();a['objects'][0]['anchor']['page']=2
        r=m.evaluate_paper(p,a,i);self.assertEqual((r['tp'],r['fp'],r['fn']),(0,1,1))
        self.assertEqual(r['invalid_caption_anchors'],{0:'invalid_caption_anchor'})
        self.assertEqual(r['region_values'],[0])

    def should_preserve_scalar_boundaries_when_caption_offsets_are_utf16(self):
        i={'text':'😀 xyz','pages':[{'number':1,'start':0,'end':6,'provenance':'native'}]}
        members,pages=m.utf16_members(i,[{'start':0,'end':2}]);self.assertEqual(members,{0,1})
        with self.assertRaisesRegex(ValueError,'split UTF-16'):m.utf16_members(i,[{'start':1,'end':2}])

    def should_reject_ocr_membership_when_native_caption_truth_is_required(self):
        p,a,i=fixture();i['pages'][0]['provenance']='ocr'
        with self.assertRaisesRegex(ValueError,'native page'):m.evaluate_paper(p,a,i)

    def should_reject_wrong_canonical_identity_when_registry_records_drift(self):
        p,_,_=fixture();root=Path('/corpus/pdf');name='2104.01511v1.pdf'
        original={'schema_version':1,'library_root':str(root),'unresolved':{},'records':{p['paper_id']:{'active':True,'relative_path':name,'content_hash':p['pdf_sha256']}}}
        m.validate_registry(original,[p],root)
        for change,value in [('content_hash','c'*64),('relative_path','other.pdf'),('active',False)]:
            changed=copy.deepcopy(original);changed['records'][p['paper_id']][change]=value
            with self.assertRaisesRegex(ValueError,'identity'):m.validate_registry(changed,[p],root)
        changed=copy.deepcopy(original);changed['unresolved']={'a'*64:[p['paper_id']]}
        with self.assertRaisesRegex(ValueError,'ambiguous'):m.validate_registry(changed,[p],root)

    def should_reject_duplicate_keys_or_nonfinite_json_when_evidence_is_malformed(self):
        for raw in [b'{"a":1,"a":2}',b'{"a":NaN}']:
            with self.assertRaises(ValueError):m.document(raw)

    def should_preserve_output_when_a_symlink_or_oversized_publication_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);saved=root/'saved.json';saved.write_text('original');link=root/'link';link.symlink_to(saved)
            with self.assertRaisesRegex(ValueError,'symlink'):m.atomic_json(link,{'replacement':True})
            self.assertEqual(saved.read_text(),'original')
            with self.assertRaisesRegex(ValueError,'symlink'):m.read(link)
            with self.assertRaisesRegex(ValueError,'oversized'):m.read(saved,cap=2)


def load_tests(loader, tests, pattern):
    return unittest.TestSuite(ObjectMetricTests(name) for name in dir(ObjectMetricTests) if name.startswith('should_'))


if __name__=='__main__':unittest.main()
