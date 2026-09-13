"""Offline semantic/ownership boundaries for independently reviewed object truth."""
from copy import deepcopy
import unittest
import json
import tempfile
from pathlib import Path
from manual import attach_objects

from annotations import canonical
from archive import read_archive, sha256
from object_annotations import apply_object_overlay
from parser import parse_project
from native_exports import FORMAT, native_projection


def fixture(extra_object_kind=None, extra_source='', proof_heading='', statement_alias='', reference_label='thm:a'):
    source = (r'\documentclass{article}\begin{document}\begin{theorem}\label{thm:a}A complete theorem body.\end{theorem}'
              r'\begin{equation}\label{eq:a}abcdefghi=12345\end{equation}\begin{proof}A complete proof body.\end{proof}'
              r'\begin{algorithm}\label{alg:a}A complete algorithm body.\end{algorithm}See \ref{thm:a}.\section{Related}\label{sec:related}See \ref{sec:related}.\end{document}')
    if extra_object_kind:
        source = source.replace(r'\end{document}', '\\begin{' + extra_object_kind + r'}\caption{A separately displayed visual caption.}\label{visual:a}\end{' + extra_object_kind + r'}See \ref{visual:a}.\end{document}')
    source = source.replace(r'\label{thm:a}', r'\label{thm:a}' + statement_alias)
    source = source.replace(r'\ref{thm:a}', '\\ref{' + reference_label + '}')
    source = source.replace(r'\end{document}', extra_source + r'\end{document}')
    source = source.replace(r'\begin{proof}', r'\begin{proof}' + proof_heading)
    source = source.encode()
    files, members = read_archive(source); parsed = parse_project(files)
    text, rows, root_rows = '', [], []
    for number, obj in enumerate(row for row in parsed['objects'] if row['kind'] in {'statement', 'equation', 'proof', 'algorithm'}):
        body = obj['text']; start = len(text); text += body + '\n'
        span = {'start': start, 'end': start + len(body), 'text': body, 'text_utf8_sha256': sha256(body.encode())}
        source_members = [{**row, 'utf8_sha256': sha256(files[row['path']][row['start']:row['end']].encode())} for row in obj['source_members']]
        rows.append({'id': obj['id'], 'kind': obj['kind'], 'labels': obj['labels'], 'printed_heading': obj['kind'], 'source_members': source_members,
                     'direct_index_spans': [span], 'visual_body_boxes': [{'page': 1, 'pixels_96dpi': [0, number * 40, 100, number * 40 + 30], 'pdf_points': {'x_min': 0, 'y_min': number * 30, 'x_max': 75, 'y_max': number * 30 + 22.5}}], 'source_parent_object': None, 'source_child_objects': [], 'source_vs_pdf_math': {'judgment': 'visually agrees'}, 'reading_index_fidelity': {'judgment': 'lossy'}})
        root_rows.append({'source_id': obj['id'], 'proof_targets': obj['proof_targets'], 'source_members': source_members, 'separately_annotated_nested_objects': [], 'spans': [{**span, 'native_text_sha256': span['text_utf8_sha256']}], 'regions': [{'page': 1, 'rect': rows[-1]['visual_body_boxes'][0]['pdf_points'], 'role': 'body'}]})
    ref_start = len(text); text += '1 2' + (' 3' if extra_object_kind else '')
    index = {'index': {'text': text, 'tokens': [], 'pages': [{'number': 1, 'start': 0, 'end': len(text), 'width': 612, 'height': 792}]}}
    candidate = {'arxiv_id': '2001.00001', 'paper_id': 'actual-mapped-id', 'pdf_sha256': '1' * 64, 'source_sha256': sha256(source), 'index': {'path': 'papers/actual/index.json', 'sha256': sha256(canonical(index))}, 'source_inventory': parsed, 'source_inventory_sha256': sha256(canonical(parsed)), 'automatic_exclusions': ['unchanged']}
    image = {'path': 'page-1.png', 'sha256': '2' * 64}
    packet = {'arxiv_id': candidate['arxiv_id'], 'paper_id': candidate['paper_id'], 'candidate_sha256': sha256(canonical(candidate)), 'inventory_sha256': candidate['source_inventory_sha256'], 'index': candidate['index'], 'pdf': {'sha256': candidate['pdf_sha256']}, 'source': {'sha256': candidate['source_sha256']}, 'images': [image], 'page_text': [{'page': 1, 'text': text}], 'objects': parsed['objects'], 'links': parsed['links']}
    number = next(i for i, row in enumerate(parsed['links']) if row['kind'] == 'reference')
    native_ref = {'start': ref_start, 'end': ref_start + 1, 'text': '1', 'text_utf8_sha256': sha256(b'1')}
    root = {**{key: candidate[key] for key in ('arxiv_id', 'paper_id', 'pdf_sha256', 'source_sha256', 'index', 'source_inventory_sha256')}, 'annotator': 'root', 'candidate_sha256': sha256(canonical(candidate)), 'packet_sha256': sha256(canonical(packet)), 'attestation': {'detector_outputs_read': False, 'other_annotator_labels_read': False}, 'viewed_all_pages': [1], 'page_images': [image], 'complete_visual_inventory': {'equation': 1, 'statement': 1, 'proof': 1, 'algorithm': 1}, 'objects': root_rows, 'references': [{'source_link': number, 'target': 'object:thm:a', 'span': {'start': ref_start, 'end': ref_start + 1, 'native_text_sha256': sha256(b'1')}}]}
    independent = {'annotator': 'independent', 'complete_inventory': {'paper_complete_for_scope': True, 'counts': {'equation': 1, 'statement': 1, 'proof': 1, 'algorithm': 1}}, 'inputs': {'candidate_sha256_from_packet': sha256(canonical(candidate)), 'inventory_sha256_from_packet': candidate['source_inventory_sha256'], 'packet': {'sha256': sha256(canonical(packet))}, 'pdf': {'sha256': candidate['pdf_sha256']}, 'source': {'sha256': candidate['source_sha256']}, 'reading_index': {'sha256': sha256(canonical(index))}, 'source_members': members, 'original_images': [{**image, 'viewed_full_original': True}]}, 'objects': rows, 'associated_content': [], 'proof_attribution': [{'proof': 'object:proof:1', 'statement': 'object:thm:a', 'explicit_source_ref': False}], 'object_references': [{'source_members': parsed['links'][number]['source_members'], 'target': 'object:thm:a', 'number_occurrence': native_ref}]}
    receipt = {'schema_version': 1, 'annotation': {}, 'detail_crops': []}
    comparison = {'schema_version': 1, 'verdict': 'clear_complete_object_overlay', 'findings': [], 'objects_verified': [row['id'] for row in rows], 'object_reference_source_links_verified': [number], 'non_object_references_verified': [], 'complete_inventory_verified': dict(root['complete_visual_inventory']), 'proof_attribution_verified': deepcopy(independent['proof_attribution']), 'comparisons': [{'source_id': row['id'], 'geometry': [{'page': box['page'], 'root_rect': dict(box['pdf_points']), 'independent_rect': dict(box['pdf_points']), 'disposition': 'accepted; reviewed'} for box in row['visual_body_boxes']]} for row in rows]}
    section_number = next(i for i, row in enumerate(parsed['links']) if row['targets'] == ['sec:related'])
    section_source = parsed['links'][section_number]['source_members']
    section_span = {'start': ref_start + 2, 'end': ref_start + 3, 'native_text_sha256': sha256(b'2')}
    root['references'].append({'source_link': section_number, 'target': 'section:related', 'source_target_label': 'sec:related', 'source_members': section_source, 'printed': '2', 'span': section_span})
    comparison['non_object_references_verified'] = [{'source_link': section_number, 'target': 'section:related', 'source_label': 'sec:related', 'source_members': deepcopy(section_source), 'printed': '2', 'span': {'start': ref_start + 2, 'end': ref_start + 3}}]
    if extra_object_kind:
        visual_number = next(i for i, row in enumerate(parsed['links']) if row['targets'] == ['visual:a'])
        visual_source = parsed['links'][visual_number]['source_members']
        visual_span = {'start': ref_start + 4, 'end': ref_start + 5, 'text': '3', 'text_utf8_sha256': sha256(b'3')}
        root['references'].append({'source_link': visual_number, 'target': 'object:visual:a', 'span': {**visual_span, 'native_text_sha256': sha256(b'3')}})
        independent['object_references'].append({'source_members': visual_source, 'target': 'object:visual:a', 'number_occurrence': visual_span})
        comparison['object_reference_source_links_verified'].append(visual_number)
        root['complete_visual_inventory'][extra_object_kind] = 1
        comparison['complete_inventory_verified'][extra_object_kind] = 1
    result = [candidate, index, source, packet, root, independent, receipt, comparison, {'page-1.png': '2' * 64}]
    reseal(result); return result


def reseal(rows):
    rows[6]['annotation'].update(sha256=sha256(canonical(rows[5])), bytes=len(canonical(rows[5])))
    rows[7].update(root_annotation_sha256=sha256(canonical(rows[4])), independent_annotation_sha256=sha256(canonical(rows[5])), independent_receipt_sha256=sha256(canonical(rows[6])))


def materialize(cache):
    rows = fixture(); candidate, index, source, packet, root, independent, receipt, _, _ = rows
    corpus, data, bundle = cache/'corpus', cache/'data', cache/'bundle'
    corpus.mkdir(); (data/'papers/actual').mkdir(parents=True); bundle.mkdir()
    pdf = b'PDF fixture'; image = b'image fixture'; serializer = b'inert serializer fixture'
    candidate['pdf_sha256'] = sha256(pdf); root['pdf_sha256'] = sha256(pdf)
    paper = {'arxiv_id': candidate['arxiv_id'], 'pdf': {'path': 'paper.pdf', 'sha256': sha256(pdf), 'bytes': len(pdf)}, 'source': {'path': 'paper.src', 'sha256': sha256(source), 'bytes': len(source)}}
    packet['pdf'] = paper['pdf']; packet['source'] = paper['source']
    candidate_hash = sha256(canonical(candidate)); packet['candidate_sha256'] = candidate_hash
    root['candidate_sha256'] = candidate_hash; independent['inputs']['candidate_sha256_from_packet'] = candidate_hash
    for row in packet['images'] + root['page_images'] + independent['inputs']['original_images']: row['sha256'] = sha256(image)
    independent['inputs']['original_images'][0]['path'] = str(bundle/'page-1.png')
    packet_hash = sha256(canonical(packet)); root['packet_sha256'] = packet_hash
    independent['inputs']['packet'] = {'sha256': packet_hash, 'path': str(bundle/'packet.json')}
    independent['inputs']['pdf'] = {'sha256': sha256(pdf), 'path': str(corpus/'paper.pdf')}
    independent['inputs']['source']['path'] = str(corpus/'paper.src')
    independent['inputs']['reading_index']['path'] = str(data/candidate['index']['path'])
    receipt['serializer'] = {'path': str(cache/'serializer.py'), 'bytes': len(serializer), 'sha256': sha256(serializer)}
    receipt['annotation']['path'] = str(bundle/'independent-v1.json')
    reseal(rows)
    for name, value in [('packet.json', packet), ('root-v1.json', root), ('independent-v1.json', independent), ('independent-v1-receipt.json', receipt), ('reconciliation-independent-v1.json', rows[7])]: (bundle/name).write_bytes(canonical(value))
    (corpus/'paper.pdf').write_bytes(pdf); (corpus/'paper.src').write_bytes(source)
    (data/candidate['index']['path']).write_bytes(canonical(index)); (bundle/'page-1.png').write_bytes(image); (cache/'serializer.py').write_bytes(serializer)
    mapped = {'paper_id': candidate['paper_id'], 'index': candidate['index']}
    return (cache, corpus, data, canonical(candidate), paper, mapped, 'bundle')


def apply(rows):
    return apply_object_overlay(*(value if i in (2, 8) else canonical(value) for i, value in enumerate(rows)))


class ObjectAnnotationTests(unittest.TestCase):
    def test_should_reject_manual_destinations_when_a_dynamic_claim_can_alias_static_labels(self):
        for launder in (False, True):
            rows = fixture(extra_source=r'\label{\alias}')
            if launder:
                reference = rows[4]['references'][0]; number = reference['source_link']
                member = rows[0]['source_inventory']['links'][number]['source_members']
                reference.update(target='section:invented', source_target_label='thm:a', source_members=member, printed='1')
                rows[5]['object_references'] = []
                rows[7]['object_reference_source_links_verified'] = []
                rows[7]['non_object_references_verified'].append({
                    'source_link': number, 'target': 'section:invented', 'source_label': 'thm:a',
                    'source_members': member, 'printed': '1', 'span': {k: reference['span'][k] for k in ('start', 'end')}})
                reseal(rows)
            with self.subTest(launder=launder), self.assertRaisesRegex(ValueError, 'unverified source label names'):
                apply(rows)

    def test_should_reject_manual_named_proofs_when_a_dynamic_claim_can_alias_the_target(self):
        rows = fixture(extra_source=r'\label{\alias}', proof_heading=r'[Proof of \ref{thm:a}]')
        rows[5]['proof_attribution'][0]['explicit_source_ref'] = True
        rows[4]['objects'][2]['proof_targets'] = ['object:thm:a']
        reseal(rows)
        with self.assertRaisesRegex(ValueError, 'unverified source label names.*proof'):
            apply(rows)

    def test_should_bind_manual_reference_when_one_literal_comma_alias_is_unique(self):
        rows = fixture(statement_alias=r'\label{a,b}', reference_label='a,b')
        self.assertEqual(apply(rows)['manual_object_overlay']['references'][0]['target'], 'object:thm:a')

    def test_should_reject_manual_named_proof_when_a_literal_comma_alias_is_ambiguous(self):
        rows = fixture(statement_alias=r'\label{a,b}', reference_label='a,b', extra_source=r'\label{a,b}',
                       proof_heading=r'[Proof of \ref{a,b}]')
        rows[5]['proof_attribution'][0]['explicit_source_ref'] = True
        rows[4]['objects'][2]['proof_targets'] = ['object:thm:a']
        reseal(rows)
        with self.assertRaisesRegex(ValueError, 'ambiguous source labels'):
            apply(rows)

    def test_should_reject_manual_destinations_when_object_and_nonobject_labels_collide(self):
        for extra in (r'\label{thm:a}', r'\begin{enumerate}\item Other claim.\label{thm:a}\end{enumerate}'):
            for launder in (False, True):
                rows = fixture(extra_source=extra)
                self.assertIn('thm:a', rows[0]['source_inventory']['ambiguous_labels'])
                if launder:
                    reference = rows[4]['references'][0]; number = reference['source_link']
                    member = rows[0]['source_inventory']['links'][number]['source_members']
                    reference.update(target='section:invented', source_target_label='thm:a', source_members=member, printed='1')
                    rows[5]['object_references'] = []
                    rows[7]['object_reference_source_links_verified'] = []
                    rows[7]['non_object_references_verified'].append({
                        'source_link': number, 'target': 'section:invented', 'source_label': 'thm:a',
                        'source_members': member, 'printed': '1', 'span': {k: reference['span'][k] for k in ('start', 'end')}})
                    reseal(rows)
                with self.subTest(extra=extra, launder=launder), self.assertRaisesRegex(ValueError, 'ambiguous source reference'):
                    apply(rows)

    def test_should_reject_manual_proof_attribution_when_explicit_label_is_ambiguous(self):
        rows = fixture(extra_source=r'\label{thm:a}', proof_heading=r'[Proof of \ref{thm:a}]')
        rows[5]['proof_attribution'][0]['explicit_source_ref'] = True
        # Even agreement between annotators cannot invent a unique source name.
        rows[4]['objects'][2]['proof_targets'] = ['object:thm:a']
        reseal(rows)
        with self.assertRaisesRegex(ValueError, 'ambiguous source labels'):
            apply(rows)

    def test_should_retain_visual_reference_roles_when_a_paper_also_contains_math_objects(self):
        for kind in ('figure', 'table'):
            rows = fixture(kind)
            overlay = apply(rows)['manual_object_overlay']
            with self.subTest(kind=kind):
                self.assertEqual(overlay['reference_coverage'], {'source_occurrences': 3, 'object_occurrences': 2, 'O4_occurrences': 1})
                self.assertEqual(overlay['other_object_references'][0]['target_kind'], kind)
                self.assertEqual(overlay['other_object_references'][0]['target'], 'object:visual:a')
                self.assertEqual(len(overlay['references']), 1)
                self.assertEqual(len(overlay['non_object_references']), 1)

    def test_should_reject_visual_reference_role_laundering_when_source_labels_identify_a_figure(self):
        rows = fixture('figure'); ref = rows[4]['references'][-1]; number = ref['source_link']
        ref.update(target='section:invented', source_target_label='visual:a')
        rows[5]['object_references'].pop(); rows[7]['object_reference_source_links_verified'].remove(number)
        rows[7]['non_object_references_verified'].append({'source_link': number, 'target': 'section:invented'})
        reseal(rows)
        with self.assertRaisesRegex(ValueError, 'object reference cannot'): apply(rows)

    def test_should_reject_incomplete_visual_reference_coverage_when_an_annotation_omits_the_occurrence(self):
        rows = fixture('table'); rows[5]['object_references'].pop(); reseal(rows)
        with self.assertRaises(ValueError): apply(rows)

    def test_should_preserve_automatic_truth_when_complete_manual_membership_is_attached(self):
        rows = fixture(); before = deepcopy(rows[0]); result = apply(rows)
        self.assertEqual(rows[0], before)
        self.assertEqual({key: result[key] for key in before}, before)
        overlay = result['manual_object_overlay']
        self.assertTrue(all(overlay['metric_eligibility'].values()))
        self.assertFalse(overlay['final_k1_publication'])
        self.assertTrue(all(row['semantic_quote_truth'] is False for row in overlay['objects']))
        self.assertEqual(overlay['reference_coverage']['O4_occurrences'], 1)

    def test_should_verify_actual_object_evidence_when_the_safe_file_boundary_assembles_a_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            arguments = materialize(Path(directory))
            self.assertEqual(len(attach_objects(*arguments)['manual_object_overlay']['objects']), 4)
            (Path(directory)/'bundle/page-1.png').write_bytes(b'changed image')
            with self.assertRaisesRegex(ValueError, 'original/detail image differs'): attach_objects(*arguments)

    def test_should_bind_the_actual_blind_export_when_annotators_did_not_read_raw_index_predictions(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory); arguments = materialize(cache)
            index_raw = (cache/'data/papers/actual/index.json').read_bytes()
            exported = canonical(native_projection(index_raw, 'actual-mapped-id'))
            (cache/'native.json').write_bytes(exported)
            bundle = cache/'bundle'
            independent = json.loads((bundle/'independent-v1.json').read_bytes())
            independent['inputs']['reading_index'].pop('path')
            independent['inputs']['native_export'] = {'format': FORMAT, 'path': str(cache/'native.json'), 'sha256': sha256(exported), 'bytes': len(exported), 'index_sha256': sha256(index_raw)}
            independent_raw = canonical(independent); (bundle/'independent-v1.json').write_bytes(independent_raw)
            receipt = json.loads((bundle/'independent-v1-receipt.json').read_bytes())
            receipt['annotation'].update(sha256=sha256(independent_raw), bytes=len(independent_raw))
            receipt_raw = canonical(receipt); (bundle/'independent-v1-receipt.json').write_bytes(receipt_raw)
            comparison = json.loads((bundle/'reconciliation-independent-v1.json').read_bytes())
            comparison.update(independent_annotation_sha256=sha256(independent_raw), independent_receipt_sha256=sha256(receipt_raw))
            (bundle/'reconciliation-independent-v1.json').write_bytes(canonical(comparison))
            result = attach_objects(*arguments)
            self.assertEqual(result['manual_object_overlay']['verified_native_export']['sha256'], sha256(exported))

    def test_should_reject_changed_source_or_serializer_when_manual_receipts_claim_old_bytes(self):
        for relative in ('corpus/paper.src', 'serializer.py'):
            with self.subTest(relative=relative), tempfile.TemporaryDirectory() as directory:
                arguments = materialize(Path(directory)); (Path(directory)/relative).write_bytes(b'changed')
                with self.assertRaises(ValueError): attach_objects(*arguments)

    def test_should_reject_pending_comparison_when_original_annotations_are_valid(self):
        rows = fixture(); rows[7]['verdict'] = 'pending'
        with self.assertRaisesRegex(ValueError, 'not independently cleared'): apply(rows)

    def test_should_reject_partial_inventory_when_a_complete_review_omits_an_object_or_reference(self):
        for field in ('objects', 'object_references', 'proof_attribution'):
            rows = fixture(); rows[5][field].pop(); reseal(rows)
            with self.subTest(field=field), self.assertRaises(ValueError): apply(rows)

    def test_should_reject_wrong_anchor_or_geometry_when_all_receipt_hashes_are_consistent(self):
        for mutate in (lambda r: r['direct_index_spans'][0].update(end=999999), lambda r: r['visual_body_boxes'][0]['pdf_points'].update(x_max=999), lambda r: r['source_members'][0].update(start=0), lambda r: r.update(source_child_objects=['object:proof:1'])):
            rows = fixture(); mutate(rows[5]['objects'][0]); reseal(rows)
            with self.subTest(mutate=mutate), self.assertRaises(ValueError): apply(rows)

    def test_should_reject_missing_ancillary_membership_when_root_review_retains_its_text(self):
        rows = fixture()
        # The first complete native body was independently split into direct and
        # ancillary membership. Removing the latter must lose reviewed coverage.
        body = rows[5]['objects'][0]['direct_index_spans'][0]
        original = body['text']; split = original.index(' ') + 1
        body.update(end=body['start'] + split, text=original[:split], text_utf8_sha256=sha256(original[:split].encode()))
        reseal(rows)
        with self.assertRaisesRegex(ValueError, 'direct and ancillary ownership'): apply(rows)

    def test_should_reject_missing_nested_edges_when_source_environments_are_nested(self):
        rows = fixture(); parsed = rows[0]['source_inventory']
        parent, child = parsed['objects'][0], parsed['objects'][1]
        # A separate expanded-source span models nesting while the deposited
        # original memberships remain independently bound and unchanged.
        parent['source_span']['end'] = child['source_span']['end'] + 1
        rows[0]['source_inventory_sha256'] = sha256(canonical(parsed))
        rows[3]['inventory_sha256'] = rows[0]['source_inventory_sha256']
        rows[4]['source_inventory_sha256'] = rows[0]['source_inventory_sha256']
        rows[5]['inputs']['inventory_sha256_from_packet'] = rows[0]['source_inventory_sha256']
        candidate_hash = sha256(canonical(rows[0]))
        rows[3]['candidate_sha256'] = candidate_hash; rows[4]['candidate_sha256'] = candidate_hash
        rows[5]['inputs']['candidate_sha256_from_packet'] = candidate_hash
        packet_hash = sha256(canonical(rows[3])); rows[4]['packet_sha256'] = packet_hash
        rows[5]['inputs']['packet']['sha256'] = packet_hash; reseal(rows)
        with self.assertRaisesRegex(ValueError, 'ownership omits'): apply(rows)

    def test_should_reject_contradictory_reconciliation_when_counts_proof_or_boxes_disagree(self):
        mutations = [lambda r: r[7]['complete_inventory_verified'].update(statement=0),
                     lambda r: r[7]['proof_attribution_verified'][0].update(statement='object:missing'),
                     lambda r: r[7]['comparisons'][0]['geometry'][0]['independent_rect'].update(x_max=74)]
        for mutation in mutations:
            rows = fixture(); mutation(rows); reseal(rows)
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'comparison.*contradicts'): apply(rows)

    def test_should_reject_reference_role_laundering_when_source_labels_prove_an_object(self):
        rows = fixture(); reference = rows[4]['references'][0]; number = reference['source_link']
        reference.update(target='section:invented', source_target_label='thm:a')
        rows[5]['object_references'] = []; rows[7]['object_reference_source_links_verified'] = []
        rows[7]['non_object_references_verified'].append({'source_link': number, 'target': 'section:invented'})
        reseal(rows)
        with self.assertRaisesRegex(ValueError, 'object reference cannot'): apply(rows)

    def test_should_reject_fabricated_non_object_anchors_when_reviewed_roles_are_valid(self):
        mutations = [lambda row: row['span'].update(start=999999, end=1000000),
                     lambda row: row['source_members'][0].update(start=0),
                     lambda row: row.update(printed='invented')]
        for mutation in mutations:
            rows = fixture(); mutation(rows[7]['non_object_references_verified'][0]); reseal(rows)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): apply(rows)

    def test_should_retain_reviewed_absent_kinds_when_every_page_and_source_inventory_were_checked(self):
        rows = fixture()
        rows[4]['complete_visual_inventory'].update(figure=0, table=0)
        rows[7]['complete_inventory_verified'].update(figure=0, table=0)
        rows[7]['negative_kind_review'] = {'pages_viewed': [1], 'figure': 'Complete original page has no figure', 'table': 'Complete original page has no table'}
        reseal(rows)
        self.assertEqual(apply(rows)['manual_object_overlay']['reviewed_absent_kinds'], ['figure', 'table'])
        rows[7]['negative_kind_review']['pages_viewed'] = []
        with self.assertRaisesRegex(ValueError, 'absent kind lacks complete'): apply(rows)

    def test_should_reject_unverified_images_when_manual_hash_claims_do_not_match_the_file_boundary(self):
        rows = fixture(); rows[8]['page-1.png'] = 'f' * 64
        with self.assertRaisesRegex(ValueError, 'actual verified bytes'): apply(rows)


if __name__ == '__main__': unittest.main()
