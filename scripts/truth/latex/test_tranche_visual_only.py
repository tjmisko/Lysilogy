"""Visual-only admission boundaries with positive, ambiguous and omitted roles."""
from copy import deepcopy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from annotations import canonical
from archive import read_archive, sha256
from test_tranche_visual import fixture as original_fixture
from tranche_visual_only import (FORMAT, OMITTED_METRICS, pointer, retained_original, retained_source,
                                 ARTIFACTS, REVIEW_FORMAT, associated_notes,
                                 visual_projection, visual_source, validate_visual_only)
from native_exports import native_projection
from parser import parse_project


def fixture():
    candidate, wrapped, raw, docs, _ = original_fixture(with_links=True)
    p, i = docs['primary'], docs['independent']
    p['objects'][0]['body_regions'] = [deepcopy(p['objects'][0]['body_region'])]
    i['objects'][0]['body_regions'] = [{'page': 1, 'region': deepcopy(i['objects'][0]['full_visual_body']['region'])}]
    geometry_row = {'primary_id': p['objects'][0]['id'], 'independent_id': i['objects'][0]['id'],
        'kind': 'table', 'printed_number': '1', 'page': 1,
        'original_primary_body': deepcopy(p['objects'][0]['body_regions']),
        'original_independent_body': deepcopy(i['objects'][0]['body_regions']),
        'body_pdf_points': {'x_min': 9, 'y_min': 12, 'x_max': 60, 'y_max': 42},
        'body_pixels_96dpi': [12, 16, 80, 56]}
    geometry = {'format': 'k1-manual-visual-region-reconciliation-v1', 'regions': [geometry_row]}
    files, _ = read_archive(raw)
    source_id = candidate['source_inventory']['objects'][0]['id']
    construction = {'visuals': [{'source_id': source_id, 'primary': '/objects/0', 'independent': '/objects/0',
        'region': {'page': 1, 'rect': {'x_min': 9, 'y_min': 12, 'x_max': 60, 'y_max': 42}},
        'body_basis': {'pointer': '/regions/0', 'record_sha256': sha256(canonical(geometry_row))}}],
        'source_visual_exclusions': [], 'complete_visual_counts': {'figure': 0, 'table': 1},
        'reviewed_absent_visual_kinds': ['figure']}
    data = [construction, p, i, candidate['source_inventory'], files, wrapped['index'], geometry]
    seal(data)
    return data


def seal(data):
    c, p, i, inventory, _, _, _ = data
    c['retained_inventory'] = {
        'primary': retained_original(p, {r['primary'] for r in c['visuals']}),
        'independent': retained_original(i, {r['independent'] for r in c['visuals']}),
        'current_source': retained_source(inventory, {r['source_id'] for r in c['visuals']},
                                         {r['source_id']: r for r in c['source_visual_exclusions']})}


def admission_fixture():
    data = fixture(); candidate, wrapped, source_raw, old, images = original_fixture(with_links=True)
    c, p, i, parsed, files, index, geometry = data
    docs = {key: {} for key in ARTIFACTS}
    docs.update(primary=p, independent=i, packet=old['packet'], prompt=old['prompt'], geometry=geometry,
                source_export=old['source_export'], native_export=native_projection(canonical(wrapped), candidate['paper_id']),
                primary_receipt=old['primary_receipt'], independent_receipt=old['independent_receipt'],
                assignment_proposal=old['assignment_proposal'], assignment_history=old['assignment_history'],
                construction=c, supplement_history={'status':'retained_unscored_additive_evidence','artifacts':[]})
    docs['primary_receipt']['protocol']['all_original_pages_viewed_before_source_and_native'] = True
    docs['independent_receipt'].update(annotator='synthetic-second', all_original_pages_before_source_native=True)
    docs['identity_review'] = {'format':'k1-manual-producer-map-review-v1',
        'verdict':'clear_grounded_original_producers','findings':[], 'reviewer':'synthetic-third',
        'confirmed_producers':{'primary':'synthetic-first','independent':'synthetic-second'}}
    history = {'retained_path':'history/current.md','sha256':'f'*64,'bytes':10,
               'git_commit':'a'*40,'git_path':'docs/knowledge-base-phases.md'}
    docs['assignment_request'] = {'git_history':history}
    docs['review'] = {'format':REVIEW_FORMAT,'verdict':'clear_complete_visual_only_projection','findings':[],
        'annotators':{'primary':'synthetic-first','independent':'synthetic-second'},'reviewer':'synthetic-third',
        'pages_covered_by_original_review':[1], 'original_page_review_basis':'Separate synthetic fixture reviewer.',
        'later_crosswalk_reviewed_after_annotation_freeze':True,'omitted_metrics':OMITTED_METRICS.copy()}
    candidate['metric_eligibility'] = {f'O{n}':False for n in range(1,12)}
    return candidate, wrapped, source_raw, docs, images


def seal_admission(data):
    candidate, wrapped, source_raw, docs, images = data
    def raw(name): return docs[name].encode() if name == 'prompt' else canonical(docs[name])
    def ref(name):
        value=raw(name);return {'sha256':sha256(value),'bytes':len(value)}
    ledger = [{'path':name,**ref(name)} for name in ('packet','source_export','native_export','prompt')]
    ledger += deepcopy(docs['packet']['images'])
    docs['primary_receipt'].update(inputs=deepcopy(ledger),inventory=ref('primary'))
    docs['independent_receipt'].update(inputs=deepcopy(ledger), inventory=ref('independent'))
    request = docs['assignment_request']
    request.update(assignment_proposal=ref('assignment_proposal'), prior_running_assignment_record=ref('assignment_history'),
        papers=[{'arxiv_id':candidate['arxiv_id'],'original_artifacts':{name:ref(name) for name in
            ('primary','independent','primary_receipt','independent_receipt')},'phase_exact_hash_occurrences':[]}])
    docs['identity_review'].update(request=ref('assignment_request'),
        assignment_proposal_sha256=ref('assignment_proposal')['sha256'], assignment_history_sha256=ref('assignment_history')['sha256'],
        git_history=deepcopy(request['git_history']),papers=[{**deepcopy(request['papers'][0]),'history_occurrences':[]}])
    construction = docs['construction']
    inventory = parse_project(docs['source_export']['text_members'])
    construction.update(not_an_original_annotator_input=True,
        candidate={'sha256':sha256(canonical(candidate)),'bytes':len(canonical(candidate))},
        original_artifacts={name:ref(name) for name in ('primary','independent','primary_receipt','independent_receipt','packet','source_export','native_export')},
        current_source_inventory=inventory,current_source_inventory_sha256=sha256(canonical(inventory)),changed_source_inventory_fields=[])
    seal([construction,docs['primary'],docs['independent'],inventory,None,None,None])
    docs['review'].update(artifacts={name:ref(name)['sha256'] for name in ARTIFACTS if name!='review'},
        candidate_sha256=sha256(canonical(candidate)),current_source_inventory_sha256=sha256(canonical(inventory)),
        complete_visual_counts=construction['complete_visual_counts'],source_visual_exclusions=construction['source_visual_exclusions'])
    history = {row['retained_path']:row['sha256'] for row in docs['assignment_history']['records']}
    history[request['git_history']['retained_path']]=request['git_history']['sha256']
    return {name:raw(name) for name in ARTIFACTS}, history


def admit(data):
    raws, history = seal_admission(data)
    return validate_visual_only(canonical(data[0]),canonical(data[1]),data[2],raws,data[4],history)


class VisualOnlyProjectionTests(unittest.TestCase):
    def test_should_dispatch_only_visual_projection_when_explicit_visual_only_manifest_is_selected(self):
        from manual import attach_declared_tranche
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'bundle').mkdir()
            (root/'bundle/manifest.json').write_bytes(canonical({'format':FORMAT}))
            with patch('tranche_visual_only.attach_visual_only',return_value={'only':'O1/O2'}) as selected, \
                    patch('tranche_visual.attach_visual',side_effect=AssertionError('formal negative codec used')):
                self.assertEqual(attach_declared_tranche(root,root,root,b'{}',{}, {},'bundle'),{'only':'O1/O2'})
                self.assertEqual(selected.call_count,1)

    def test_should_retain_attached_table_note_when_it_has_a_separate_original_source_role(self):
        text = r'\begin{table}Cell\begin{tablenotes}\footnotesize\item Note.\end{tablenotes}\end{table}'
        files={'main.tex':text}
        def src(a,b):return {'member':'main.tex','start':a,'end':b,'text':text[a:b]}
        env=src(0,len(text));start=text.index(r'\begin{tablenotes}');end=text.index(r'\end{tablenotes}')+len(r'\end{tablenotes}')
        full=src(start,end);visible=src(text.index('Note.'),text.index('Note.')+5)
        native={'start':5,'end':10,'text':'Note.','pages':[1]}
        p={'objects':[{'id':'p','kind':'table'}], 'ancillary_members':[{'id':'p-note','parent_id':'p','source':full,'native_members':[native]}]}
        i={'objects':[{'id':'i','kind':'table','source':env,'ancillary':[{'role':'table_note','source':visible,'native_members':[native]}]}]}
        claims=[{'primary':'/objects/0','independent':'/objects/0','source_id':'source-table','region':{'page':1}}]
        index={'text':'Cell\nNote.','pages':[{'number':1,'start':0,'end':10,'width':150,'height':150}]}
        result=associated_notes(p,i,claims,files,index)
        self.assertEqual(result[0]['parent'],'source-table')
        self.assertEqual(result[0]['metric_eligibility'],{})
        for case in ('missing_primary','other_owner','altered_native','additional_source_payload'):
            with self.subTest(case=case):
                a,b=deepcopy(p),deepcopy(i)
                if case=='missing_primary':a['ancillary_members']=[]
                elif case=='other_owner':a['ancillary_members'][0]['parent_id']='unrelated'
                elif case=='altered_native':b['objects'][0]['ancillary'][0]['native_members'][0]['text']='Other'
                else:
                    span=b['objects'][0]['ancillary'][0]['source'];span['end']-=1;span['text']=span['text'][:-1]
                with self.assertRaises(ValueError):associated_notes(a,b,claims,files,index)

    def test_should_require_separate_admission_when_pure_visual_projection_is_complete(self):
        data = admission_fixture(); result = admit(data)
        self.assertFalse(result['accepted'])
        self.assertEqual(result['metric_eligibility'], data[0]['metric_eligibility'])
        self.assertEqual(result['manual_visual_only_overlay']['metric_eligibility'], {'O1':True,'O2':True})
        self.assertNotIn('manual_object_overlay', result)

    def test_should_reject_self_review_when_all_producer_and_artifact_hashes_are_fresh(self):
        data = admission_fixture();data[3]['review']['reviewer']='synthetic-first'
        with self.assertRaisesRegex(ValueError,'construction reviewer'): admit(data)

    def test_should_reject_pending_review_when_all_original_visuals_are_complete(self):
        data = admission_fixture();data[3]['review']['verdict']='pending'
        with self.assertRaisesRegex(ValueError,'construction review is not clear'): admit(data)

    def test_should_reject_formal_eligibility_when_visual_construction_is_separately_reviewed(self):
        data = admission_fixture();data[3]['review']['omitted_metrics'].remove('O5')
        with self.assertRaisesRegex(ValueError,'grants unscored metrics'): admit(data)

    def test_should_reject_changed_current_inventory_when_original_automatic_candidate_is_preserved(self):
        data = admission_fixture();raws, history=seal_admission(data)
        c=data[3]['construction'];c['current_source_inventory']['coverage']['forged']=True
        c['current_source_inventory_sha256']=sha256(canonical(c['current_source_inventory']))
        raws['construction']=canonical(c)
        with self.assertRaisesRegex(ValueError,'current source inventory differs'):
            validate_visual_only(canonical(data[0]),canonical(data[1]),data[2],raws,data[4],history)

    def test_should_reject_lost_pages_first_evidence_when_receipt_hashes_are_resealed(self):
        data=admission_fixture();data[3]['independent_receipt']['all_original_pages_before_source_native']=False
        with self.assertRaisesRegex(ValueError,'pages-first assertion'): admit(data)

    def test_should_reject_mixed_overlay_when_a_visual_negative_already_has_formal_truth(self):
        data=admission_fixture();data[0]['manual_object_overlay']={'metric_eligibility':{'O5':True}}
        with self.assertRaisesRegex(ValueError,'cannot mix'): admit(data)

    def test_should_reject_original_image_replacement_when_dimensions_and_page_stay_equal(self):
        data=admission_fixture();key=next(iter(data[4]));data[4][key]='0'*64
        with self.assertRaisesRegex(ValueError,'image paths/hashes differ'): admit(data)

    def test_should_retain_reference_and_bibliography_roles_when_only_visuals_are_projected(self):
        data = fixture(); before = deepcopy(data)
        result = visual_projection(*data)
        self.assertEqual(data, before)
        self.assertEqual(result['metric_eligibility'], {'O1': True, 'O2': True})
        self.assertEqual(result['omitted_metrics'], [f'O{i}' for i in range(3, 12)])
        self.assertEqual(len(result['retained_inventory']['current_source']['links']), 3)
        self.assertTrue(all(r['disposition'] == 'retained_unscored_source'
                            for r in result['retained_inventory']['current_source']['links']))
        self.assertEqual(len(result['retained_inventory']['current_source']['entries']), 1)
        self.assertNotIn('references', result)

    def test_should_retain_formal_positive_and_unknown_target_when_visual_inventory_is_empty(self):
        data = fixture(); c, p, i, parsed, _, _, _ = data
        for original in (p, i):
            original['objects'] = [{'id': 'theorem', 'kind': 'statement', 'child_objects': ['proof'],
                                    'fidelity_gaps': ['original source rendering is unverified']},
                                   {'id': 'proof', 'kind': 'proof', 'parent_object': 'theorem'}]
            original['references'].append({'id': 'missing', 'kind': 'statement_reference', 'target': None,
                                            'ambiguity': 'The printed lemma does not exist.'})
        parsed['objects'] = [{**parsed['objects'][0], 'id': 'theorem', 'kind': 'statement'}]
        c.update(visuals=[], complete_visual_counts={'figure': 0, 'table': 0},
                 reviewed_absent_visual_kinds=['figure', 'table'])
        seal(data); result = visual_projection(*data)
        self.assertEqual(result['objects'], [])
        self.assertEqual(result['reviewed_absent_kinds'], ['figure', 'table'])
        self.assertEqual(result['omitted_metrics'], OMITTED_METRICS)
        roles = next(r for r in result['retained_inventory']['primary'] if r['pointer'] == '/objects')
        self.assertEqual(roles['collection_count'], 2)
        self.assertEqual(roles['records'][1]['original_roles']['parent_object'], 'theorem')

    def test_should_reject_omitted_source_visual_when_original_annotations_do_not_include_it(self):
        data = fixture(); extra = deepcopy(data[3]['objects'][0]); extra['id'] = 'unaccounted'
        data[3]['objects'].append(extra); seal(data)
        with self.assertRaisesRegex(ValueError, 'syntactic visual source role is unaccounted'):
            visual_projection(*data)

    def test_should_reject_omitted_original_visual_when_source_crosswalk_is_short(self):
        data = fixture(); data[0]['visuals'] = []; seal(data)
        with self.assertRaisesRegex(ValueError, 'original visual inventory is incomplete'):
            visual_projection(*data)

    def test_should_reject_reclassified_visual_when_only_aggregate_counts_still_match(self):
        data = fixture()
        for original in data[1:3]: original['objects'][0]['kind'] = 'figure'
        data[0]['complete_visual_counts'] = {'figure': 1, 'table': 0}; seal(data)
        with self.assertRaisesRegex(ValueError, 'per-occurrence visual source kind differs'):
            visual_projection(*data)

    def test_should_reject_reused_original_when_source_ids_are_distinct(self):
        data = fixture(); c = data[0]
        other = deepcopy(data[3]['objects'][0]); other['id'] = 'second'
        data[3]['objects'].append(other)
        c['visuals'].append({**deepcopy(c['visuals'][0]), 'source_id': 'second'}); seal(data)
        with self.assertRaisesRegex(ValueError, 'original visual occurrence is reused'):
            visual_projection(*data)

    def test_should_reject_forged_original_caption_when_fresh_record_hashes_are_supplied(self):
        data = fixture()
        data[2]['objects'][0]['caption_native']['text'] = 'altered original'; seal(data)
        with self.assertRaises(ValueError): visual_projection(*data)

    def test_should_reject_empty_or_cross_page_caption_when_numbers_look_valid(self):
        for case in ('empty', 'other_page'):
            with self.subTest(case=case):
                data = fixture()
                if case == 'empty':
                    data[1]['objects'][0]['native_caption_members'] = []
                    data[2]['objects'][0]['caption_native_members'] = []
                else:
                    data[0]['visuals'][0]['region']['page'] = 2
                    data[5]['pages'].append({'number': 2, 'start': len(data[5]['text']),
                                             'end': len(data[5]['text']), 'width': 150, 'height': 150})
                seal(data)
                with self.assertRaises(ValueError): visual_projection(*data)

    def test_should_reject_missing_formal_ledger_row_when_visual_membership_is_unchanged(self):
        data = fixture(); data[0]['retained_inventory']['current_source']['links'].pop()
        with self.assertRaisesRegex(ValueError, 'retained unscored roles'):
            visual_projection(*data)

    def test_should_reject_unverified_source_label_when_original_caption_agrees(self):
        data = fixture(); data[3]['unverified_label_names'] = ['tab:a']
        seal(data)
        with self.assertRaisesRegex(ValueError, 'ambiguous or unverified ownership'):
            visual_projection(*data)

    def test_should_reject_nonfinite_bool_and_extra_region_fields_when_body_is_reviewed(self):
        for case in ('bool_page', 'bool_coordinate', 'nan', 'infinity', 'extra', 'backwards'):
            with self.subTest(case=case):
                data = fixture(); region = data[0]['visuals'][0]['region']
                if case == 'bool_page': region['page'] = True
                elif case == 'bool_coordinate': region['rect']['x_min'] = True
                elif case == 'nan': region['rect']['x_min'] = float('nan')
                elif case == 'infinity': region['rect']['x_max'] = float('inf')
                elif case == 'extra': region['prediction'] = 'not original evidence'
                else: region['rect']['x_max'] = 1
                with self.assertRaises(ValueError): visual_projection(*data)

    def test_should_retain_all_empty_collections_when_original_role_lists_are_empty(self):
        value = {'objects': [], 'definitions': [], 'nonvisual': {'unknown': None}}
        output = retained_original(value, set())
        self.assertEqual([row['pointer'] for row in output], ['/definitions', '/nonvisual', '/objects'])
        self.assertEqual(output[0]['collection_count'], 0)

    def test_should_reject_changed_body_when_original_geometry_decision_is_unchanged(self):
        data = fixture(); data[0]['visuals'][0]['region']['rect']['x_max'] += 1
        with self.assertRaisesRegex(ValueError, 'projected body differs'):
            visual_projection(*data)

    def test_should_reject_geometry_for_another_object_when_its_fresh_hash_is_supplied(self):
        for field, value in [('primary_id', 'another'), ('independent_id', 'another'), ('printed_number', '2'), ('kind', 'figure')]:
            with self.subTest(field=field):
                data = fixture(); row = data[6]['regions'][0]; row[field] = value
                data[0]['visuals'][0]['body_basis']['record_sha256'] = sha256(canonical(row))
                with self.assertRaises(ValueError): visual_projection(*data)

    def test_should_reject_moved_original_box_when_reconciled_choice_is_still_valid(self):
        data = fixture(); data[1]['objects'][0]['body_regions'][0]['x_min'] += 1; seal(data)
        with self.assertRaisesRegex(ValueError, 'replaces primary body'):
            visual_projection(*data)

    def test_should_reject_changed_pixel_evidence_when_pdf_rectangle_is_unchanged(self):
        data = fixture(); row = data[6]['regions'][0]; row['body_pixels_96dpi'][0] += 1
        data[0]['visuals'][0]['body_basis']['record_sha256'] = sha256(canonical(row))
        with self.assertRaises(ValueError): visual_projection(*data)

    def test_should_reject_conflicting_primary_label_when_independent_and_parser_agree(self):
        data = fixture(); data[1]['objects'][0]['source_label'] = 'another'; seal(data)
        with self.assertRaisesRegex(ValueError, 'primary visual label roles differ'):
            visual_projection(*data)

    def test_should_retain_framed_equation_role_when_source_uses_a_figure_environment(self):
        data = fixture(); c, p, i, inventory, files, _, _ = data
        name = next(iter(files)); start = len(files[name]); equation = r'\begin{equation}x=1\end{equation}'
        source_text = r'\begin{figure}' + equation + r'\end{figure}'
        files[name] += source_text
        span = {'path': name, 'start': start, 'end': start + len(source_text)}
        inventory['objects'].append({'id': 'framed', 'kind': 'figure', 'labels': [], 'source_members': [span]})
        inner_start = start + len(r'\begin{figure}')
        inner = {'member': name, 'start': inner_start, 'end': inner_start + len(equation), 'text': equation}
        p['objects'].append({'id': 'equation', 'kind': 'equation', 'source_environment': inner})
        i['objects'].append({'id': 'equation', 'kind': 'equation', 'source': deepcopy(inner)})
        evidence = [{'artifact': side, 'pointer': '/objects/1', 'record_sha256': sha256(canonical(original['objects'][1]))}
                    for side, original in [('primary', p), ('independent', i)]]
        c['source_visual_exclusions'] = [{'source_id': 'framed', 'disposition': 'reviewed_printed_nonvisual',
            'source_members': [span], 'original_evidence': evidence, 'reason': 'Paired printed equation inside source float.',
            'source_role_evidence': {'primary_object': '/objects/1', 'independent_object': '/objects/1'}}]
        seal(data); result = visual_projection(*data)
        self.assertEqual(len(result['objects']), 1)
        self.assertEqual(result['source_visual_exclusions'][0]['disposition'], 'reviewed_printed_nonvisual')
        self.assertEqual(result['retained_inventory']['current_source']['objects'][1]['source_roles']['kind'], 'figure')
        c['source_visual_exclusions'][0]['source_role_evidence']['primary_object'] = '/objects/0'; seal(data)
        with self.assertRaisesRegex(ValueError, 'printed visual cannot be relabeled'):
            visual_projection(*data)

    def test_should_retain_nonrendered_source_when_exact_empty_macro_owns_the_float(self):
        data = fixture(); c, p, i, inventory, files, _, _ = data
        name = next(iter(files)); definition = r'\newcommand{\discard}[1]{}'
        define_start = len(files[name]); files[name] += definition
        wrapper_start = len(files[name]); image = r'\begin{figure}\caption{Unused}\end{figure}'
        wrapper = '\\discard{\n' + image + '\n}'
        files[name] += wrapper
        start = wrapper_start + len('\\discard{\n')
        span = {'path': name, 'start': start, 'end': start + len(image)}
        inventory['objects'].append({'id': 'not-printed', 'kind': 'figure', 'labels': [], 'source_members': [span]})
        i['source_semantics_notes'] = ['The recorded source consumer is empty; printed figure inventory excludes its payload.']
        def src(start, end): return {'member': name, 'start': start, 'end': end, 'text': files[name][start:end]}
        c['source_visual_exclusions'] = [{'source_id': 'not-printed', 'disposition': 'reviewed_nonrendered_source',
            'source_members': [span], 'reason': 'Original pages and source role separately reviewed.',
            'original_evidence': [{'artifact': 'independent', 'pointer': '/source_semantics_notes/0',
                                   'record_sha256': sha256(canonical(i['source_semantics_notes'][0]))}],
            'source_role_evidence': {'wrapper': src(wrapper_start, len(files[name])),
                                     'empty_macro_definition': src(define_start, wrapper_start)}}]
        seal(data); result = visual_projection(*data)
        self.assertEqual(len(result['objects']), 1)
        self.assertEqual(result['source_visual_exclusions'][0]['disposition'], 'reviewed_nonrendered_source')
        original = deepcopy(data)
        for case in ('reordered', 'extra_payload', 'changed_definition'):
            with self.subTest(case=case):
                altered = deepcopy(original); roles = altered[0]['source_visual_exclusions'][0]['source_role_evidence']
                if case == 'reordered': roles['wrapper'], roles['empty_macro_definition'] = roles['empty_macro_definition'], roles['wrapper']
                elif case == 'extra_payload': roles['wrapper']['start'] -= 1; roles['wrapper']['text'] = altered[4][name][roles['wrapper']['start']:roles['wrapper']['end']]
                else: roles['empty_macro_definition']['text'] = r'\newcommand{\discard}[1]{#1}'
                with self.assertRaises(ValueError): visual_projection(*altered)

    def test_should_resolve_only_canonical_pointers_when_original_keys_contain_slashes(self):
        value = {'a/b': [{'~': 'value'}]}
        self.assertEqual(pointer(value, '/a~1b/0/~0'), 'value')
        for path in ('a/b', '/a~1b/00', '/a~1b/-1', '/a~2b/0', '/a~1b/1'):
            with self.subTest(path=path), self.assertRaises(ValueError): pointer(value, path)

    def test_should_preserve_exact_source_expansion_when_a_visual_includes_one_text_member(self):
        before = r'\begin{figure}\include{plot}'
        after = r'\caption{Plot}\label{fig:a}\end{figure}'
        main = before + after; dependency = r'\begin{tikzpicture}drawing\end{tikzpicture}'
        files = {'main.tex': main, 'plot.tex': dependency}
        def src(name, start, end):
            return {'member': name, 'start': start, 'end': end, 'text': files[name][start:end]}
        env = src('main.tex', 0, len(main)); included = src('plot.tex', 0, len(dependency))
        p = {'source_members': [env, included]}
        i = {'source': env, 'source_inclusion': src('main.tex', len(r'\begin{figure}'), len(before)),
             'source_dependency_members': ['plot.tex']}
        parts = [{'path': 'main.tex', 'start': 0, 'end': len(r'\begin{figure}')},
                 {'path': 'plot.tex', 'start': 0, 'end': len(dependency)},
                 {'path': 'main.tex', 'start': len(before), 'end': len(main)}]
        self.assertEqual(visual_source({'source_members': parts}, p, i, files)[0], parts)
        for case in ('missing_dependency', 'reordered', 'shortened', 'extra_primary'):
            with self.subTest(case=case):
                a, b, current = deepcopy(p), deepcopy(i), deepcopy(parts)
                if case == 'missing_dependency': b['source_dependency_members'] = []
                elif case == 'reordered': current.reverse()
                elif case == 'shortened': current[1]['end'] -= 1
                else: a['source_members'].append(deepcopy(env))
                with self.assertRaises(ValueError): visual_source({'source_members': current}, a, b, files)


if __name__ == '__main__':
    unittest.main()
