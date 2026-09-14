"""Finite O3 admission tests with synthetic sources, producers and native defects."""
from copy import deepcopy
import unittest

from annotations import canonical
from archive import read_archive, sha256
from native_exports import native_projection
from parser import parse_project
from tranche_numbered_math import (ARTIFACTS, FORMAT, OMITTED_METRICS, PRODUCER_MODULES,
    REVIEW_FORMAT, original_endpoints, original_native, project_numbered, retention,
    source_scope, validate_numbered_math)


def binding(raw, path='fixture.json'):
    return {'path': path, 'bytes': len(raw), 'sha256': sha256(raw)}


def fixture():
    # The comment and formula exercise Latin-1 member versus UTF8 snippet hashes.
    source = ('% café\n\\documentclass{article}\n\\newtheorem{lemma}{Lemma}\n'
              '\\begin{document}\n\\begin{lemma}An unscored owner.\n'
              '\\begin{equation}x=é\\label{eq:x}\\end{equation}\n'
              '\\end{lemma}\n\\[z=w\\]\n\\end{document}')
    source_raw = source.encode('latin-1'); files, members = read_archive(source_raw)
    name = next(iter(files)); inventory = parse_project(files)
    equation = next(row for row in inventory['objects'] if row['kind'] == 'equation')
    parent = next(row for row in inventory['objects'] if row['kind'] == 'statement')
    body = '𝑥 � = é'; text = body + ' (1)'; end = len(body.encode('utf-16-le')) // 2
    wrapped = {'index': {'text': text, 'tokens': [], 'pages': [{'number': 1, 'start': 0,
        'end': len(text.encode('utf-16-le')) // 2, 'width': 96, 'height': 96}]}}
    native = lambda start, stop: {'start': start, 'end': stop, 'pages': [1],
        'text': text.encode('utf-16-le')[start*2:stop*2].decode('utf-16-le')}
    pm = [native(0, 2), native(3, 4), native(5, 6), native(7, 8)]
    for value in pm:
        value['sha256'] = sha256(value['text'].encode())
    number = native(end + 1, end + 4)
    extent = equation['source_members'][0]
    excerpt = files[name][extent['start']:extent['end']]
    psource = {'member': name, 'start': extent['start'], 'end': extent['end'],
               'text': excerpt, 'sha256': sha256(excerpt.encode())}
    isource = dict(psource, sha256=members[0]['sha256'])
    rectangle = lambda x: {'origin': 'top-left', 'unit': 'PDF points', 'page': 1,
                           'rect': {'x_min': float(x), 'y_min': 3., 'x_max': x+12., 'y_max': 12.}}
    p = {'id': 'eq-1', 'kind': 'equation', 'printed_label': '1', 'source': psource,
         'body_native_members': pm, 'printed_number_native_members': [number],
         'body_region': rectangle(3), 'printed_number_region': rectangle(75), 'parent': 'formal-owner'}
    i = {'id': 'eq-1', 'kind': 'numbered_equation', 'printed_label': '1', 'source': isource,
         'native_expression_members': [native(0, end)], 'native_number': number,
         'body_regions': [rectangle(3)], 'number_regions': [rectangle(75)]}
    primary = {'arxiv_id': 'synthetic', 'objects': [p, {'id': 'formal-owner', 'kind': 'statement'}],
               'unnumbered_displays': [{'source': r'\[z=w\]', 'role': 'unnumbered, unscored'}],
               'bibliography': [{'id': 'bib-unknown', 'title': None, 'uncertainty': 'unknown'}],
               'literal_references': [{'target_id': 'formal-owner', 'role': 'unscored reference'}]}
    independent = {'arxiv_id': 'synthetic', 'equations': [i], 'unknown_roles': ['No quote fidelity']}
    candidate = {'arxiv_id': 'synthetic', 'paper_id': 'a'*16, 'stratum': ['synthetic', 2020],
        'index': {'path': 'index.json', 'sha256': sha256(canonical(wrapped))}, 'pdf_sha256': 'b'*64,
        'source_sha256': sha256(source_raw), 'source_inventory': inventory,
        'source_inventory_sha256': sha256(canonical(inventory)), 'accepted': False,
        'metric_eligibility': {f'O{n}': False for n in range(1, 12)}}
    geometry = {'arxiv_id': 'synthetic', 'printed_label': '1'}
    for key, region, pixels in [('body', rectangle(3), [4, 4, 20, 16]),
                                ('printed_number', rectangle(75), [100, 4, 116, 16])]:
        geometry[key] = {'root_region': region, 'one_pixel_margin_pixels': pixels,
                         'original_proposals': [deepcopy(region), deepcopy(region)]}
    membership = {'primary_id': 'eq-1', 'independent_id': 'eq-1',
        'raw_record_hashes': {'primary': sha256(canonical(p)), 'independent': sha256(canonical(i))},
        'number_membership_exact': True, 'primary_only_non_whitespace_positions': [],
        'independent_only_non_whitespace_positions': []}
    evidence = {'synthetic:primary': primary, 'synthetic:independent': independent,
        'synthetic:candidate': candidate, 'geometry': {'rows': [geometry]}, 'membership': {'rows': [membership]},
        'printed': {'rows': [{'source_vs_printed_expression_identity': 'clear_for_this_complete_display',
                    'semantic_quote_truth': False, 'original_primary': p, 'original_independent': i}]},
        'synthetic:source': {'source_sha256': sha256(source_raw), 'text_members': files, 'members': members},
        'synthetic:native': native_projection(canonical(wrapped), candidate['paper_id'])}
    def ref(name, path, value):
        return {'artifact': name, 'pointer': path, 'record_sha256': sha256(canonical(value))}
    claim = {'primary': ref('synthetic:primary', '/objects/0', p),
        'independent': ref('synthetic:independent', '/equations/0', i),
        'automatic_source_object': ref('synthetic:candidate', '/source_inventory/objects/1', equation),
        'geometry': ref('geometry', '/rows/0', geometry), 'membership': ref('membership', '/rows/0', membership),
        'source_printed': ref('printed', '/rows/0', evidence['printed']['rows'][0]),
        'proposed_id': equation['id'], 'kind': 'equation', 'manual_source_member': extent,
        'manual_source_excerpt_sha256_utf8': sha256(excerpt.encode()),
        'automatic_source_members': equation['source_members'], 'source_relation': 'exact',
        'printed_number': '1', 'proposed_body_native_members': pm,
        'proposed_number_native_members': [number], 'proposed_body_region': rectangle(3),
        'proposed_number_region': rectangle(75), 'native_fidelity': 'Corrupted glyph retained; not a semantic quotation.'}
    paper = {'arxiv_id': 'synthetic', 'paper_id': candidate['paper_id'], 'rows': [claim],
        'complete_original_numbered_equation_counts': {'primary': 1, 'independent': 1, 'source_objects': 1}}
    scope = {'basis': 'complete_original_numbered_inventory_and_finite_source_scope',
        'automatic_guards_unchanged': True, 'retained_current_coverage': inventory['coverage'],
        'observations': [{'source_members': [{'path': name, 'start': 0, 'end': len(source),
            'text_sha256': sha256(source.encode())}], 'disposition': 'numbered_inventory',
            'reason': 'Synthetic whole source with distinct fixed visibility review.'}]}
    construction = {'format': FORMAT, 'producer': 'constructor', 'not_an_original_annotator_input': True,
        'current_source_inventory': inventory, 'current_source_inventory_sha256': sha256(canonical(inventory)),
        'changed_source_inventory_fields': [], 'source_scope': scope,
        'ownership': {equation['id']: {'source_parent_object': parent['id'], 'source_child_objects': []}},
        'retained_inventory': retention(primary, independent, inventory, [claim]),
        'original_endpoint_dispositions': original_endpoints(primary, independent, {}, 'synthetic'),
        'producer_modules': {name: 'c'*64 for name in PRODUCER_MODULES}}
    return {'candidate': candidate, 'wrapped': wrapped, 'source_raw': source_raw, 'files': files,
            'members': members, 'evidence': evidence, 'paper': paper, 'construction': construction}


def project(data):
    return project_numbered(data['paper'], data['evidence'], data['candidate']['source_inventory'],
        data['files'], data['members'], data['wrapped']['index'], data['construction'])


def seal(data):
    """Synthetic independent receipts, with no real annotator identity or admission."""
    e = data['evidence']; c = data['candidate']; build = data['construction']; name = next(iter(data['files']))
    raw = lambda value: canonical(value)
    docs = {key: {} for key in ARTIFACTS}; docs['prompt'] = 'Synthetic pages first.'
    image = {'path': 'page.png', 'bytes': 3, 'sha256': 'd'*64, 'page': 1,
             'dpi': 96, 'pixel_width': 128, 'pixel_height': 128}
    packet = {'arxiv_id': 'synthetic', 'paper_id': c['paper_id'], 'stratum': c['stratum'],
        'index': c['index'], 'source': {'sha256': c['source_sha256']}, 'pdf': {'sha256': c['pdf_sha256']},
        'images': [image], 'annotation_paths': {'primary': 'primary.json', 'independent': 'independent.json'}}
    docs.update(packet=packet, construction=build)
    originals = {'primary': 'first', 'independent': 'second'}
    inputs = {key: binding(raw(value), key+'.json') for key, value in [('rendered_packet', packet),
        ('source_export', e['synthetic:source']), ('native_export', e['synthetic:native'])]}
    inputs.update(prompt=binding(docs['prompt'].encode(), 'prompt.txt'), images=[image],
        source_member_inventory=data['members'], selection_sha256=sha256(raw(docs['selection'])),
        policy_sha256=sha256(raw(docs['policy'])))
    e['synthetic:primary_receipt'] = {'arxiv_id': 'synthetic', 'producer': 'first',
        'inventory': binding(raw(e['synthetic:primary']), 'primary.json'), 'inputs': inputs,
        'inspection_history': {'all_original_pages_viewed_before_source_and_native_transcription': True,
            'peer_labels_read': False, 'production_LaTeX_object_parser_used': False, 'pages_viewed': [1]}}
    e['synthetic:independent_receipt'] = {'arxiv_id': 'synthetic', 'annotator': 'second',
        'inventory': binding(raw(e['synthetic:independent']), 'independent.json'),
        'inputs': [inputs[key] for key in ('rendered_packet', 'source_export', 'native_export', 'prompt')] + [image],
        'pages_actually_viewed': [1], 'first_full_page_inspection_before_source_native': True}
    docs['assignment'] = {'primary_agent': 'first', 'independent_agent': 'second', 'packets': [
        {'arxiv_id': 'synthetic', 'packet': binding(raw(packet)), 'annotation_paths': packet['annotation_paths']}]}
    order = ['synthetic', 'synthetic-peer'] + ['remaining-' + str(n) for n in range(12)]
    docs['checkpoint'] = {'original_order': order, 'records': [{'arxiv_id': 'synthetic', 'role': side,
        'artifacts': {'inventory-v1.json': binding(raw(e['synthetic:'+side])),
            'receipt-v1.json': binding(raw(e['synthetic:'+side+'_receipt']))}} for side in originals]}
    docs['producer_history'] = {'format': 'k1-numbered-original-producer-closure-v1',
        'assignment_sha256': sha256(raw(docs['assignment'])), 'checkpoint_sha256': sha256(raw(docs['checkpoint'])),
        'confirmed_producers': originals, 'reviewer': 'history-reviewer', 'verdict': 'clear_original_producer_history'}
    docs['proposal'] = {'artifacts': {key: binding(raw(value)) for key, value in e.items()},
        'papers': [data['paper'], {'arxiv_id': 'synthetic-peer'}]}
    docs['matrix'] = {'rows': [{'arxiv_id': pid, 'historical_source_result': {'accepted': False},
                              'unknowns': ['Unscored original roles retained']} for pid in order]}
    docs['dispositions'] = {'original_order': order, 'rows': [{'arxiv_id': pid,
        'construction_scope': 'proposed_complete_O3' if n < 2 else 'retained_pending',
        'metric_eligibility_granted': [], 'retained_matrix_row': docs['matrix']['rows'][n],
        'original_checkpoint_records': [row for row in docs['checkpoint']['records'] if row['arxiv_id'] == pid]}
        for n, pid in enumerate(order)]}
    scope_row = {'arxiv_id': 'synthetic', 'member_inventory': data['members'],
        'current_source_inventory_sha256': c['source_inventory_sha256'],
        'source_scope': build['source_scope'], 'guard_literal_inventory': []}
    archive_binding = binding(data['source_raw'])
    docs['source_scope'] = {'producer': 'constructor', 'papers': [scope_row], 'bindings': [archive_binding]}
    observation = scope_row['source_scope']['observations'][0]['source_members'][0]
    docs['source_scope_review'] = {'packet': binding(raw(docs['source_scope'])),
        'verdict': 'clear_finite_source_scope_for_exact_two_frozen_inputs', 'findings': [],
        'truth_admission': False, 'metric_eligibility_granted': [], 'reviewer': 'scope-reviewer',
        'bindings': [archive_binding], 'module_bindings': [], 'rows': [{'arxiv_id': 'synthetic',
            'current_source_sha256': c['source_inventory_sha256'],
            'automatic_metric_flags_unchanged': c['metric_eligibility'], 'guard_judgments': {},
            'source_spans_verified': [{'pointer': '/source_scope/observations/0/source_members/0',
                                      'record_sha256': sha256(raw(observation))}], 'actual_complete_arguments': []}]}
    docs['proposal_review'] = {'proposal': binding(raw(docs['proposal'])),
        'endpoint_addendum': binding(raw(docs['endpoint_addendum'])), 'verdict': 'clear_within_mechanical_scope',
        'reviewer': 'mechanical-reviewer', 'producer': 'selector', 'findings': []}
    build.update(candidate_sha256=sha256(raw(c)), proposal_sha256=sha256(raw(docs['proposal'])))
    raws = {key: value.encode() if key == 'prompt' else raw(value) for key, value in docs.items()}
    docs['review'] = {'format': REVIEW_FORMAT, 'verdict': 'clear_complete_numbered_inventory', 'findings': [],
        'artifacts': {key: sha256(value) for key, value in raws.items() if key != 'review'},
        'proposal_artifacts': {key: sha256(raw(value)) for key, value in e.items()},
        'annotators': originals, 'reviewer': 'final-reviewer', 'construction_producer': 'constructor',
        'source_scope_sha256': sha256(raw(build['source_scope'])), 'complete_numbered_count': 1,
        'candidate_sha256': sha256(raw(c)), 'current_source_inventory_sha256': c['source_inventory_sha256'],
        'omitted_metrics': OMITTED_METRICS.copy(), 'later_construction_reviewed_after_original_freeze': True}
    raws['review'] = raw(docs['review'])
    return docs, raws, {key: raw(value) for key, value in e.items()}, {image['path']: image['sha256']}


def admit(data, mutate=None):
    docs, raws, evidence, images = seal(data)
    if mutate:
        mutate(docs)
        raws = {key: value.encode() if key == 'prompt' else canonical(value) for key, value in docs.items()}
    return validate_numbered_math(canonical(data['candidate']), canonical(data['wrapped']), data['source_raw'],
        raws, evidence, images, data['construction']['producer_modules'])


class NumberedMathTests(unittest.TestCase):
    def test_should_reject_empty_mathematical_support_when_both_originals_select_only_whitespace(self):
        data = fixture(); claim = data['paper']['rows'][0]
        p = data['evidence']['synthetic:primary']['objects'][0]
        i = data['evidence']['synthetic:independent']['equations'][0]
        whitespace = [{'start': 2, 'end': 3, 'text': ' ', 'pages': [1]}]
        p['body_native_members'] = deepcopy(whitespace)
        i['native_expression_members'] = deepcopy(whitespace)
        claim['proposed_body_native_members'] = deepcopy(whitespace)
        for role, row in [('primary', p), ('independent', i)]:
            claim[role]['record_sha256'] = sha256(canonical(row))
        membership = data['evidence']['membership']['rows'][0]
        membership['raw_record_hashes'] = {role: claim[role]['record_sha256'] for role in ('primary', 'independent')}
        claim['membership']['record_sha256'] = sha256(canonical(membership))
        claim['source_printed']['record_sha256'] = sha256(canonical(data['evidence']['printed']['rows'][0]))
        data['construction']['retained_inventory'] = retention(data['evidence']['synthetic:primary'],
            data['evidence']['synthetic:independent'], data['candidate']['source_inventory'], data['paper']['rows'])
        with self.assertRaisesRegex(ValueError, 'bodies disagree or are empty'):
            admit(data)

    def test_should_preserve_complete_display_when_automatic_source_is_a_reviewed_contained_occurrence(self):
        data = fixture(); claim = data['paper']['rows'][0]
        old = data['candidate']['source_inventory']['objects'][1]['source_members'][0]
        inner = dict(old, start=old['start'] + len(r'\begin{equation}'), end=old['end'] - len(r'\end{equation}'))
        data['candidate']['source_inventory']['objects'][1]['source_members'] = [inner]
        claim['automatic_source_members'] = [inner]
        claim['source_relation'] = 'reviewed_contained_automatic_occurrence_in_whole_manual_display'
        claim['automatic_source_object']['record_sha256'] = sha256(canonical(data['candidate']['source_inventory']['objects'][1]))
        data['construction']['retained_inventory'] = retention(data['evidence']['synthetic:primary'],
            data['evidence']['synthetic:independent'], data['candidate']['source_inventory'], data['paper']['rows'])
        objects, _ = project(data)
        self.assertEqual(objects[0]['source_members'], [old])
        claim['source_relation'] = 'exact'
        with self.assertRaisesRegex(ValueError, 'contained occurrence relation'):
            project(data)

    def test_should_count_only_numbered_records_when_release_projects_a_retained_formal_owner(self):
        from release import project_paper
        from release_layout import Summary
        data = fixture(); candidate = admit(data)
        candidate['manual_assembly'] = {'candidate_sha256': sha256(canonical(data['candidate']))}
        spec = {'arxiv_id': 'synthetic', 'paper_id': candidate['paper_id'], 'candidate': 'candidate.json'}
        frozen = {'synthetic': {'version': 1, 'stratum': candidate['stratum'],
            'pdf': {'sha256': candidate['pdf_sha256']}, 'source': {'sha256': candidate['source_sha256']}}}
        row = project_paper(candidate, spec, frozen)
        self.assertEqual(row['counts'], {'equation': 1, 'bib_entry': 0})
        self.assertEqual([key for key, value in row['metric_eligibility'].items() if value], ['O3'])
        self.assertEqual(row['objects'][0]['printed_number_spans'][0]['end'], 12)
        summary = Summary(); summary.add(row)
        self.assertEqual(summary.totals['equation'], 1)
        self.assertEqual(summary.totals['statement'], 0)
        candidate['manual_object_overlay'] = {}
        with self.assertRaisesRegex(ValueError, 'cannot mix'):
            project_paper(candidate, spec, frozen)

    def test_should_keep_unresolved_original_endpoint_when_only_a_peer_owner_exists(self):
        primary = {'literal_references': [{'destination_id': 'section-3', 'source': {'text': 'section 3'}}]}
        independent = {'sections': [{'id': 'section-3'}]}; original = primary['literal_references'][0]
        addendum = {'original_endpoint': {'artifact': 'primary', 'pointer': '/literal_references/0',
            'field': 'destination_id', 'literal_target': 'section-3', 'record_sha256': sha256(canonical(original)),
            'source_and_native_record_unchanged': deepcopy(original), 'status': 'unresolved_in_original_primary_inventory'}}
        result = original_endpoints(primary, independent, addendum, '1911.08525')
        self.assertIsNone(result[0]['owner_pointer'])
        self.assertEqual(result[0]['disposition'], 'unresolved_in_original_primary_inventory')
        primary['literal_references'][0]['destination_id'] = 'section-4'
        with self.assertRaisesRegex(ValueError, 'undeclared dangling'):
            original_endpoints(primary, independent, addendum, '1911.08525')

    def test_should_reject_another_source_printed_row_when_review_verdict_is_still_clear(self):
        data = fixture(); printed = data['evidence']['printed']['rows'][0]
        printed['original_primary'] = dict(printed['original_primary'], id='another-equation')
        data['paper']['rows'][0]['source_printed']['record_sha256'] = sha256(canonical(printed))
        with self.assertRaisesRegex(ValueError, 'source/printed review changes primary'):
            project(data)

    def test_should_reject_changed_producer_module_when_construction_review_is_unchanged(self):
        data = fixture(); _, raws, evidence, images = seal(data)
        implementation = dict(data['construction']['producer_modules'], **{'parser.py': 'f'*64})
        with self.assertRaisesRegex(ValueError, 'producer bytes changed'):
            validate_numbered_math(canonical(data['candidate']), canonical(data['wrapped']), data['source_raw'],
                raws, evidence, images, implementation)

    def test_should_retain_corruption_unnumbered_and_owner_when_complete_numbered_inventory_is_reviewed(self):
        data = fixture(); result = admit(data); overlay = result['manual_numbered_math_overlay']
        self.assertEqual(overlay['metric_eligibility'], {'O3': True})
        self.assertEqual(result['source_inventory'], data['candidate']['source_inventory'])
        self.assertFalse(result['accepted'])
        self.assertEqual(len(overlay['objects']), 1)
        self.assertEqual(overlay['objects'][0]['source_parent_object'], 'object:statement:1')
        self.assertFalse(overlay['objects'][0]['semantic_quote_truth'])
        self.assertEqual(overlay['objects'][0]['spans'][0]['end'], 2)
        ledger = overlay['retained_inventory']
        self.assertTrue(any(row['pointer'] == '/unnumbered_displays' for row in ledger['primary']))
        self.assertTrue(any(row['pointer'] == '/bibliography' for row in ledger['primary']))
        self.assertEqual(ledger['source']['objects'][0]['disposition'], 'retained_unscored_source')
        self.assertEqual(ledger['source_metadata']['label_targets'], data['candidate']['source_inventory']['label_targets'])
        self.assertEqual(len(overlay['fourteen_paper_dispositions']['rows']), 14)

    def test_should_reject_whole_paper_when_number_region_or_independent_membership_is_missing(self):
        for mutation in [lambda d: d['paper']['rows'].clear(),
            lambda d: d['paper']['rows'][0]['proposed_number_native_members'].clear(),
            lambda d: d['paper']['rows'][0]['proposed_body_region']['rect'].update(x_max=4),
            lambda d: d['evidence']['synthetic:independent']['equations'][0].pop('native_expression_members')]:
            with self.subTest(mutation=mutation), self.assertRaises((ValueError, KeyError)):
                data = fixture(); mutation(data); project(data)

    def test_should_reject_changed_source_symbol_and_hash_domain_when_review_is_only_a_flag(self):
        for mutation in [lambda d: d['files'].update({next(iter(d['files'])): next(iter(d['files'].values())).replace('x=é', 'x=à')}),
            lambda d: d['evidence']['synthetic:independent']['equations'][0]['source'].update(
                sha256=sha256(next(iter(d['files'].values())).encode())),
            lambda d: d['evidence']['synthetic:primary']['objects'][0]['source'].update(start=107)]:
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                data = fixture(); mutation(data); project(data)

    def test_should_reject_unknown_source_guard_when_historical_source_scope_is_unchanged(self):
        data = fixture(); current = deepcopy(data['candidate']['source_inventory'])
        current['coverage']['unsupported_source_semantics']['unknown_inventory_command:hidden'] = 1
        data['construction']['current_source_inventory'] = current
        data['construction']['current_source_inventory_sha256'] = sha256(canonical(current))
        data['construction']['changed_source_inventory_fields'] = ['coverage']
        with self.assertRaisesRegex(ValueError, 'entire historical inventory unchanged'):
            source_scope(current, data['candidate']['source_inventory'], data['files'], data['construction'])

    def test_should_reject_surrogate_split_and_wrong_page_when_original_native_membership_is_corrupt(self):
        index = fixture()['wrapped']['index']
        for rows in [[{'start': 0, 'end': 1}], [{'start': 0, 'end': 2, 'pages': [2]}],
                     [{'start': 0, 'end': 2, 'sha256': 'f'*64}]]:
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                original_native(rows, index)

    def test_should_reject_duplicate_and_dangling_owners_when_unscored_records_are_retained(self):
        for original in [{'objects': [{'id': 'same'}, {'id': 'same'}]},
                         {'references': [{'target_id': 'absent'}]}]:
            with self.subTest(original=original), self.assertRaises(ValueError):
                original_endpoints(original, {}, {}, 'synthetic')

    def test_should_reject_tampered_freeze_review_producer_and_metadata_when_bundle_is_loaded(self):
        mutations = [lambda d: d['review'].update(reviewer='constructor'),
            lambda d: d['checkpoint']['records'][0]['artifacts']['inventory-v1.json'].update(sha256='e'*64),
            lambda d: d['construction']['retained_inventory']['source_metadata'].pop('label_targets'),
            lambda d: d['producer_history']['confirmed_producers'].update(primary='invented'),
            lambda d: d['source_scope_review'].update(verdict='review=true'),
            lambda d: d['dispositions']['rows'].pop(),
            lambda d: d['review'].update(complete_numbered_count=0)]
        for mutation in mutations:
            with self.subTest(mutation=mutation), self.assertRaises((ValueError, KeyError)):
                admit(fixture(), mutation)

    def test_should_reject_mixed_format_when_numbered_only_candidate_has_another_overlay(self):
        data = fixture(); data['candidate']['manual_object_overlay'] = {}
        with self.assertRaisesRegex(ValueError, 'cannot mix'):
            admit(data)


if __name__ == '__main__':
    unittest.main()
