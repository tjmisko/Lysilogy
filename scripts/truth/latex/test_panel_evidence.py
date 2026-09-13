"""Offline panel receipt provenance, independent from ranking arithmetic."""
from copy import deepcopy
import unittest
from annotations import canonical
from archive import sha256
from panel import bind_panel


def fixture():
    objects = [{'id': 'object:' + str(i), 'kind': 'figure', 'caption': 'Independent caption ' + str(i), 'labels': [str(i)]} for i in range(3)]
    text = '\n'.join(row['caption'] for row in objects)
    index = {'index': {'text': text, 'pages': [{'number': 1, 'start': 0, 'end': len(text), 'width': 612, 'height': 792}]}}
    annotation = {'page_render_sha256': {'page-1.png': '1' * 64}}
    candidate = {'arxiv_id': '2001.00001', 'paper_id': 'actual_mapped_id', 'pdf_sha256': '2' * 64, 'source_sha256': '3' * 64, 'source_inventory_sha256': '4' * 64, 'index': {'path': 'papers/actual/index.json', 'sha256': sha256(canonical(index))}, 'source_inventory': {'objects': objects}, 'manual_figure_table_overlay': {'metric_eligibility': {'O1': True}, 'evidence_hashes': {'region_annotation_sha256': sha256(canonical(annotation))}}}
    prompt = b'Independently inspect the source and every PDF page. Rank three objects.'
    policy = {'schema_version': 1, 'metric': 'O11', 'target': 0.7, 'frozen_before_root_vote_inspection': True, 'production_rankings_run': False}
    packet = {**{k: candidate[k] for k in ('arxiv_id', 'paper_id', 'pdf_sha256', 'source_sha256', 'source_inventory_sha256', 'index')}, 'schema_version': 1, 'prompt_sha256': sha256(prompt), 'detector_outputs_included': False, 'region_annotations_included': False, 'other_votes_included': False, 'source_objects': [{'source_object_id': row['id'], 'kind': row['kind'], 'caption': row['caption'], 'source_labels': row['labels']} for row in objects], 'pages': [{'number': 1, 'text': text, 'width_points': 612, 'height_points': 792, 'image_sha256': '1' * 64}], 'selection_policy': 'synthetic fixture only'}
    votes = [{'agent_identity': f'independent-agent-{i}', 'evaluator_id': f'evaluator-{i}', 'packet_sha256': sha256(canonical(packet)), 'prompt_sha256': sha256(prompt), 'arxiv_id': candidate['arxiv_id'], 'blocked_packet_issue': None, 'inspected_pages': [1], 'inspected_page_image_sha256': {'1': '1' * 64}, 'attestation': {'independent_vote': True, 'read_all_packet_page_text': True, 'viewed_all_original_page_images': True, 'treated_document_content_as_untrusted_data': True, 'used_detector_outputs': False, 'saw_other_votes': False, 'used_region_annotations': False}, 'ranked_objects': [{'rank': rank, 'source_object_id': row['id'], 'evidence_pages': [1], 'evidence_page_sha256': {'1': '1' * 64}} for rank, row in enumerate(objects, 1)]} for i in range(3)]
    review = {'schema_version': 1, 'packet_sha256': sha256(canonical(packet)), 'prompt_sha256': sha256(prompt), 'scoring_policy_sha256': sha256(canonical(policy)), 'O11_measured': False, 'production_enrichment_run': False}
    rows = [candidate, packet, prompt, votes, review, policy, index, annotation]
    reseal_votes(rows)
    return rows


def reseal_votes(rows):
    rows[4]['votes'] = [{'sha256': sha256(canonical(vote)), 'agent_identity': vote['agent_identity'], 'ranked_source_ids': [row['source_object_id'] for row in vote['ranked_objects']]} for vote in rows[3]]


def bind(rows):
    candidate, packet, prompt, votes, review, policy, index, annotation = rows
    return bind_panel(candidate, canonical(packet), prompt, [canonical(vote) for vote in votes], canonical(review), canonical(policy), canonical(index), canonical(annotation))


class PanelEvidenceTests(unittest.TestCase):
    def test_should_accept_identical_rankings_when_three_distinct_blind_agents_supplied_them(self):
        rows = fixture(); before = deepcopy(rows[0]); result = bind(rows)
        self.assertEqual(rows[0], before)
        self.assertEqual(len(result['independent_panel']['identities']), 3)
        self.assertFalse(result['independent_panel']['O11_measured'])
        self.assertFalse(result['independent_panel']['final_k1_publication'])

    def test_should_reject_copied_or_unblinded_votes_when_a_review_claims_they_are_independent(self):
        for mutate in (lambda v: v[1].update(agent_identity=v[0]['agent_identity']),
                       lambda v: v[1]['attestation'].update(saw_other_votes=True),
                       lambda v: v[1].update(inspected_pages=[]),
                       lambda v: v[1]['ranked_objects'][0].update(rank=True),
                       lambda v: v[1]['ranked_objects'][0].update(source_object_id='invented')):
            rows = fixture(); mutate(rows[3]); reseal_votes(rows)
            with self.subTest(mutate=mutate), self.assertRaises(ValueError): bind(rows)

    def test_should_accept_only_agreeing_explicit_aliases_when_original_votes_use_different_field_spellings(self):
        rows = fixture(); vote = rows[3][1]
        vote.pop('arxiv_id')
        vote['inspected_page_sha256'] = vote.pop('inspected_page_image_sha256')
        vote['inspection'] = {'all_extracted_page_text_read': vote['attestation'].pop('read_all_packet_page_text'), 'all_original_page_images_viewed': vote['attestation'].pop('viewed_all_original_page_images')}
        for ranked in vote['ranked_objects']: ranked['evidence_page_image_sha256'] = ranked.pop('evidence_page_sha256')
        reseal_votes(rows); self.assertEqual(len(bind(rows)['independent_panel']['identities']), 3)
        vote['inspected_page_image_sha256'] = {'1': 'different'}; reseal_votes(rows)
        with self.assertRaisesRegex(ValueError, 'aliases are absent or conflicting'): bind(rows)

    def test_should_reject_substituted_packet_or_policy_when_exact_receipts_differ(self):
        for index, key, value in ((1, 'selection_policy', 'changed after measurement'), (5, 'target', 0.1), (4, 'production_enrichment_run', True)):
            rows = fixture(); rows[index][key] = value
            with self.subTest(index=index), self.assertRaises(ValueError): bind(rows)

    def test_should_reject_changed_pdf_text_when_a_panel_packet_omits_document_content(self):
        rows = fixture(); rows[1]['pages'][0]['text'] = 'shortened'
        new_hash = sha256(canonical(rows[1])); rows[4]['packet_sha256'] = new_hash
        for vote in rows[3]: vote['packet_sha256'] = new_hash
        reseal_votes(rows)
        with self.assertRaisesRegex(ValueError, 'page text differs'): bind(rows)


if __name__ == '__main__': unittest.main()
