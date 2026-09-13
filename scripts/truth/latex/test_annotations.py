"""Offline manual-evidence validation; synthetic boxes never become K1 labels."""
from copy import deepcopy
import unittest
import tempfile
from pathlib import Path
from builder import attach_manual_regions

from annotations import apply_regions, canonical, document
from archive import sha256


def fixture(unsupported=None):
    text = 'A complete independently annotated figure caption.'
    index = {'index': {'text': text, 'tokens': [], 'pages': [{'number': 1, 'start': 0, 'end': len(text), 'width': 612, 'height': 792}]}}
    source = {'id': 'object:a', 'kind': 'figure', 'caption': text, 'text': text, 'text_sha256': sha256(text.encode()), 'labels': ['a'], 'source_members': [{'path': 'main.tex', 'start': 0, 'end': 100}], 'unsupported_commands': unsupported or {}}
    inventory = {'objects': [source], 'coverage': {'unsupported_source_semantics': {}, 'unsupported_object_environments': {}}}
    candidate = {'arxiv_id': '2001.00001', 'pdf_sha256': '1' * 64, 'source_sha256': '2' * 64,
                 'source_inventory': inventory, 'source_inventory_sha256': sha256(canonical(inventory)), 'index': {'path': 'papers/actual/reading-index.json', 'sha256': sha256(canonical(index))},
                 'objects': [], 'excluded_objects': [{'id': 'object:a', 'reason': 'automatic exclusion remains'}]}
    counts = {'figures': 1, 'tables': 0}
    annotation = {'schema_version': 1, 'arxiv_id': candidate['arxiv_id'], 'pdf_sha256': candidate['pdf_sha256'], 'annotator': 'annotator', 'pages_inspected': [1], 'complete_printed_inventory': counts,
                  'coordinate_system': {'page_numbers': 'one-based', 'origin': 'top-left of unrotated PDF CropBox', 'unit': 'PDF point', 'page_width': 612, 'page_height': 792}, 'page_render_sha256': {'page-1.png': '3' * 64},
                  'regions': [{'kind': 'figure', 'printed_label': '1', 'page': 1, 'pdf_points': {'x': 10, 'y': 10, 'width': 200, 'height': 100}}]}
    region_review = {'schema_version': 1, 'arxiv_id': candidate['arxiv_id'], 'reviewer': 'independent', 'verdict': 'clear_for_independent_region_candidate', 'findings': [], 'annotation_sha256': sha256(canonical(annotation)), 'pages_visually_inspected': [1], 'inventory': {**counts, 'complete': True}}
    hashes = {'source_candidate_file_sha256': sha256(canonical(candidate)), 'source_inventory_sha256': candidate['source_inventory_sha256'], 'region_annotation_sha256': sha256(canonical(annotation)), 'independent_region_review_sha256': sha256(canonical(region_review)), 'source_sha256': candidate['source_sha256'], 'pdf_sha256': candidate['pdf_sha256']}
    binding = {'source_object_id': 'object:a', 'kind': 'figure', 'printed_label': '1', 'page': 1, 'source_caption_sha256': source['text_sha256']}
    association = {**hashes, 'schema_version': 1, 'arxiv_id': candidate['arxiv_id'], 'annotator': 'annotator', 'index': candidate['index'], 'pages_visually_inspected': [1], 'complete_figure_table_inventory': counts, 'associations': [binding]}
    review = {'schema_version': 1, 'arxiv_id': candidate['arxiv_id'], 'reviewer': 'independent', 'verdict': 'clear_source_to_visual_associations', 'findings': [], 'reviewed_association_sha256': sha256(canonical(association)), 'verified_hashes': hashes, 'index_sha256_verified': candidate['index']['sha256'], 'complete_inventory': counts,
              'associations': [{**binding, 'review_verdict': 'confirmed', 'source_members': source['source_members']}]}
    return [candidate, index, annotation, region_review, association, review]


def reseal(rows):
    candidate, index, annotation, region_review, association, review = rows
    candidate['source_inventory_sha256'] = sha256(canonical(candidate['source_inventory']))
    candidate['index']['sha256'] = sha256(canonical(index))
    region_review['annotation_sha256'] = sha256(canonical(annotation))
    hashes = {'source_candidate_file_sha256': sha256(canonical(candidate)), 'source_inventory_sha256': candidate['source_inventory_sha256'], 'region_annotation_sha256': sha256(canonical(annotation)), 'independent_region_review_sha256': sha256(canonical(region_review)), 'source_sha256': candidate['source_sha256'], 'pdf_sha256': candidate['pdf_sha256']}
    association.update(hashes)
    review['reviewed_association_sha256'] = sha256(canonical(association))
    review['verified_hashes'] = hashes
    review['index_sha256_verified'] = candidate['index']['sha256']
    return rows


def apply(rows):
    return apply_regions(*(canonical(row) for row in rows))


class AnnotationTests(unittest.TestCase):
    def test_should_preserve_automatic_exclusions_when_independent_manual_evidence_is_attached(self):
        rows = fixture(); before = deepcopy(rows[0]); result = apply(rows)
        self.assertEqual(rows[0], before)
        self.assertEqual({key: result[key] for key in before}, before)
        overlay = result['manual_figure_table_overlay']
        self.assertFalse(overlay['final_k1_publication'])
        self.assertTrue(overlay['metric_eligibility']['O2'])
        self.assertEqual(overlay['objects'][0]['region'][0]['rect']['x_max'], 210)

    def test_should_reject_substituted_artifacts_when_reviews_bind_other_bytes(self):
        for index, key, value in [(0, 'pdf_sha256', 'f' * 64), (2, 'pdf_sha256', 'f' * 64), (3, 'reviewer', 'someone else'), (4, 'annotator', 'someone else')]:
            rows = fixture(); rows[index][key] = value
            with self.subTest(index=index), self.assertRaises(ValueError): apply(rows)

    def test_should_reject_incomplete_or_unreviewed_evidence_when_a_manual_overlay_is_requested(self):
        for key, value in [('findings', ['open issue']), ('verdict', 'pending'), ('reviewer', 'annotator'), ('associations', [])]:
            rows = fixture(); rows[5][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError): apply(rows)

    def test_should_reject_unknown_rendering_when_only_script_binding_has_manual_support(self):
        for unknown in ({'unverified_script_binding': 1}, {'emphclaim': 1}):
            with self.subTest(unknown=unknown), self.assertRaises(ValueError): apply(fixture(unknown))

    def test_should_require_supported_coordinates_when_independently_signed_evidence_is_inconsistent(self):
        mutations = [lambda r: r[2]['regions'][0]['pdf_points'].update(width=-1),
                     lambda r: r[2]['coordinate_system'].update(origin='bottom-left'),
                     lambda r: r[2]['page_render_sha256'].clear(),
                     lambda r: r[2]['regions'].append(deepcopy(r[2]['regions'][0])),
                     lambda r: r[0]['source_inventory']['coverage']['unsupported_source_semantics'].update(hidden=1)]
        for mutation in mutations:
            rows = fixture(); mutation(rows); reseal(rows)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): apply(rows)

    def test_should_attach_separate_manual_math_evidence_when_script_binding_was_visually_verified(self):
        rows = fixture({'unverified_script_binding': 1})
        rows[5]['manual_math_caption_review'] = {'source_object_id': 'object:a', 'verdict': 'confirmed_separately_from_automatic_text_alignment', 'automatic_script_binding_withholding_retained': True, 'automatic_confidence_changed': False, 'page': 1, 'printed_label': '1', 'source_member': rows[0]['source_inventory']['objects'][0]['source_members'][0]}
        result = apply(rows)
        self.assertEqual(result['source_inventory']['objects'][0]['unsupported_commands'], {'unverified_script_binding': 1})
        self.assertFalse(result['manual_figure_table_overlay']['objects'][0]['automatic_alignment_changed'])

    def test_should_rehash_actual_render_bytes_when_a_reviewed_bundle_is_loaded(self):
        rows = fixture()
        rows[0]['paper_id'] = 'actual-mapped-id'
        rows[0]['pdf_sha256'] = sha256(b'pdf fixture')
        rows[0]['source_sha256'] = sha256(b'source fixture')
        rows[2]['pdf_sha256'] = rows[0]['pdf_sha256']
        rows[2]['page_render_sha256']['page-1.png'] = sha256(b'render fixture')
        reseal(rows)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'papers/actual').mkdir(parents=True)
            (root/'paper.pdf').write_bytes(b'pdf fixture')
            (root/'paper.src').write_bytes(b'source fixture')
            (root/'page-1.png').write_bytes(b'render fixture')
            (root/rows[0]['index']['path']).write_bytes(canonical(rows[1]))
            names = ('regions-root-v1.json', 'review-independent-v1.json', 'source-associations-root-v1.json', 'source-associations-independent-review-v1.json')
            for name, value in zip(names, rows[2:]): (root/name).write_bytes(canonical(value))
            paper = {'arxiv_id': rows[0]['arxiv_id'], 'pdf': {'path': 'paper.pdf', 'sha256': rows[0]['pdf_sha256'], 'bytes': 11}, 'source': {'path': 'paper.src', 'sha256': rows[0]['source_sha256'], 'bytes': 14}}
            mapped = {'paper_id': 'actual-mapped-id', 'index': rows[0]['index']}
            self.assertTrue(attach_manual_regions(canonical(rows[0]), paper, mapped, root, root, root)['manual_figure_table_overlay'])
            (root/'page-1.png').write_bytes(b'replaced render')
            with self.assertRaisesRegex(ValueError, 'page render differs'):
                attach_manual_regions(canonical(rows[0]), paper, mapped, root, root, root)

    def test_should_reject_ambiguous_json_when_receipts_repeat_fields_or_nonfinite_numbers(self):
        for raw in (b'{"a":1,"a":2}', b'{"a":NaN}', b'[]'):
            with self.subTest(raw=raw), self.assertRaises(ValueError): document(raw)


if __name__ == '__main__': unittest.main()
