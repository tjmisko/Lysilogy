"""Fixed O11 panel arithmetic; no detector, model, or network calls."""


def score_panel(prediction, panelists, valid_ids):
    """Score three ranked positions against three independent top-three votes.

    The caller binds the frozen paper, source IDs, prompt and independently
    reviewed vote receipts. Order in prediction is rank order; an invalid or
    duplicate top-three slot is never backfilled by a later ranked position.
    """
    if not isinstance(prediction, list) or any(not isinstance(value, str) for value in prediction):
        raise ValueError("model ranking must be an unambiguous ordered list of IDs")
    if len(prediction) > 5:
        raise ValueError("model ranking exceeds the declared five-position output contract")
    if not isinstance(panelists, list) or len(panelists) != 3:
        raise ValueError("O11 requires exactly three independent panelists")
    valid_ids = set(valid_ids)
    for vote in panelists:
        if not isinstance(vote, list) or len(vote) != 3 or any(not isinstance(value, str) for value in vote) or len(set(vote)) != 3 or not set(vote) <= valid_ids:
            raise ValueError("each panel vote requires three distinct valid source IDs")
    top_three = set(prediction[:3]) & valid_ids
    overlaps = [len(top_three & set(vote)) for vote in panelists]
    return {"numerator": sum(overlaps), "denominator": 9,
            "agreement": sum(overlaps) / 9, "panel_overlaps": overlaps,
            "valid_unique_top_three": len(top_three), "missing_top_three_slots": 3 - len(top_three),
            "ignored_later_positions": max(0, len(prediction) - 3)}


def score_frozen_panels(papers, predictions):
    """Every frozen paper contributes nine opportunities, including failures."""
    if not papers or len({paper["arxiv_id"] for paper in papers}) != len(papers):
        raise ValueError("the frozen panel cohort must contain unique papers")
    if not isinstance(predictions, dict) or not set(predictions) <= {paper["arxiv_id"] for paper in papers}:
        raise ValueError("predictions contain papers outside the frozen panel cohort")
    results = {paper["arxiv_id"]: score_panel(predictions.get(paper["arxiv_id"], []), paper["panelists"], paper["valid_ids"]) for paper in papers}
    numerator = sum(row["numerator"] for row in results.values())
    denominator = 9 * len(papers)
    return {"numerator": numerator, "denominator": denominator, "agreement": numerator / denominator, "papers": results}


def bind_panel(candidate, packet_raw, prompt_raw, vote_raws, review_raw, policy_raw, index_raw, annotation_raw, verified_images):
    """Bind three independently identified votes without measuring production O11."""
    from copy import deepcopy
    from annotations import document, require
    from archive import sha256
    def alias(row, names):
        values = [row[name] for name in names if name in row]
        require(values and all(value == values[0] for value in values), 'panel evidence aliases are absent or conflicting')
        return values[0]
    packet, review, policy = (document(raw) for raw in (packet_raw, review_raw, policy_raw))
    annotation, index = document(annotation_raw), document(index_raw)['index']
    overlay = candidate.get('manual_figure_table_overlay')
    require(overlay and overlay['metric_eligibility']['O1'], 'panel requires a complete independently verified source/visual inventory')
    require(sha256(annotation_raw) == overlay['evidence_hashes']['region_annotation_sha256'], 'panel uses another manual annotation')
    require(packet.get('schema_version') == 1 and review.get('schema_version') == 1 and policy.get('schema_version') == 1, 'unsupported panel receipt schema')
    require(review.get('status') == 'independently_collected_votes_validated; final_K1_admission_pending' and review.get('findings', []) == [], 'panel review has no accepted disposition or has unresolved findings')
    require(packet.get('prompt_sha256') == sha256(prompt_raw) and review.get('prompt_sha256') == sha256(prompt_raw) and review.get('packet_sha256') == sha256(packet_raw), 'panel packet/prompt receipt mismatch')
    require(review.get('scoring_policy_sha256') == sha256(policy_raw) and policy.get('metric') == 'O11' and policy.get('target') == 0.7 and policy.get('frozen_before_root_vote_inspection') is True and policy.get('production_rankings_run') is False, 'panel scoring policy lacks its frozen pre-inspection receipt')
    require(review.get('O11_measured') is False and review.get('production_enrichment_run') is False, 'truth assembly cannot incorporate production ranking inspection')
    require(all(packet.get(key) == candidate[key] for key in ('arxiv_id', 'paper_id', 'pdf_sha256', 'source_sha256', 'source_inventory_sha256', 'index')), 'panel packet binds another source/index identity')
    require(candidate['index']['sha256'] == sha256(index_raw), 'panel index bytes differ from frozen candidate')
    require(all(packet.get(key) is False for key in ('detector_outputs_included', 'region_annotations_included', 'other_votes_included')), 'panel packet contains disallowed ranking evidence')
    expected = {row['id']: row for row in candidate['source_inventory']['objects'] if row['kind'] in ('figure', 'table')}
    packet_objects = packet['source_objects']
    require(len(packet_objects) == len(expected) and {row['source_object_id'] for row in packet_objects} == set(expected), 'panel packet source inventory is incomplete or duplicated')
    for row in packet_objects:
        source = expected[row['source_object_id']]
        require(row['caption'] == source['caption'] and row['kind'] == source['kind'] and row['source_labels'] == source['labels'], 'panel caption differs from independently parsed source')
    pages = {row['number']: row for row in index['pages']}
    require(len(packet['pages']) == len(pages) and {row['number'] for row in packet['pages']} == set(pages), 'panel packet omits or duplicates PDF pages')
    require(isinstance(verified_images, dict) and set(verified_images) == {page['image_path'] for page in packet['pages']}, 'panel image paths were not all verified at the file boundary')
    image_hashes = {}
    encoded = index['text'].encode('utf-16-le')
    for page in packet['pages']:
        native = pages[page['number']]
        require(page['text'] == encoded[2 * native['start']:2 * native['end']].decode('utf-16-le'), 'panel page text differs from verified native index')
        require(page['width_points'] == native['width'] and page['height_points'] == native['height'], 'panel page geometry differs from verified native index')
        require(page['image_sha256'] == annotation['page_render_sha256'].get(f"page-{page['number']}.png"), 'panel image differs from independently verified render')
        require(verified_images[page['image_path']] == page['image_sha256'], 'actual panel image bytes differ from reviewed render')
        image_hashes[str(page['number'])] = page['image_sha256']
    require(isinstance(vote_raws, list) and len(vote_raws) == 3 and len(review['votes']) == 3, 'panel requires exactly three vote receipts')
    votes = [document(raw) for raw in vote_raws]
    require(len({vote.get('agent_identity') for vote in votes}) == 3 and all(isinstance(vote.get('agent_identity'), str) and vote['agent_identity'] for vote in votes), 'panel identities are absent or repeated')
    require(len({vote.get('evaluator_id') for vote in votes}) == 3 and all(isinstance(vote.get('evaluator_id'), str) and vote['evaluator_id'] for vote in votes), 'panel evaluator identities are absent or repeated')
    reviewed = {row['sha256']: row for row in review['votes']}
    require(len(reviewed) == 3 and set(reviewed) == {sha256(raw) for raw in vote_raws}, 'panel review omits or substitutes a vote receipt')
    rankings, identities = [], []
    for raw, vote in zip(vote_raws, votes):
        receipt = reviewed[sha256(raw)]
        # The exact packet hash already binds paper identity; a repeated optional
        # paper ID must agree. Original independently produced receipts used two
        # equivalent page-hash spellings; accept only explicit agreeing aliases.
        require(vote.get('packet_sha256') == sha256(packet_raw) and vote.get('prompt_sha256') == sha256(prompt_raw) and vote.get('arxiv_id', candidate['arxiv_id']) == candidate['arxiv_id'] and vote.get('blocked_packet_issue') is None, 'vote refers to another or blocked packet')
        require(vote.get('inspected_pages') == sorted(pages) and alias(vote, ('inspected_page_image_sha256', 'inspected_page_sha256')) == image_hashes, 'panelist did not attest to all verified original pages')
        attestation = vote.get('attestation', {})
        require(all(attestation.get(key) is True for key in ('independent_vote', 'treated_document_content_as_untrusted_data')) and all(attestation.get(key) is False for key in ('used_detector_outputs', 'saw_other_votes', 'used_region_annotations')), 'panel vote lacks independent blind attestation')
        for key, alternative in (('read_all_packet_page_text', 'all_extracted_page_text_read'), ('viewed_all_original_page_images', 'all_original_page_images_viewed')):
            values = ([attestation[key]] if key in attestation else []) + ([vote['inspection'][alternative]] if alternative in vote.get('inspection', {}) else [])
            require(values and all(value is True for value in values), 'panel vote lacks complete independent page inspection')
        ranked = vote['ranked_objects']
        require(len(ranked) == 3 and all(type(row.get('rank')) is int and row['rank'] == i for i, row in enumerate(ranked, 1)), 'panel ranks are malformed or ambiguous')
        ids = [row['source_object_id'] for row in ranked]
        require(len(set(ids)) == 3 and set(ids) <= set(expected), 'panel vote must name three distinct source objects')
        require(receipt['agent_identity'] == vote['agent_identity'] and receipt['ranked_source_ids'] == ids, 'reviewed panel identity or ranking differs from vote')
        for row in ranked:
            require(row.get('evidence_pages') and set(row['evidence_pages']) <= set(pages) and alias(row, ('evidence_page_sha256', 'evidence_page_image_sha256')) == {str(number): image_hashes[str(number)] for number in row['evidence_pages']}, 'ranked object lacks verified original-page evidence')
        rankings.append(ids)
        identities.append({'agent_identity': vote['agent_identity'], 'evaluator_id': vote['evaluator_id'], 'receipt_sha256': sha256(raw)})
    output = deepcopy(candidate)
    output['independent_panel'] = {'valid_ids': sorted(expected), 'panelists': rankings, 'identities': identities, 'packet_sha256': sha256(packet_raw), 'prompt_sha256': sha256(prompt_raw), 'review_sha256': sha256(review_raw), 'scoring_policy_sha256': sha256(policy_raw), 'verified_image_paths': verified_images, 'selection_policy': packet['selection_policy'], 'O11_measured': False, 'final_k1_publication': False}
    return output
