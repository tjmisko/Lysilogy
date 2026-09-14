"""Synthetic blind histories for direct visuals and separately owned table notes."""
from copy import deepcopy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from annotations import canonical
from archive import read_archive, sha256
from native_exports import native_projection
from parser import parse_project
from tranche_visual import (ARTIFACTS, FORMAT, REVIEW_FORMAT, PRIMARY_ROLES, INDEPENDENT_ROLES,
                            validate_visual, validate_objects, validate_roles, audit_excerpts)


def fixture(note_group=3, with_links=False):
    source = r'\begin{document}\begin{table}\caption{Results.}\label{tab:a}Cell $^1$A note\end{table}\end{document}'
    if with_links:
        source = source.replace(r'\end{document}', r'\section{Context}\label{sec:a}See Table~\ref{tab:a} and Section~\ref{sec:a}. \cite{key}\begin{thebibliography}{1}\bibitem{key}Author. Title. 2020.\end{thebibliography}\end{document}')
    source_raw = source.encode(); files, archive_members = read_archive(source_raw); name = next(iter(files))
    parsed = parse_project(files); parsed_object = parsed['objects'][0]
    text = 'Table 1. Results.\nCell\nA note\n1\nOther prose.'
    if with_links:
        text += '\n1 Context\nSee Table 1 and Section 1. [1]'
    index = {'index': {'text': text, 'pages': [{'number': 1, 'start': 0, 'end': len(text), 'width': 150, 'height': 150}],
                       'tokens': [{'start': text.index('\n1\n') + 1, 'end': text.index('\n1\n') + 2, 'page': 1,
                                   'rects': [{'x_min': 12, 'y_min': 35, 'x_max': 14, 'y_max': 38}]}]}}
    def src(start, end):
        return {'member': name, 'start': start, 'end': end, 'text': source[start:end], 'sha256': sha256(source[start:end].encode())}
    def occurrence(value):
        start = text.index(value)
        return {'start': start, 'end': start + len(value), 'text': value, 'pages': [1], 'unit': 'UTF-16', 'sha256': sha256(value.encode())}
    core = occurrence('Cell'); note = occurrence('A note'); marker = occurrence('1\nOther')
    marker = {'start': marker['start'], 'end': marker['start'] + 1, 'text': '1', 'token': deepcopy(index['index']['tokens'][0])}
    caption = occurrence('Table 1. Results.')
    span = parsed_object['source_members'][0]; env = src(span['start'], span['end'])
    caption_source = src(source.index('Results.'), source.index('Results.') + len('Results.'))
    note_start = source.index('$^1$'); note_source = src(note_start, source.index(r'\end{table}'))
    note_visible = src(note_start + 4, note_source['end'])
    body = [deepcopy(core), deepcopy(note), {k: marker[k] for k in ('start', 'end', 'text')}]
    region = {'x_min': 9, 'y_min': 12, 'x_max': 60, 'y_max': 42}
    candidate = {'arxiv_id': 'synthetic-only', 'paper_id': 'a' * 16, 'index': {'path': 'index.json', 'sha256': sha256(canonical(index))},
                 'stratum': ['synthetic', 2020], 'pdf_sha256': 'p' * 64, 'source_sha256': sha256(source_raw),
                 'accepted': False, 'source_inventory': parsed, 'source_inventory_sha256': sha256(canonical(parsed))}
    image = {'path': '/synthetic/page.png', 'sha256': 'b' * 64, 'page': 1, 'dpi': 96, 'pixel_width': 200,
             'pixel_height': 200, 'pdf_width_points': 150, 'pdf_height_points': 150}
    packet = {k: deepcopy(candidate[k]) for k in ('arxiv_id', 'paper_id', 'index', 'stratum')}
    packet.update(version=1, pdf={'sha256': candidate['pdf_sha256']}, source={'sha256': candidate['source_sha256']}, images=[image])
    identity = {k: packet[k] for k in ('arxiv_id', 'paper_id', 'version')}
    primary = {'schema_version': 1, 'annotator_role': 'primary', 'paper': identity, 'objects': [{
        'id': 'primary-table', 'kind': 'table', 'source_environment': env, 'source_caption': caption_source,
        'source_label': 'tab:a', 'printed_number': '1', 'body_region': {'page': 1, **region},
        'native_caption_members': [caption], 'native_core_members': [core], 'native_body_members': body}],
        'counts': {'figure': 0, 'table': 1, **{k: 0 for k in ('numbered_equation', 'unnumbered_display_equation', 'formal_statement', 'formal_proof', 'captioned_algorithm_or_listing')}},
        'protocol': {'all_original_pages_viewed_before_source_and_native': True, 'pages_seen': [1],
                     'excluded_material_not_accessed': ['Raw reading-index', 'Parser/automatic inventories', 'Detector outputs', 'Any independent annotator labels']},
        'attached_table_notes': [{'id': 'primary-note', 'parent': 'primary-table', 'source_note': note_source,
                                  'native_body_members': body[1:], 'source_row_callouts': [], 'native_row_callouts': []}],
        'headings': [], 'references': [], 'citations': [], **{k: [] for k in PRIMARY_ROLES}}
    group = f'table_{note_group}_notes'; scope = f'table_{note_group}_note_scope'
    independent = {'schema': 'lysilogy.blind-manual-inventory.v1', 'annotation_role': 'independent synthetic annotation',
        'paper': identity, 'pages_inspected': [1], 'objects': [{'id': 'independent-table', 'kind': 'table', 'source': env,
        'source_labels': ['tab:a'], 'printed_label': '1', 'pages': [1], 'caption_source': caption_source, 'caption_native': caption,
        'full_visual_body': {'page': 1, 'region': region, 'native_members': [core]}}],
        'counts': {'figures': 0, 'tables': 1, **{k: 0 for k in ('numbered_equations', 'formal_statements', 'formal_proofs', 'captioned_algorithms_or_code_listings')}},
        'ancillary': {group: [{'printed_label': '1', 'source': note_visible, 'native': note}]}, 'references': [], 'citations': [],
        'nonobject_destinations': [], **{k: [] for k in INDEPENDENT_ROLES}}
    docs = {k: {} for k in ARTIFACTS}
    docs.update(primary=primary, independent=independent, packet=packet, prompt='Blind synthetic prompt.',
                source_export={'source_sha256': sha256(source_raw), 'text_members': files, 'members': archive_members},
                native_export=native_projection(canonical(index), candidate['paper_id']))
    docs['primary_receipt'] = {'schema_version': 1, 'annotator_role': 'primary', 'paper': identity,
                                'protocol': deepcopy(primary['protocol']), 'pages_viewed_first': [1]}
    docs['independent_receipt'] = {'schema': 'lysilogy.blind-manual-receipt.v1', 'annotation_role': independent['annotation_role'],
        'paper': identity, 'page_inspection_order': [1], 'all_pages_before_source_native': True,
        'read_boundary': 'No primary labels or parser/detector access.'}
    docs['visual_review'] = {'verdict': 'clear_full_body_regions_for_later_validated_assembly', 'all_original_pages_viewed': [1],
        'regions': [{'kind': 'table', 'printed_number': '1', 'page': 1, 'source_image_sha256': image['sha256'],
                     'full_body_basis': 'Complete synthetic grid and separately printed note.', 'body_pdf_points': region,
                     'body_pixels_96dpi': [12, 16, 80, 56]}],
        scope: {'core_plus_notes_plus_markers_equals_primary': True, 'decision': 'Retain core, note and late superscript.',
                'original_superscript_markers': [marker], 'independent_core_nonwhitespace': 4, 'primary_full_body_nonwhitespace': 10}}
    docs['role_review'] = {'status': 'clear_scoped_zero_formal_kind_evidence_for_later_validation', 'truth_admitted': False,
        'root_full_original_pages_viewed': [1], 'formal_kind_counts_supported_by_root_visual_review': {k: 0 for k in ('algorithm', 'numbered_equation', 'proof', 'statement')},
        'retained_candidate_evidence': {'primary': [], 'independent': []}, 'independent_external_reference_retained': []}
    docs['object_crosswalk'] = {'source_inventory_sha256': candidate['source_inventory_sha256'],
        'objects': [{'primary_id': 'primary-table', 'independent_id': 'independent-table', 'source_id': parsed_object['id']}],
        'table_notes': [{'primary_id': 'primary-table', 'independent_id': 'independent-table', 'source_id': parsed_object['id'],
                         'independent_notes_key': group, 'review_scope_key': scope}]}
    docs['link_crosswalk'] = {'links': []}
    if with_links:
        def last_occurrence(value):
            start = text.rindex(value)
            return {'start': start, 'end': start + len(value), 'text': value, 'pages': [1], 'unit': 'UTF-16'}
        heading = source.index(r'\section{Context}'); label = source.index(r'\label{sec:a}')
        independent['nonobject_destinations'] = [{'id': 'section-1', 'printed_label': '1',
            'source_heading': src(heading, heading + len(r'\section{Context}')), 'source_label': src(label, label + len(r'\label{sec:a}')),
            'native_heading': last_occurrence('1 Context')}]
        for number, link in enumerate(parsed['links']):
            span = link['source_members'][0]; command = src(span['start'], span['end']); pid = 'primary-link-' + str(number); iid = 'independent-link-' + str(number)
            if link['kind'] == 'reference':
                target = link['targets'][0]; is_table = target == 'tab:a'; phrase = last_occurrence('Table 1' if is_table else 'Section 1')
                marker_span = {'start': phrase['end'] - 1, 'end': phrase['end'], 'text': '1'}
                primary['references'].append({'id': pid, 'source_command': command, 'source_target_label': target,
                    'target': 'primary-table' if is_table else 'section:sec:a', 'target_kind': 'table' if is_table else 'section',
                    'printed_number': '1', 'native_occurrence': phrase, 'native_number': marker_span})
                independent['references'].append({'id': iid, 'source_command': command, 'source_label': target, 'native': phrase,
                    'kind': 'object_reference' if is_table else 'section_reference', 'target_id': 'independent-table' if is_table else 'section-1', 'target_ambiguities': []})
            else:
                primary['citations'].append({'id': pid, 'source_command': command, 'target_keys': ['key'], 'native_occurrence': last_occurrence('[1]')})
                independent['citations'].append({'id': iid, 'source': command, 'source_keys': ['key'], 'native': last_occurrence('[1]')})
            docs['link_crosswalk']['links'].append({'source_link': number, 'kind': link['kind'], 'primary_id': pid, 'independent_id': iid})
    docs['review'] = {'format': REVIEW_FORMAT, 'verdict': 'clear_complete_visual_negative_inventory', 'findings': [],
        'pages_inspected': [1], 'annotators': {'primary': 'synthetic-first', 'independent': 'synthetic-second'}, 'reviewer': 'synthetic-third',
        'later_crosswalk_reviewed_after_annotation_freeze': True,
        'complete_kind_counts': {'figure': 0, 'table': 1, **{k: 0 for k in ('equation', 'statement', 'proof', 'algorithm')}},
        'reference_counts': {'local': 0, 'visual': 0, 'section': 0, 'external': 0, 'O4': 0}}
    if with_links:
        docs['review']['reference_counts'].update(local=2, visual=1, section=1)
    return candidate, index, source_raw, docs, {image['path']: image['sha256']}


def seal(candidate, docs):
    """Rebind hashes so adversarial mutations exercise semantics, not stale hashes."""
    def raw(k):
        return docs[k].encode() if k == 'prompt' else canonical(docs[k])
    def ref(k):
        value = raw(k); return {'sha256': sha256(value), 'bytes': len(value)}
    inputs = [{'path': k + '.json', **ref(k)} for k in ('packet', 'source_export', 'native_export', 'prompt')]
    inputs += deepcopy(docs['packet']['images'])
    docs['primary']['inputs'] = deepcopy(inputs)
    docs['primary_receipt'].update(inputs=deepcopy(inputs), inventory=ref('primary'))
    docs['independent_receipt'].update(inputs=deepcopy(inputs), packet_identities=deepcopy(docs['packet']),
                                     inventory_sha256=ref('independent')['sha256'], inventory_bytes=ref('independent')['bytes'])
    for key in ('visual_review', 'role_review'):
        docs[key]['inputs'] = {k: ref(k)['sha256'] for k in ('primary', 'independent', 'source_export', 'native_export')}
    candidate_raw = canonical(candidate)
    current = parse_project(docs['source_export']['text_members'])
    docs['construction'] = {'not_an_original_annotator_input': True,
        'candidate': {'sha256': sha256(candidate_raw), 'bytes': len(candidate_raw)},
        'original_artifacts': {k: ref(k) for k in ('primary', 'independent', 'packet', 'source_export', 'native_export')},
        'objects': deepcopy(candidate['source_inventory']['objects']), 'links': deepcopy(candidate['source_inventory']['links']),
        'current_source_inventory': current, 'current_source_inventory_sha256': sha256(canonical(current)),
        'changed_source_inventory_fields': sorted(k for k in set(current) | set(candidate['source_inventory'])
            if canonical(current.get(k)) != canonical(candidate['source_inventory'].get(k)))}
    docs['review']['candidate_sha256'] = sha256(candidate_raw)
    docs['review']['artifacts'] = {k: ref(k)['sha256'] for k in docs if k != 'review'}
    return {k: raw(k) for k in docs}


class VisualTrancheTests(unittest.TestCase):
    def validate(self, data):
        candidate, index, source_raw, docs, images = data
        return validate_visual(canonical(candidate), canonical(index), source_raw, seal(candidate, docs), images)

    def test_should_retain_separate_note_components_when_marker_follows_note_in_native_order(self):
        data = fixture(); result = self.validate(data)
        self.assertEqual({k: result[k] for k in data[0]}, data[0])
        self.assertFalse(result['accepted'])
        overlay = result['manual_object_overlay']; note = overlay['associated_content'][0]
        self.assertGreater(note['marker_spans'][0]['start'], note['text_spans'][0]['end'])
        self.assertEqual(len(note['spans']), 2)
        self.assertEqual(overlay['objects'], [])
        self.assertTrue(all(overlay['metric_eligibility'].values()))
        self.assertEqual(set(overlay['omitted_metrics']), {'O8', 'O9', 'O10', 'O11'})

    def test_should_support_original_group_names_when_crosswalk_explicitly_binds_other_table_numbers(self):
        for number in (1, 3, 12):
            with self.subTest(number=number):
                self.assertEqual(len(self.validate(fixture(number))['manual_figure_table_overlay']['objects']), 1)

    def test_should_accept_direct_table_membership_when_both_originals_record_no_attached_notes(self):
        data = fixture(); docs = data[3]
        docs['primary']['attached_table_notes'] = []
        docs['primary']['objects'][0]['native_body_members'] = deepcopy(docs['primary']['objects'][0]['native_core_members'])
        docs['independent']['ancillary'] = {}; docs['visual_review'].pop('table_3_note_scope')
        docs['object_crosswalk']['table_notes'] = []
        self.assertEqual(self.validate(data)['manual_object_overlay']['associated_content'], [])

    def test_should_reject_formal_or_nested_source_when_visual_only_ownership_is_claimed(self):
        data = fixture(); candidate, index, raw, docs, _ = data; files, _ = read_archive(raw)
        added = deepcopy(candidate['source_inventory']['objects'][0]); added['id'] += ':nested'
        candidate['source_inventory']['objects'].append(added)
        primary = deepcopy(docs['primary']['objects'][0]); primary['id'] += ':nested'; docs['primary']['objects'].append(primary)
        independent = deepcopy(docs['independent']['objects'][0]); independent['id'] += ':nested'; docs['independent']['objects'].append(independent)
        with self.assertRaisesRegex(ValueError, 'nested or overlapping'):
            validate_objects(docs, candidate, files, index['index'])
        candidate['source_inventory']['objects'][1]['kind'] = 'equation'
        with self.assertRaisesRegex(ValueError, 'positive nonvisual'):
            validate_objects(docs, candidate, files, index['index'])

    def test_should_preserve_unknown_external_locator_when_review_retains_its_ambiguity(self):
        data = fixture(); docs = data[3]
        row = {'id': 'external', 'kind': 'external_section_reference', 'source': deepcopy(docs['primary']['objects'][0]['source_caption']),
               'native': deepcopy(docs['independent']['objects'][0]['caption_native']), 'target_subsection': None, 'target_ambiguities': ['External locator unknown.']}
        docs['independent']['references'].append(row)
        docs['role_review']['independent_external_reference_retained'] = [deepcopy(row)]
        docs['review']['reference_counts']['external'] = 1
        retained = self.validate(data)['manual_object_overlay']['associated_content'][-1]['evidence']
        self.assertIsNone(retained['target_subsection'])
        for value in ('1.2',):
            docs['independent']['references'][-1]['target_subsection'] = value
            docs['role_review']['independent_external_reference_retained'][-1]['target_subsection'] = value
            with self.assertRaisesRegex(ValueError, 'without evidence'): self.validate(data)

    def test_should_reject_note_crosswalk_when_group_or_owner_is_omitted_reused_or_changed(self):
        mutations = [lambda d: d['object_crosswalk'].update(table_notes=[]),
            lambda d: d['object_crosswalk']['table_notes'].append(deepcopy(d['object_crosswalk']['table_notes'][0])),
            lambda d: d['object_crosswalk']['table_notes'][0].update(source_id='another-object'),
            lambda d: d['object_crosswalk']['table_notes'][0].update(independent_id='another-table'),
            lambda d: d['object_crosswalk']['table_notes'][0].update(independent_notes_key='../notes'),
            lambda d: d['independent']['ancillary'].update(table_4_notes=[])]
        for mutate in mutations:
            data = fixture(); mutate(data[3])
            with self.subTest(mutation=mutate), self.assertRaises(ValueError): self.validate(data)

    def test_should_reject_table_note_membership_when_marker_text_or_core_ownership_changes(self):
        for mutate in [lambda d: d['primary']['attached_table_notes'][0]['native_body_members'].pop(),
            lambda d: d['primary']['objects'][0]['native_body_members'].pop(),
            lambda d: d['visual_review']['table_3_note_scope']['original_superscript_markers'][0].update(text='2'),
            lambda d: d['primary']['attached_table_notes'][0]['native_body_members'].append(deepcopy(d['primary']['objects'][0]['native_core_members'][0]))]:
            data = fixture(); mutate(data[3])
            with self.subTest(mutation=mutate), self.assertRaises(ValueError): self.validate(data)

    def test_should_reject_review_when_complete_negative_counts_or_independence_are_changed(self):
        for mutate in [lambda d: d['primary']['counts'].update(unnumbered_display_equation=1),
            lambda d: d['independent']['counts'].update(formal_proofs=1),
            lambda d: d['review']['complete_kind_counts'].update(table=False),
            lambda d: d['review'].update(reviewer='synthetic-first'),
            lambda d: d['review'].update(findings=['unresolved']),
            lambda d: d['review'].update(pages_inspected=[]),
            lambda d: d['primary']['protocol'].update(all_original_pages_viewed_before_source_and_native=False)]:
            data = fixture(); mutate(data[3])
            with self.subTest(mutation=mutate), self.assertRaises(ValueError): self.validate(data)

    def test_should_reject_original_image_changes_when_dpi_dimensions_or_page_hash_differ(self):
        for change in ({'dpi': 192}, {'pixel_width': 201}, {'pdf_height_points': 149}, {'sha256': 'c' * 64}):
            data = fixture(); data[3]['packet']['images'][0].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError): self.validate(data)

    def test_should_reject_unreviewed_prose_role_when_new_candidate_is_added(self):
        data = fixture(); doc = data[3]
        doc['primary']['definition_candidates'].append({'id': 'new-definition', 'source': doc['primary']['objects'][0]['source_caption']})
        with self.assertRaisesRegex(ValueError, 'informal candidate'): self.validate(data)

    def test_should_reject_source_membership_when_note_escapes_its_table(self):
        data = fixture(); doc = data[3]; source = data[2].decode(); start = source.index(r'\end{document}')
        doc['primary']['attached_table_notes'][0]['source_note'] = {'member': next(iter(doc['source_export']['text_members'])),
            'start': start, 'end': len(source), 'text': source[start:]}
        with self.assertRaisesRegex(ValueError, 'source owner'): self.validate(data)

    def test_should_reject_remote_core_when_attached_notes_stay_on_the_declared_table_page(self):
        candidate, wrapped, raw, docs, _ = fixture(); index = wrapped['index']; start = len(index['text'])
        index['text'] += 'Cell'; index['pages'].append({'number': 2, 'start': start, 'end': start + 4, 'width': 150, 'height': 150})
        remote = {'start': start, 'end': start + 4, 'text': 'Cell', 'pages': [2], 'unit': 'UTF-16'}
        docs['independent']['objects'][0]['full_visual_body']['native_members'] = [remote]
        docs['primary']['objects'][0]['native_core_members'] = [deepcopy(remote)]
        docs['primary']['objects'][0]['native_body_members'] = docs['primary']['objects'][0]['native_body_members'][1:] + [deepcopy(remote)]
        files, _ = read_archive(raw)
        with self.assertRaisesRegex(ValueError, 'core crosses'):
            validate_objects(docs, candidate, files, index)

    def test_should_retain_context_excerpt_when_its_exact_native_text_crosses_pages(self):
        index = {'text': 'First\nSecond', 'pages': [{'number': 1, 'start': 0, 'end': 6}, {'number': 2, 'start': 6, 'end': 12}]}
        audit_excerpts({'context': {'start': 0, 'end': 12, 'text': 'First\nSecond', 'pages': [1, 2], 'unit': 'UTF-16'}}, {}, index)
        with self.assertRaisesRegex(ValueError, 'excerpt differs'):
            audit_excerpts({'start': 0, 'end': 12, 'text': 'Changed text', 'unit': 'UTF-16'}, {}, index)

    def test_should_preserve_all_source_links_when_visual_section_and_citation_roles_differ(self):
        data = fixture(with_links=True); result = self.validate(data); overlay = result['manual_object_overlay']
        self.assertEqual(len(overlay['other_object_references']), 1)
        self.assertEqual(len(overlay['non_object_references']), 1)
        self.assertEqual(overlay['references'], [])
        self.assertEqual(result['source_inventory']['links'], data[0]['source_inventory']['links'])
        self.assertNotIn('manual_bibliography_overlay', result)

    def test_should_reject_link_changes_when_source_occurrences_targets_or_exact_native_membership_differ(self):
        for mutate in [lambda d: d['primary']['references'].pop(),
            lambda d: d['independent']['references'][0].update(target_id='section-1'),
            lambda d: d['independent']['references'][0].update(target_ambiguities=['unresolved']),
            lambda d: d['link_crosswalk']['links'].pop(),
            lambda d: d['primary']['citations'][0].update(target_keys=['other-key']),
            lambda d: d['independent']['citations'].clear()]:
            data = fixture(with_links=True); mutate(data[3])
            with self.subTest(mutation=mutate), self.assertRaises(ValueError): self.validate(data)

    def test_should_reject_section_reclassification_when_current_source_label_owns_a_visual_object(self):
        data = fixture(with_links=True); docs = data[3]
        docs['primary']['references'][0].update(target='section:tab:a', target_kind='section')
        docs['independent']['references'][0].update(kind='section_reference', target_id='section-1')
        with self.assertRaisesRegex(ValueError, 'reclassified as a section'): self.validate(data)

    def test_should_reject_empty_or_reused_number_when_distinct_source_references_are_retained(self):
        data = fixture(with_links=True); docs = data[3]; row = docs['primary']['references'][0]
        row['printed_number'] = ' '; start = row['native_occurrence']['end'] - 2
        row['native_number'] = {'start': start, 'end': start + 1, 'text': ' '}
        with self.assertRaisesRegex(ValueError, 'reference number'): self.validate(data)
        data = fixture(with_links=True); docs = data[3]
        docs['primary']['references'][1]['native_occurrence'] = deepcopy(docs['primary']['references'][0]['native_occurrence'])
        docs['primary']['references'][1]['native_number'] = deepcopy(docs['primary']['references'][0]['native_number'])
        docs['independent']['references'][1]['native'] = deepcopy(docs['independent']['references'][0]['native'])
        with self.assertRaisesRegex(ValueError, 'reuse a native occurrence'): self.validate(data)

    def test_should_reject_wrong_section_heading_when_its_label_is_owned_by_another_source_command(self):
        data = fixture(with_links=True); docs = data[3]
        docs['independent']['nonobject_destinations'][0]['source_heading'] = deepcopy(docs['independent']['objects'][0]['caption_source'])
        with self.assertRaisesRegex(ValueError, 'heading does not own'): self.validate(data)

    def test_should_preserve_shared_context_when_separate_source_refs_have_distinct_numeric_members(self):
        data = fixture(with_links=True); docs = data[3]; text = data[1]['index']['text']
        start = text.index('See Table'); end = text.index('. [1]', start)
        phrase = {'start': start, 'end': end, 'text': text[start:end], 'pages': [1], 'unit': 'UTF-16'}
        for primary, independent in zip(docs['primary']['references'], docs['independent']['references']):
            primary['native_occurrence'] = deepcopy(phrase); independent['native'] = deepcopy(phrase)
        overlay = self.validate(data)['manual_object_overlay']
        self.assertEqual(overlay['other_object_references'][0]['occurrence_spans'], overlay['non_object_references'][0]['occurrence_spans'])
        self.assertNotEqual(overlay['other_object_references'][0]['spans'], overlay['non_object_references'][0]['spans'])

    def test_should_dispatch_explicit_format_when_legacy_and_visual_manifests_are_distinct(self):
        from manual import attach_declared_tranche
        from tranche import FORMAT as LEGACY_FORMAT
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'bundle').mkdir(); manifest = root / 'bundle/manifest.json'
            with patch('tranche.attach_tranche', return_value={'legacy': True}) as legacy, patch('tranche_visual.attach_visual', return_value={'visual': True}) as visual:
                for declared, expected in ((LEGACY_FORMAT, {'legacy': True}), (FORMAT, {'visual': True})):
                    manifest.write_bytes(canonical({'format': declared}))
                    self.assertEqual(attach_declared_tranche(root, root, root, b'{}', {}, {}, 'bundle'), expected)
                self.assertEqual(legacy.call_count, 1); self.assertEqual(visual.call_count, 1)
                manifest.write_bytes(canonical({'format': 'unknown'}))
                with self.assertRaisesRegex(ValueError, 'unsupported explicit'):
                    attach_declared_tranche(root, root, root, b'{}', {}, {}, 'bundle')
                self.assertEqual(legacy.call_count, 1); self.assertEqual(visual.call_count, 1)

    def test_should_reject_dispatch_when_manifest_path_is_symlinked_or_changes_during_codec(self):
        from manual import attach_declared_tranche
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'bundle').mkdir(); original = root / 'original.json'
            original.write_bytes(canonical({'format': FORMAT})); manifest = root / 'bundle/manifest.json'
            manifest.symlink_to(original)
            with self.assertRaises(ValueError): attach_declared_tranche(root, root, root, b'{}', {}, {}, 'bundle')
            manifest.unlink(); manifest.write_bytes(original.read_bytes())
            def changed(*args):
                manifest.write_bytes(canonical({'format': 'changed'})); return {}
            with patch('tranche_visual.attach_visual', side_effect=changed), self.assertRaisesRegex(ValueError, 'manifest changed'):
                attach_declared_tranche(root, root, root, b'{}', {}, {}, 'bundle')

    def test_should_use_current_duplicate_label_guard_when_historical_inventory_claimed_unique_target(self):
        data = fixture(with_links=True); candidate, index, source, docs, images = data
        current = deepcopy(candidate['source_inventory']); current['ambiguous_labels'] = {'tab:a': ['first', 'second']}
        with patch('tranche_visual.parse_project', return_value=current), patch('test_tranche_visual.parse_project', return_value=current):
            with self.assertRaisesRegex(ValueError, 'ambiguous'):
                self.validate(data)

    def test_should_retain_historical_source_differences_when_current_fields_are_diagnostic_only(self):
        data = fixture(); current = deepcopy(data[0]['source_inventory'])
        current['coverage']['new_reviewed_source_diagnostic'] = 'Kept separately from historical acceptance.'
        with patch('tranche_visual.parse_project', return_value=current), patch('test_tranche_visual.parse_project', return_value=current):
            result = self.validate(data)
        self.assertEqual(result['source_inventory'], data[0]['source_inventory'])
        self.assertEqual(result['manual_object_overlay']['historical_source_difference_fields'], ['coverage'])
