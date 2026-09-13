"""Independent small synthetic histories; no real paper labels or detector input."""
from copy import deepcopy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from annotations import canonical
from archive import read_archive, sha256
from native_exports import native_projection
from parser import parse_project
from tranche import (ARTIFACTS, FORMAT, REVIEW_FORMAT, attach_tranche, covered, history, native_members,
                     rectangle, validate_crosswalk, validate_objects, validate_references, validate_roles, validate_tranche,
                     verify_correction_files)


def fixture(visual_kind='figure'):
    source = r'\begin{document}\begin{figure}\caption{A small chart.}\label{fig:a}\end{figure}\begin{equation}abcdefghij=12345\label{eq:a}\end{equation}\end{document}'
    if visual_kind == 'table':
        source = source.replace('{figure}', '{table}').replace(r'\label{fig:a}', r'cell\label{fig:a}')
    files, members = read_archive(source.encode())
    name = next(iter(files)); parsed = parse_project(files)
    text = visual_kind.title() + ' 1. A small chart. ' + ('cell ' if visual_kind == 'table' else '') + 'abcdefghij=12345 (1)'
    native = {'index': {'text': text, 'pages': [{'number': 1, 'start': 0, 'end': len(text), 'width': 100, 'height': 100}], 'tokens': []}}
    index_raw = canonical(native)
    candidate = {'arxiv_id': 'synthetic', 'paper_id': 'a' * 16, 'index': {'path': 'index.json', 'sha256': sha256(index_raw)},
                 'stratum': ['synthetic', 2020], 'pdf_sha256': 'p' * 64, 'source_sha256': sha256(source.encode()),
                 'accepted': False, 'source_inventory': parsed, 'source_inventory_sha256': sha256(canonical(parsed))}
    exported = native_projection(index_raw, candidate['paper_id'])
    source_export = {'text_members': files, 'members': members, 'source_sha256': sha256(source.encode())}
    image = {'path': '/synthetic/page.png', 'sha256': 'b' * 64, 'page': 1}
    packet = {'arxiv_id': 'synthetic', 'paper_id': candidate['paper_id'], 'index': candidate['index'],
              'version': 1, 'stratum': candidate['stratum'], 'pdf': {'sha256': candidate['pdf_sha256']},
              'source': {'sha256': candidate['source_sha256']}, 'images': [image]}
    def member(start, end):
        return {'member': {'path': name, 'start': start, 'end': end}, 'member_sha256': sha256(source.encode()),
                'text_sha256': sha256(source[start:end].encode())}
    def primary_source(start, end):
        return {'member': name, 'start': start, 'end': end, 'text': source[start:end]}
    caption_start = source.index('A small chart.'); caption_end = caption_start + len('A small chart.')
    equation_start = text.index('abcdefghij'); equation_end = equation_start + len('abcdefghij=12345')
    number_start = text.index('(1)')
    fig = next(row for row in parsed['objects'] if row['kind'] == visual_kind)
    eq = next(row for row in parsed['objects'] if row['kind'] == 'equation')
    independent_objects = []; primary_objects = []
    for label, row in [('fig-original', fig), ('eq-original', eq)]:
        span = row['source_members'][0]
        original = {'id': label, 'kind': row['kind'], 'source': member(span['start'], span['end']),
                    'source_labels': row['labels'], 'printed_label': '1', 'pages': [1], 'parent_object': None, 'nested_objects': []}
        primary = {'id': 'primary-' + label, 'kind': row['kind'], 'source_environment': primary_source(span['start'], span['end']),
                   'printed_number': '1', 'body_region': {'page': 1, 'x_min': 10, 'y_min': 10, 'x_max': 30, 'y_max': 30}}
        if row['kind'] in ('figure', 'table'):
            caption_native_end = text.index('A small chart.') + len('A small chart.')
            original.update(caption_native_spans=[{'start': 0, 'end': caption_native_end}],
                            caption_command_source={'start': source.index('\\caption'), 'end': caption_end + 1},
                            caption_source=member(caption_start, caption_end),
                            visual_body_region={'page': 1, 'rects': [{'x_min': 10, 'y_min': 10, 'x_max': 30, 'y_max': 30}]})
            primary.update(native_caption_members=[{'start': 0, 'end': caption_native_end}],
                           source_caption=primary_source(caption_start, caption_end))
            if row['kind'] == 'table':
                body = {'start': text.index('cell'), 'end': text.index('cell') + 4}
                original['body_native_spans'] = [body]; primary['native_body_members'] = [deepcopy(body)]
        else:
            original.update(body_native_spans=[{'start': equation_start, 'end': equation_end}],
                            number_native_spans=[{'start': number_start, 'end': number_start + 3}], native_fidelity='No semantic quote asserted')
            primary.update(native_body_members=[{'start': equation_start, 'end': equation_end}],
                           native_printed_number={'start': number_start, 'end': number_start + 3},
                           printed_number_region={'page': 1, 'x_min': 75, 'y_min': 6, 'x_max': 81, 'y_max': 12})
        independent_objects.append(original); primary_objects.append(primary)
    inputs = [{'path': key + '.json', 'sha256': sha256(canonical(value))} for key, value in
              [('packet', packet), ('source_export', source_export), ('native_export', exported)]]
    inputs.append(image)
    protocol = {'all_original_pages_viewed_before_source_and_native': True, 'pages_seen': [1],
                'excluded_material_not_accessed': ['Detector outputs', 'Automatic/parser inventories', 'Other annotator labels']}
    identity = {key: packet[key] for key in ('arxiv_id', 'paper_id', 'version')}
    primary = {'annotator_role': 'primary', 'paper': identity, 'protocol': protocol, 'inputs': inputs, 'objects': primary_objects,
               'counts': {'formal_statement': 0, 'proof': 0, 'captioned_algorithm': 0, 'figure': int(visual_kind == 'figure'), 'table': int(visual_kind == 'table'), 'numbered_equation': 1, 'source_reference': 0},
               'headings': [], 'manual_lists': [], 'narrative_procedure_candidates': [], 'references': [], 'citations': []}
    independent = {**identity, 'annotator': 'independent secondary', 'inspection_order': 'all originals before source/native',
                   'pages_inspected': [1], 'objects': independent_objects, 'bibliography': [],
                   'counts': {'formal_statement': 0, 'formal_proof': 0, 'formal_algorithm': 0, 'figure': int(visual_kind == 'figure'), 'table': int(visual_kind == 'table'), 'numbered_equation': 1, 'explicit_reference_occurrences': 0},
                   'statement_role_candidates': [], 'manual_procedures_and_role_candidates': [],
                   'section_destinations': [], 'references': [], 'citations': []}
    def ref(value):
        raw = canonical(value); return {'sha256': sha256(raw), 'bytes': len(raw)}
    primary_receipt = {'inventory': ref(primary), 'annotator_role': 'primary', 'blindness': protocol, 'inputs': inputs}
    independent_receipt = {'inventory': ref(independent), 'annotator': 'secondary', 'all_pages_inspected': [1],
                           'packet': ref(packet), 'source_export': ref(source_export), 'native_export': ref(exported), 'images': [image]}
    eq_original = independent_objects[1]
    box = {'page': 1, 'rect': [3, 6, 30, 12], 'pixels_96dpi': [4, 8, 40, 16]}
    num_box = {'page': 1, 'rect': [75, 6, 81, 12], 'pixels_96dpi': [100, 8, 108, 16]}
    equation = {'id': eq_original['id'], 'original_object_sha256': sha256(canonical(eq_original)),
                'source': eq_original['source'], 'source_text': source[eq['source_members'][0]['start']:eq['source_members'][0]['end']],
                'unresolved_expression_ambiguities': [], 'source_vs_printed_judgment': 'Complete expression independently agrees',
                'existing_native_memberships': {k: deepcopy(eq_original[k]) for k in ('body_native_spans', 'number_native_spans')},
                'visual_body_boxes': [box], 'printed_number_boxes': [num_box]}
    math = {'paper': identity, 'original_inventory_sha256': sha256(canonical(independent)), 'field_corrected_inventory_v2_sha256': sha256(canonical(independent)),
            'annotator': 'secondary', 'objects': [equation]}
    math_receipt = {'supplement_sha256': sha256(canonical(math)), 'supplement_bytes': len(canonical(math)), 'annotator': 'secondary'}
    construction = {'not_an_original_annotator_input': True, 'original_blind_packet': ref(packet), 'primary_annotation': ref(primary),
                    'independent_annotation_v2': ref(independent), 'candidate': ref(candidate),
                    **{key: packet[key] for key in ('arxiv_id', 'paper_id', 'index', 'pdf', 'source', 'version')},
                    'objects': parsed['objects'], 'links': parsed['links']}
    docs = {key: {} for key in ARTIFACTS}
    docs.update(packet=packet, source_export=source_export, native_export=exported, primary=primary, primary_receipt=primary_receipt,
                independent=independent, independent_corrected=deepcopy(independent), independent_receipt=independent_receipt,
                correction_receipt={'inventory': ref(independent)}, math=math, math_receipt=math_receipt, construction=construction,
                visual_review={'primary_inventory_sha256': sha256(canonical(primary)), 'independent_inventory_sha256': sha256(canonical(independent)),
                               'verdict': 'clear_full_body_regions_for_later_validated_assembly',
                               'regions': [{'kind': visual_kind, 'printed_number': '1', 'page': 1, 'full_body_basis': 'independent full region review',
                                            'body_pdf_points': {'x_min': 10.5, 'y_min': 10.5, 'x_max': 30, 'y_max': 30}, 'body_pixels_192dpi': [28, 28, 80, 80]}]},
                math_review={'status': 'clear_independent_math_region_supplement_not_truth_admission',
                             'supplement_sha256': sha256(canonical(math)), 'supplement_receipt_sha256': sha256(canonical(math_receipt)),
                             'rows': [{'id': eq_original['id'], 'selected_body_boxes': [box], 'selected_number_boxes': [num_box],
                                       'source_native_memberships_unchanged': True, 'root_visual_disposition': 'Complete expression independently verified'}]},
                role_review={'verdict': 'clear_scope_classification_for_later_independent_negative_cohort_validation',
                             'formal_scope_counts': {'captioned_algorithm_or_code_listing': 0, 'proof': 0, 'theorem_like_statement': 0},
                             'retained_role_candidates': []})
    docs['review'] = {'format': REVIEW_FORMAT, 'verdict': 'clear_complete_visual_math_inventory', 'findings': [],
                      'artifacts': {}, 'source_inventory_sha256': candidate['source_inventory_sha256'],
                      'annotators': {'primary': 'first', 'independent': 'secondary'}, 'reviewer': 'third',
                      'initial_enumeration_before_source_and_native': True, 'later_crosswalk_reviewed_after_annotation_freeze': True,
                      'pages_inspected': [1], 'complete_kind_counts': {'figure': int(visual_kind == 'figure'), 'table': int(visual_kind == 'table'), 'equation': 1, 'statement': 0, 'proof': 0, 'algorithm': 0},
                      'reference_counts': {'all': 0, 'O4': 0, 'visual': 0, 'non_object': 0}}
    docs['object_crosswalk'] = {'source_inventory_canonical_sha256': candidate['source_inventory_sha256'], 'unmatched_parsed_object_ids': [],
                               'crosswalk': [{'independent_id': identifier, 'primary_id': 'primary-' + identifier, 'parsed_ids': [row['id']],
                                              'kind': row['kind'], 'original_source_member': deepcopy(row['source_members'][0])}
                                             for identifier, row in [('fig-original', fig), ('eq-original', eq)]]}
    docs['link_crosswalk'] = {'links': []}
    return candidate, native, source.encode(), docs, {image['path']: image['sha256']}


def seal(docs):
    raws = {k: canonical(v) for k, v in docs.items()}
    docs['review']['artifacts'] = {k: sha256(raw) for k, raw in raws.items() if k != 'review'}
    return {k: canonical(v) for k, v in docs.items()}


class TrancheTests(unittest.TestCase):
    def test_should_preserve_automatic_exclusions_when_complete_blind_mixed_inventory_is_reviewed(self):
        candidate, native, source, docs, images = fixture()
        result = validate_tranche(canonical(candidate), canonical(native), source, seal(docs), images)
        self.assertEqual({key: result[key] for key in candidate}, candidate)
        self.assertFalse(result['accepted'])
        self.assertEqual(len(result['manual_figure_table_overlay']['objects']), 1)
        overlay = result['manual_object_overlay']
        self.assertEqual(len(overlay['objects']), 1)
        self.assertFalse(overlay['objects'][0]['semantic_quote_truth'])
        self.assertTrue(overlay['metric_eligibility']['O5'])
        self.assertEqual(set(overlay['omitted_metrics']), {'O8', 'O9', 'O10', 'O11'})

    def test_should_reject_resealed_review_changes_when_complete_evidence_or_independence_is_lost(self):
        for mutation in [lambda d: d['review'].update(findings=['unresolved']),
                         lambda d: d['review'].update(verdict='rejected'),
                         lambda d: d['review'].update(reviewer=None),
                         lambda d: d['review'].update(reviewer=''),
                         lambda d: d['review'].update(pages_inspected=[]),
                         lambda d: d['review']['complete_kind_counts'].update(equation=0),
                         lambda d: d['review']['complete_kind_counts'].update(table=False),
                         lambda d: d['review']['annotators'].update(primary='secondary'),
                         lambda d: d['review'].update(reviewer='secondary'),
                         lambda d: d['review'].update(later_crosswalk_reviewed_after_annotation_freeze=False)]:
            candidate, native, source, docs, images = fixture(); mutation(docs)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate_tranche(canonical(candidate), canonical(native), source, seal(docs), images)

    def test_should_reject_original_history_changes_when_a_later_candidate_is_claimed_as_blind_input(self):
        candidate, native, source, docs, images = fixture()
        docs['packet']['candidate_sha256'] = sha256(canonical(candidate))
        # All packet receipts are resealed: the chronological contradiction remains.
        new = sha256(canonical(docs['packet']))
        docs['independent_receipt']['packet'] = {'sha256': new}
        for row in docs['primary']['inputs']:
            if row['path'] == 'packet.json': row['sha256'] = new
        docs['primary_receipt']['inventory'] = {'sha256': sha256(canonical(docs['primary']))}
        with self.assertRaisesRegex(ValueError, 'initial blind packet'):
            validate_tranche(canonical(candidate), canonical(native), source, seal(docs), images)

    def test_should_reject_object_changes_when_bibliography_correction_exceeds_its_declared_scope(self):
        candidate, _, _, docs, _ = fixture()
        docs['independent_corrected']['objects'][0]['printed_label'] = '2'
        raw = canonical(docs['independent_corrected'])
        docs['correction_receipt']['inventory'] = {'sha256': sha256(raw), 'bytes': len(raw)}
        with self.assertRaisesRegex(ValueError, 'correction changed'):
            history(docs, seal(docs), canonical(candidate))

    def test_should_retain_correction_metadata_when_original_limitations_and_receipt_chain_remain_exact(self):
        candidate, _, _, docs, _ = fixture()
        original = docs['independent']; corrected = docs['independent_corrected']
        original['limitations'] = ['Native text omits an accent.']
        docs['independent_receipt']['inventory'] = {'sha256': sha256(canonical(original))}
        provenance = {'original_inventory': {'sha256': sha256(canonical(original))},
                      'original_receipt': {'sha256': sha256(canonical(docs['independent_receipt']))},
                      'scope': 'Bibliography native memberships only; original objects and links unchanged.',
                      'reason': 'Retain a raw combining accent in the field membership.',
                      'changed_fields': 1, 'audited_fields': 3, 'added_nonwhitespace_characters': 1,
                      'field_audit_path': 'independent/field-audit.json'}
        corrected['limitations'] = original['limitations'] + ['The field audit restores the accent; no truth acceptance.']
        corrected['correction_provenance'] = provenance
        docs['correction_receipt']['correction_scope'] = deepcopy(provenance)
        docs['correction_receipt']['inventory'] = {'sha256': sha256(canonical(corrected))}
        docs['math']['original_inventory_sha256'] = sha256(canonical(original))
        docs['math']['field_corrected_inventory_v2_sha256'] = sha256(canonical(corrected))
        docs['math_receipt']['supplement_sha256'] = sha256(canonical(docs['math']))
        docs['math_receipt']['supplement_bytes'] = len(canonical(docs['math']))
        docs['construction']['independent_annotation_v2'] = {'sha256': sha256(canonical(corrected))}
        history(docs, seal(docs), canonical(candidate))
        for mutation in [lambda d: d['independent_corrected']['limitations'].pop(0),
                         lambda d: d['independent_corrected']['limitations'].__setitem__(0, 'No native loss.'),
                         lambda d: d['independent_corrected']['correction_provenance'].update(reason='another reason'),
                         lambda d: d['independent_corrected']['correction_provenance']['original_inventory'].update(sha256='0' * 64),
                         lambda d: d['independent_corrected']['objects'].pop()]:
            changed = deepcopy(docs); mutation(changed)
            changed['correction_receipt']['inventory'] = {'sha256': sha256(canonical(changed['independent_corrected']))}
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                history(changed, seal(changed), canonical(candidate))

    def test_should_reject_math_geometry_changes_when_reviewed_boxes_or_native_members_differ(self):
        for mutation in [lambda d: d['math_review']['rows'][0].update(selected_body_boxes=[]),
                         lambda d: d['math']['objects'][0].update(unresolved_expression_ambiguities=['unclear script']),
                         lambda d: d['math']['objects'][0]['existing_native_memberships']['body_native_spans'][0].update(start=0)]:
            candidate, native, source, docs, images = fixture(); mutation(docs)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate_tranche(canonical(candidate), canonical(native), source, seal(docs), images)

    def test_should_reject_cross_page_geometry_when_regions_disagree_with_native_owners(self):
        for kind in ('figure', 'table'):
            candidate, native, source, docs, _ = fixture(kind); index = native['index']; files, _ = read_archive(source)
            start = len(index['text']); index['text'] += 'cell on another page'
            index['pages'].append({'number': 2, 'start': start, 'end': len(index['text']), 'width': 100, 'height': 100})
            validate_objects(docs, seal(docs), candidate, files, index)
            mutations = [lambda d: d['math']['objects'][0]['printed_number_boxes'][0].update(page=2),
                         lambda d: d['primary']['objects'][0]['body_region'].update(page=2),
                         lambda d: d['primary']['objects'][1]['body_region'].update(page=2),
                         lambda d: d['primary']['objects'][1]['printed_number_region'].update(page=2),
                         lambda d: d['independent_corrected']['objects'][1].update(pages=[True])]
            if kind == 'table':
                def move_table_body(d):
                    d['independent_corrected']['objects'][0]['body_native_spans'] = [{'start': start, 'end': start + 4}]
                    d['primary']['objects'][0]['native_body_members'] = [{'start': start, 'end': start + 4}]
                mutations.append(move_table_body)
            for mutation in mutations:
                changed = deepcopy(docs); mutation(changed)
                changed['math_receipt']['supplement_sha256'] = sha256(canonical(changed['math']))
                changed['math_receipt']['supplement_bytes'] = len(canonical(changed['math']))
                changed['math_review']['supplement_sha256'] = sha256(canonical(changed['math']))
                changed['math_review']['supplement_receipt_sha256'] = sha256(canonical(changed['math_receipt']))
                changed['visual_review']['primary_inventory_sha256'] = sha256(canonical(changed['primary']))
                with self.subTest(kind=kind, mutation=mutation), self.assertRaises(ValueError):
                    validate_objects(changed, seal(changed), candidate, files, index)

    def test_should_reject_reused_or_empty_reference_numbers_when_source_occurrences_are_distinct(self):
        source = r'\ref{e} and \ref{e}'; text = 'Equation 1 and Equation 1'
        index = {'text': text, 'pages': [{'number': 1, 'start': 0, 'end': len(text)}]}
        candidate = {'source_inventory': {'objects': [{'id': 'equation-source', 'kind': 'equation'}],
                                          'label_targets': {'e': 'equation-source'}, 'links': []}}
        docs = {'primary': {'references': [], 'counts': {'source_reference': 2}},
                'independent_corrected': {'references': [], 'section_destinations': [], 'counts': {'explicit_reference_occurrences': 2}}}
        for n, (s, t) in enumerate([(0, 0), (source.rindex(r'\ref'), text.rindex('Equation'))]):
            member = {'path': 'main.tex', 'start': s, 'end': s + len(r'\ref{e}')}
            phrase = {'start': t, 'end': t + len('Equation 1')}; number = {'start': phrase['end'] - 1, 'end': phrase['end']}
            candidate['source_inventory']['links'].append({'kind': 'reference', 'source_members': [member], 'targets': ['e']})
            docs['independent_corrected']['references'].append({'id': 'i' + str(n), 'source': {'member': member, 'text_sha256': sha256(source[member['start']:member['end']].encode())},
                'unresolved_targets': [], 'source_label': 'e', 'native_number_span': deepcopy(number), 'native_phrase_span': deepcopy(phrase), 'destination_id': 'i-equation', 'destination_kind': 'equation'})
            docs['primary']['references'].append({'id': 'p' + str(n), 'source_occurrence': {'member': 'main.tex', 'start': member['start'], 'end': member['end']},
                'target_label': 'e', 'native_number': deepcopy(number), 'native_occurrence': deepcopy(phrase), 'target_id': 'p-equation', 'target_kind': 'equation'})
        args = (candidate, {'main.tex': source}, index, {'i-equation': 'equation-source'}, {'p-equation': 'equation-source'})
        self.assertEqual(len(validate_references(docs, *args)[0]), 2)
        for case in ('reused', 'whitespace', 'outside_phrase'):
            changed = deepcopy(docs); independent = changed['independent_corrected']['references']; primary = changed['primary']['references']
            if case == 'reused':
                for a, b in [('native_number_span', 'native_number'), ('native_phrase_span', 'native_occurrence')]:
                    independent[1][a] = deepcopy(independent[0][a]); primary[1][b] = deepcopy(primary[0][b])
            elif case == 'whitespace':
                independent[0]['native_number_span'] = primary[0]['native_number'] = {'start': 8, 'end': 9}
            else:
                independent[0]['native_phrase_span'] = primary[0]['native_occurrence'] = {'start': 0, 'end': 8}
            with self.subTest(case=case), self.assertRaises(ValueError): validate_references(changed, *args)
        # One shared phrase may describe two independently located numbers.
        for row in docs['independent_corrected']['references']: row['native_phrase_span'] = {'start': 0, 'end': len(text)}
        for row in docs['primary']['references']: row['native_occurrence'] = {'start': 0, 'end': len(text)}
        self.assertEqual(len(validate_references(docs, *args)[0]), 2)

    def test_should_reject_page_type_and_geometry_errors_when_spans_or_rectangles_are_malformed(self):
        index = {'text': 'A😀 B', 'pages': [{'number': 1, 'start': 0, 'end': 5, 'width': 100, 'height': 100}]}
        for span in [{'start': False, 'end': 1}, {'start': 2, 'end': 3}, {'start': 0, 'end': 1, 'page': True}]:
            with self.subTest(span=span), self.assertRaises(ValueError): native_members([span], index)
        for rect in [[0, 0, 101, 1], [0, 0, float('inf'), 1], [False, 0, 1, 1]]:
            with self.subTest(rect=rect), self.assertRaises(ValueError): rectangle(1, rect, index)
        self.assertEqual(covered(native_members([{'start': 0, 'end': 3}], index), index), {0, 1})

    def test_should_reject_resealed_caption_role_changes_when_both_annotations_select_a_subphrase(self):
        candidate, native, source, docs, _ = fixture(); files, _ = read_archive(source)
        first = docs['independent_corrected']['objects'][0]
        first['caption_source']['member']['start'] += 2
        member = first['caption_source']['member']
        first['caption_source']['text_sha256'] = sha256(files[member['path']][member['start']:member['end']].encode())
        original = docs['primary']['objects'][0]['source_caption']; original['start'] += 2
        original['text'] = original['text'][2:]
        raws = seal(docs); docs['visual_review']['primary_inventory_sha256'] = sha256(raws['primary'])
        with self.assertRaisesRegex(ValueError, 'complete declared source argument'):
            validate_objects(docs, seal(docs), candidate, files, native['index'])

    def test_should_reject_inventory_or_crosswalk_omissions_when_a_review_is_resealed(self):
        for mutation in [lambda d: d['object_crosswalk']['crosswalk'].pop(),
                         lambda d: d['object_crosswalk']['crosswalk'][0].update(parsed_ids=['another']),
                         lambda d: d['object_crosswalk']['crosswalk'][0]['original_source_member'].update(start=False),
                         lambda d: d['object_crosswalk'].update(unmatched_parsed_object_ids=['missing'])]:
            candidate, _, _, docs, _ = fixture(); mutation(docs)
            source = candidate['source_inventory']['objects']; ids = {'fig-original': source[0]['id'], 'eq-original': source[1]['id']}
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate_crosswalk(docs, candidate, ids, {'primary-' + k: v for k, v in ids.items()})

    def test_should_share_literal_reference_grammar_when_tranche_binds_a_comma_label(self):
        for duplicate in (False, True):
            source = (r'\begin{document}\begin{equation}\label{e,x}abcdefghij=12345\end{equation}'
                      + (r'\label{e,x}' if duplicate else '') + r'Equation \ref{e,x}\end{document}')
            files = {'main.tex': source}; parsed = parse_project(files); link = parsed['links'][0]
            self.assertEqual(link['targets'], ['e,x'])
            member = link['source_members'][0]; target = parsed['objects'][0]['id']
            source_digest = sha256(source[member['start']:member['end']].encode())
            number = {'start': 9, 'end': 10}; phrase = {'start': 0, 'end': 10}
            docs = {'primary': {'counts': {'source_reference': 1}, 'references': [{
                'id': 'primary-reference', 'source_occurrence': {'member': member['path'], 'start': member['start'], 'end': member['end']},
                'target_label': 'e,x', 'target_id': 'primary-equation', 'target_kind': 'equation',
                'native_number': number, 'native_occurrence': phrase}]},
                'independent_corrected': {'counts': {'explicit_reference_occurrences': 1}, 'section_destinations': [], 'references': [{
                    'id': 'independent-reference', 'source': {'member': member, 'text_sha256': source_digest}, 'source_label': 'e,x',
                    'unresolved_targets': [], 'destination_id': 'independent-equation', 'destination_kind': 'equation',
                    'native_number_span': number, 'native_phrase_span': phrase}]}}
            args = (docs, {'source_inventory': parsed}, files, {'text': 'Equation 1', 'pages': [{'number': 1, 'start': 0, 'end': 10}]},
                    {'independent-equation': target}, {'primary-equation': target})
            with self.subTest(duplicate=duplicate):
                if duplicate:
                    with self.assertRaisesRegex(ValueError, 'ambiguous source reference'): validate_references(*args)
                else:
                    self.assertEqual(validate_references(*args)[0][0]['target'], target)

    def test_should_retain_typed_visual_and_section_references_when_every_source_occurrence_is_bound(self):
        source = r'\ref{e}\ref{f}\ref{s}\section{End}\label{s}'
        text = 'Equation 1 Figure 2 Section 3'
        index = {'text': text, 'pages': [{'number': 1, 'start': 0, 'end': len(text)}]}
        files = {'main.tex': source}; links = []; independent = []; primary = []
        for n, (label, word, target, kind) in enumerate([('e', 'Equation', 'eq', 'equation'), ('f', 'Figure', 'fig', 'figure'), ('s', 'Section', 'section:s', 'section')]):
            start = source.index(r'\ref{' + label + '}'); end = start + len(r'\ref{' + label + '}')
            phrase_start = text.index(word); phrase_end = phrase_start + len(word) + 2
            member = {'path': 'main.tex', 'start': start, 'end': end}
            links.append({'kind': 'reference', 'source_members': [member], 'targets': [label]})
            independent.append({'id': 'i' + str(n), 'source': {'member': member, 'text_sha256': sha256(source[start:end].encode())},
                                'unresolved_targets': [], 'source_label': label, 'destination_id': target, 'destination_kind': kind,
                                'native_number_span': {'start': phrase_end - 1, 'end': phrase_end}, 'native_phrase_span': {'start': phrase_start, 'end': phrase_end}})
            primary.append({'id': 'p' + str(n), 'source_occurrence': {'member': 'main.tex', 'start': start, 'end': end},
                            'target_label': label, 'target_id': target, 'target_kind': kind,
                            'native_number': {'start': phrase_end - 1, 'end': phrase_end}, 'native_occurrence': {'start': phrase_start, 'end': phrase_end}})
        section_start = source.index(r'\section')
        docs = {'primary': {'references': primary, 'counts': {'source_reference': 3}},
                'independent_corrected': {'references': independent, 'counts': {'explicit_reference_occurrences': 3},
                                         'section_destinations': [{'id': 'section:s', 'source': {'member': {'path': 'main.tex', 'start': section_start, 'end': len(source)},
                                                                                              'text_sha256': sha256(source[section_start:].encode())}}]}}
        candidate = {'source_inventory': {'links': links, 'objects': [{'id': 'eq', 'kind': 'equation'}, {'id': 'fig', 'kind': 'figure'}], 'label_targets': {'e': 'eq', 'f': 'fig'}}}
        refs, visuals, sections = validate_references(docs, candidate, files, index, {'eq': 'eq', 'fig': 'fig'}, {'eq': 'eq', 'fig': 'fig'})
        self.assertEqual([row['target'] for row in refs], ['eq'])
        self.assertEqual([row['target'] for row in visuals], ['fig'])
        self.assertEqual([row['target'] for row in sections], ['section:s'])
        for label in ('e', 'f', 's'):
            changed = deepcopy(candidate)
            changed['source_inventory']['ambiguous_labels'] = {label: {'candidate_count': 2, 'roles': ['object', 'section']}}
            with self.subTest(ambiguous=label), self.assertRaisesRegex(ValueError, 'ambiguous source reference'):
                validate_references(docs, changed, files, index, {'eq': 'eq', 'fig': 'fig'}, {'eq': 'eq', 'fig': 'fig'})
        for changed in ['section:s', 'missing']:
            altered = deepcopy(docs); altered['independent_corrected']['references'][0]['destination_id'] = changed
            altered['independent_corrected']['references'][0]['destination_kind'] = 'section'
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                validate_references(altered, candidate, files, index, {'eq': 'eq', 'fig': 'fig'}, {'eq': 'eq', 'fig': 'fig'})

    def test_should_reject_missing_role_dispositions_when_an_informal_candidate_is_not_reconciled(self):
        _, _, _, docs, _ = fixture()
        files = {'main.tex': 'A short procedure'}; digest = sha256(files['main.tex'].encode())
        row = {'id': 'procedure', 'source': {'member': {'path': 'main.tex', 'start': 0, 'end': 17}, 'text_sha256': digest}}
        docs['independent_corrected']['manual_procedures_and_role_candidates'] = [row]
        docs['primary']['manual_lists'] = [{'id': 'primary-procedure', 'source_environment': {'member': 'main.tex', 'start': 0, 'end': 17}}]
        with self.assertRaisesRegex(ValueError, 'omits/adds'): validate_roles(docs, files)
        docs['role_review']['retained_role_candidates'] = [{'independent_candidate_id': 'procedure', 'primary_candidate_id': 'primary-procedure',
                                                         'candidate_retained': True, 'reason': 'Informal prose workflow, no formal listing',
                                                         'proposed_disposition': 'outside_E1.5_algorithm_listing_scope',
                                                         'source_member': 'main.tex', 'source_span': {'start': 0, 'end': 17}, 'source_excerpt_sha256': digest}]
        self.assertEqual(len(validate_roles(docs, files)), 1)
        # The reviewed full passage can include a heading around the primary's
        # narrower body occurrence. Retain the complete independent outer span.
        docs['primary']['manual_lists'][0]['source_environment'].update(start=4, end=15)
        self.assertEqual(validate_roles(docs, files)[0]['source_span'], {'start': 0, 'end': 17})
        for member in [{'member': 'other.tex', 'start': 4, 'end': 15},
                       {'member': 'main.tex', 'start': 4, 'end': 18},
                       {'member': 'main.tex', 'start': 18, 'end': 19}]:
            changed = deepcopy(docs); changed['primary']['manual_lists'][0]['source_environment'] = member
            with self.subTest(member=member), self.assertRaises(ValueError):
                validate_roles(changed, {**files, 'other.tex': files['main.tex']})
        docs['role_review']['retained_role_candidates'][0]['proposed_disposition'] = 'uncertain'
        with self.assertRaises(ValueError): validate_roles(docs, files)

    def test_should_reject_manifest_paths_when_they_escape_or_follow_a_symlink(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name); bundle = root/'bundle'; bundle.mkdir(); (root/'value.json').write_bytes(b'{}')
            (root/'alias.json').symlink_to(root/'value.json')
            for path in ['../value.json', 'alias.json']:
                manifest = {'schema_version': 1, 'format': FORMAT,
                            'artifacts': {key: {'path': path, 'sha256': sha256(b'{}'), 'bytes': 2} for key in ARTIFACTS}}
                (bundle/'manifest.json').write_bytes(canonical(manifest))
                with self.subTest(path=path), self.assertRaises(ValueError):
                    attach_tranche(root, root, root, b'{}', {}, {}, 'bundle')

    def test_should_rehash_correction_audit_and_inputs_when_declared_paths_are_part_of_the_history(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name); annotation = root/'annotation'; annotation.mkdir()
            def reference(name, raw):
                (annotation/name).write_bytes(raw)
                return {'path': name, 'sha256': sha256(raw), 'bytes': len(raw)}
            receipt = {'audit': reference('audit.json', b'{"checked":true}'),
                       'inputs_rehashed': [reference('source.json', b'{"source":"original"}')]}
            corrected = {'correction_provenance': {'field_audit_path': 'audit.json'}}
            verify_correction_files(root, annotation, receipt, corrected)
            for field in ('audit', 'input'):
                row = receipt['audit'] if field == 'audit' else receipt['inputs_rehashed'][0]
                path = annotation/row['path']; original = path.read_bytes(); path.write_bytes(b'{}')
                with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'bytes differ'):
                    verify_correction_files(root, annotation, receipt, corrected)
                path.write_bytes(original)
            for path in ('../audit.json', 'alias.json'):
                if path == 'alias.json': (annotation/path).symlink_to(annotation/'audit.json')
                changed = deepcopy(receipt); changed['audit']['path'] = path
                with self.subTest(path=path), self.assertRaises(ValueError):
                    verify_correction_files(root, annotation, changed, {'correction_provenance': {'field_audit_path': path}})
            with self.assertRaisesRegex(ValueError, 'another actual audit path'):
                verify_correction_files(root, annotation, receipt, {'correction_provenance': {'field_audit_path': 'another.json'}})
            changed = deepcopy(receipt); changed['audit']['bytes'] = True
            with self.assertRaisesRegex(ValueError, 'bytes exceed'):
                verify_correction_files(root, annotation, changed, corrected)

    def test_should_reject_malformed_format_when_a_manifest_attempts_implicit_version_selection(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name); (root/'bundle').mkdir()
            for version, format_name in [(True, FORMAT), (1, 'legacy'), (1, None)]:
                (root/'bundle/manifest.json').write_bytes(canonical({'schema_version': version, 'format': format_name, 'artifacts': {}}))
                with self.subTest(version=version, format_name=format_name), self.assertRaises(ValueError):
                    attach_tranche(root, root, root, b'{}', {}, {}, 'bundle')

    def test_should_reject_review_hash_drift_when_the_same_input_document_is_replaced(self):
        candidate, native, source, docs, images = fixture(); raws = seal(docs)
        raws['role_review'] = canonical({**docs['role_review'], 'new_unreviewed_note': 'changed'})
        with self.assertRaisesRegex(ValueError, 'complete evidence closure'):
            validate_tranche(canonical(candidate), canonical(native), source, raws, images)

    def test_should_reject_truncated_body_or_geometry_when_math_history_is_otherwise_unchanged(self):
        for field, value in [('source_native_memberships_unchanged', False), ('root_visual_disposition', '')]:
            candidate, native, source, docs, _ = fixture(); files, _ = read_archive(source)
            docs['math_review']['rows'][0][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'complete source/native disposition'):
                validate_objects(docs, seal(docs), candidate, files, native['index'])
        candidate, native, source, docs, _ = fixture(); files, _ = read_archive(source)
        docs['independent_corrected']['objects'][1]['body_native_spans'][0]['end'] -= 1
        with self.assertRaises(ValueError): validate_objects(docs, seal(docs), candidate, files, native['index'])

    def test_should_reject_foreign_candidate_identity_when_a_blind_packet_is_reused(self):
        for key, value in [('paper_id', 'foreign'), ('stratum', ['foreign', 1900]), ('source_sha256', 'f' * 64)]:
            candidate, native, source, docs, images = fixture(); candidate[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate_tranche(canonical(candidate), canonical(native), source, seal(docs), images)

    def test_should_reject_missing_original_image_when_the_claimed_hash_is_not_verified(self):
        candidate, native, source, docs, _ = fixture()
        with self.assertRaisesRegex(ValueError, 'image paths/hashes'):
            validate_tranche(canonical(candidate), canonical(native), source, seal(docs), {})

    def test_should_reject_oversized_evidence_when_the_paper_wide_byte_budget_is_exhausted(self):
        candidate, native, source, docs, images = fixture()
        with patch('tranche.MAX_BUNDLE_BYTES', 64), self.assertRaisesRegex(ValueError, 'cumulative byte bound'):
            validate_tranche(canonical(candidate), canonical(native), source, seal(docs), images)


if __name__ == '__main__': unittest.main()
