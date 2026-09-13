"""Bind separately reviewed manual math/object membership without flattening semantics."""
from collections import Counter
from copy import deepcopy
import math

from annotations import canonical, document, require
from archive import read_archive, sha256

KINDS = {'equation', 'statement', 'proof', 'algorithm'}


def member_identity(rows):
    return [{'path': row['path'], 'start': row['start'], 'end': row['end']} for row in rows]


def checked_span(row, text):
    start, end = row['start'], row['end']
    encoded = text.encode('utf-16-le')
    require(type(start) is int and type(end) is int and 0 <= start < end <= len(encoded) // 2, 'manual span is outside the UTF-16 index')
    try:
        actual = encoded[2 * start:2 * end].decode('utf-16-le')
    except UnicodeDecodeError as error:
        raise ValueError('manual span splits a UTF-16 character') from error
    require(row.get('text_utf8_sha256') == sha256(actual.encode()), 'manual member text differs from its native hash')
    require('text' not in row or row['text'] == actual, 'manual member excerpt differs from native text')
    return {'start': start, 'end': end, 'native_text_sha256': sha256(actual.encode())}


def checked_members(rows, files, expected=None):
    if expected is not None:
        require(member_identity(rows) == member_identity(expected), 'manual source membership differs from complete parsed object')
    output = []
    for row in rows:
        start, end = row['start'], row['end']
        require(row['path'] in files and type(start) is int and type(end) is int and 0 <= start < end <= len(files[row['path']]), 'manual source member is outside the verified archive')
        value = files[row['path']][start:end]
        if 'utf8_sha256' in row:
            require(row['utf8_sha256'] == sha256(value.encode()), 'manual source slice hash mismatch')
        output.append({'path': row['path'], 'start': start, 'end': end, 'source_slice_sha256': sha256(value.encode())})
    require(output, 'manual object has no source members')
    return output


def checked_regions(rows, pages):
    output = []
    for row in rows:
        require(type(row['page']) is int and row['page'] in pages, 'manual region names an absent PDF page')
        page = pages[row['page']]
        rect = row['pdf_points']
        coordinates = [rect[key] for key in ('x_min', 'y_min', 'x_max', 'y_max')]
        require(all(type(value) in (int, float) and math.isfinite(value) for value in coordinates), 'manual region is nonfinite')
        left, top, right, bottom = coordinates
        require(0 <= left < right <= page['width'] and 0 <= top < bottom <= page['height'], 'manual region is outside its PDF page')
        pixels = row['pixels_96dpi']
        require(len(pixels) == 4 and all(type(value) in (int, float) and math.isfinite(value) for value in pixels) and all(abs(point - pixel * .75) < 1e-8 for point, pixel in zip(coordinates, pixels)), 'manual region pixel-to-point conversion differs')
        output.append({'page': row['page'], 'rect': rect})
    require(output, 'manual object has no visual region')
    return output


def covered_characters(spans, text):
    """Compare exact authored membership, allowing only whitespace-only splits."""
    encoded = text.encode('utf-16-le')
    covered = set()
    for span in spans:
        offset = span['start']
        for character in encoded[2 * offset:2 * span['end']].decode('utf-16-le'):
            width = len(character.encode('utf-16-le')) // 2
            if not character.isspace():
                covered.update(range(offset, offset + width))
            offset += width
    return covered


def checked_root_span(row, text):
    return checked_span({**row, 'text_utf8_sha256': row['native_text_sha256']}, text)


def apply_object_overlay(candidate_raw, index_raw, source_raw, packet_raw, root_raw, independent_raw,
                         independent_receipt_raw, comparison_raw, verified_images):
    """Retain immutable automatic results; add only completely reviewed metric kinds.

    The file boundary must additionally verify actual PDF bytes and each supplied
    page path, just as the figure/panel assembly does. This function verifies the
    deposited source bytes, complete source inventory and exact native members.
    """
    candidate, wrapper, packet, root, independent, receipt, comparison = (document(raw) for raw in (candidate_raw, index_raw, packet_raw, root_raw, independent_raw, independent_receipt_raw, comparison_raw))
    require(comparison.get('schema_version') == 1 and comparison.get('verdict') == 'clear_complete_object_overlay' and comparison.get('findings') == [], 'manual object comparison is not independently cleared')
    for key, raw in (('root_annotation_sha256', root_raw), ('independent_annotation_sha256', independent_raw), ('independent_receipt_sha256', independent_receipt_raw)):
        require(comparison.get(key) == sha256(raw), 'manual comparison binds different annotation bytes')
    require(receipt.get('schema_version') == 1 and receipt['annotation']['sha256'] == sha256(independent_raw) and receipt['annotation']['bytes'] == len(independent_raw), 'independent execution receipt binds another annotation')
    require(root.get('annotator') and independent.get('annotator') and root['annotator'] != independent['annotator'], 'manual object annotators are absent or repeated')
    require(root.get('attestation', {}).get('detector_outputs_read') is False and root['attestation'].get('other_annotator_labels_read') is False, 'root annotation is not independently blind')
    require(independent['complete_inventory'].get('paper_complete_for_scope') is True, 'independent object inventory is incomplete')
    for key in ('arxiv_id', 'paper_id', 'pdf_sha256', 'source_sha256', 'index', 'source_inventory_sha256'):
        require(root.get(key) == candidate[key], 'root annotation differs from frozen candidate identity')
    require(root.get('candidate_sha256') == sha256(candidate_raw) and root.get('packet_sha256') == sha256(packet_raw), 'root annotation binds another packet/candidate')
    require(packet.get('arxiv_id') == candidate['arxiv_id'] and packet.get('paper_id') == candidate['paper_id'] and packet.get('candidate_sha256') == sha256(candidate_raw) and packet.get('inventory_sha256') == candidate['source_inventory_sha256'] and packet.get('index') == candidate['index'], 'manual packet binds another source/index identity')
    require(packet['pdf']['sha256'] == candidate['pdf_sha256'] and packet['source']['sha256'] == candidate['source_sha256'], 'manual packet uses another source/PDF')
    require(sha256(source_raw) == candidate['source_sha256'] and sha256(index_raw) == candidate['index']['sha256'], 'manual source/index bytes changed')
    require(sha256(canonical(candidate['source_inventory'])) == candidate['source_inventory_sha256'], 'manual candidate source inventory hash mismatch')
    inputs = independent['inputs']
    require(inputs['candidate_sha256_from_packet'] == sha256(candidate_raw) and inputs['inventory_sha256_from_packet'] == candidate['source_inventory_sha256'], 'independent annotator used another candidate inventory')
    require(inputs['packet']['sha256'] == sha256(packet_raw) and inputs['pdf']['sha256'] == candidate['pdf_sha256'] and inputs['source']['sha256'] == candidate['source_sha256'] and inputs['reading_index']['sha256'] == sha256(index_raw), 'independent annotation used different source/PDF/index artifacts')
    files, members = read_archive(source_raw)
    require(inputs['source_members'] == members, 'independent source archive member inventory differs')
    index = wrapper['index']; pages = {row['number']: row for row in index['pages']}
    require(root['viewed_all_pages'] == sorted(pages), 'root did not inspect the complete PDF')
    images = inputs['original_images']
    require(len(images) == len(pages) and len({row['path'] for row in images}) == len(pages), 'independent page image inventory is incomplete or duplicated')
    all_images = images + packet['images'] + root['page_images'] + receipt['detail_crops']
    if root.get('supplemental_image'):
        all_images.append(root['supplemental_image'])
    require(set(verified_images) == {row['path'] for row in all_images} and all(verified_images[row['path']] == row['sha256'] for row in all_images), 'manual original/detail images differ from actual verified bytes')
    require(all(row['viewed_full_original'] is True for row in images) and all(row['viewed'] is True for row in receipt['detail_crops']), 'independent annotation lacks complete original/detail image inspection')
    require(len(packet['images']) == len(pages) and len(root['page_images']) == len(pages) and packet['images'] == root['page_images'], 'root and packet original image inventories differ')
    require(len(packet['page_text']) == len(pages) and {row['page'] for row in packet['page_text']} == set(pages), 'manual packet page text is incomplete or duplicated')
    encoded = index['text'].encode('utf-16-le')
    for row in packet['page_text']:
        page = pages[row['page']]
        require(row['text'] == encoded[page['start'] * 2:page['end'] * 2].decode('utf-16-le'), 'manual packet page text differs from native index')
    parsed = candidate['source_inventory']
    require(packet['objects'] == parsed['objects'] and packet['links'] == parsed['links'], 'manual packet changed the complete source object/link inventory')
    sources = {row['id']: row for row in parsed['objects'] if row['kind'] in KINDS}
    objects = independent['objects']
    require(len(objects) == len(sources) and {row['id'] for row in objects} == set(sources) and set(comparison['objects_verified']) == set(sources), 'manual object inventory is incomplete or duplicated')
    counts = dict(Counter(row['kind'] for row in objects))
    require(comparison.get('complete_inventory_verified') == root['complete_visual_inventory'], 'comparison complete inventory contradicts root review')
    require(independent['complete_inventory']['counts'] == counts and all(root['complete_visual_inventory'].get(kind, 0) == counts.get(kind, 0) for kind in KINDS), 'independent complete kind counts disagree')
    output_objects = []
    for row in objects:
        source = sources[row['id']]
        require(row['kind'] == source['kind'] and row['labels'] == source['labels'], 'manual object kind/labels disagree with source')
        require(row['source_vs_pdf_math']['judgment'] == 'visually agrees', 'manual source/PDF expression identity is not verified')
        spans = [checked_span(span, index['text']) for span in row['direct_index_spans']]
        require(spans and all(left['end'] <= right['start'] for left, right in zip(spans, spans[1:])), 'manual object spans are empty, unordered or overlapping')
        regions = checked_regions(row['visual_body_boxes'], pages)
        require(all(any(pages[region['page']]['start'] <= span['start'] < span['end'] <= pages[region['page']]['end'] for region in regions) for span in spans), 'manual text member lies outside its object pages')
        output_objects.append({'id': row['id'], 'kind': row['kind'], 'labels': row['labels'], 'printed_heading': row['printed_heading'], 'spans': spans, 'region': regions, 'source_members': checked_members(row['source_members'], files, source['source_members']), 'source_parent_object': row['source_parent_object'], 'source_child_objects': row['source_child_objects'], 'reading_index_fidelity': row['reading_index_fidelity'], 'semantic_quote_truth': False, 'automatic_alignment_changed': False})
    # Parent/child structure is sourced from exact nested source environments;
    # floating placement does not turn child bodies into parent direct spans.
    by_id = {row['id']: row for row in output_objects}
    root_objects = {row['source_id']: row for row in root['objects']}
    require(len(root['objects']) == len(sources) and set(root_objects) == set(sources), 'root object inventory differs from independent source inventory')
    parents = {}
    for identifier, source in sources.items():
        span = source['source_span']
        enclosing = [other for other in sources.values() if other['source_span']['start'] < span['start'] < span['end'] < other['source_span']['end']]
        parents[identifier] = min(enclosing, key=lambda item: item['source_span']['end'] - item['source_span']['start'])['id'] if enclosing else None
    for row in output_objects:
        children = {identifier for identifier, parent in parents.items() if parent == row['id']}
        require(row['source_parent_object'] == parents[row['id']] and len(row['source_child_objects']) == len(children) and set(row['source_child_objects']) == children, 'manual source ownership omits or changes a nested object')
        require(set(root_objects[row['id']]['separately_annotated_nested_objects']) == children, 'root nested ownership differs from source structure')
        checked_members(root_objects[row['id']]['source_members'], files, sources[row['id']]['source_members'])
    geometry = {row['source_id']: row for row in comparison['comparisons']}
    require(len(comparison['comparisons']) == len(sources) and set(geometry) == set(sources), 'comparison geometry inventory is incomplete or duplicated')
    for row in output_objects:
        root_regions = [region for region in root_objects[row['id']]['regions'] if region['role'] != 'attached_footnote']
        compared = geometry[row['id']]['geometry']
        require(len(compared) == len(row['region']) == len(root_regions), 'comparison geometry count contradicts annotations')
        for recorded, actual, original in zip(compared, row['region'], root_regions):
            require(recorded['page'] == actual['page'] == original['page'] and recorded['root_rect'] == original['rect'] and recorded['independent_rect'] == actual['rect'], 'comparison geometry contradicts frozen annotation boxes')
            require(recorded.get('disposition', '').startswith('accepted;'), 'comparison geometry is not accepted')
    ancillary = []
    for row in independent['associated_content']:
        parent = row['source_parent_object']
        require(parent in sources and row['role'] == 'footnote', 'unsupported manual associated-content relationship')
        source_members = checked_members(row['source_members'], files)
        require(all(any(parent_member['path'] == member['path'] and parent_member['start'] <= member['start'] < member['end'] <= parent_member['end'] for parent_member in sources[parent]['source_members']) for member in source_members), 'ancillary source content is outside its parent')
        spans = [checked_span(span, index['text']) for span in row['index_spans']]
        regions = checked_regions(row['visual_body_boxes'], pages)
        require(spans and all(any(pages[region['page']]['start'] <= span['start'] < span['end'] <= pages[region['page']]['end'] for region in regions) for span in spans), 'ancillary text lies outside its source-associated pages')
        ancillary.append({'id': row['id'], 'role': row['role'], 'source_parent_object': parent, 'source_members': source_members, 'spans': spans, 'region': regions})
    owned = sorted((span['start'], span['end']) for row in output_objects + ancillary for span in row['spans'])
    require(all(left[1] <= right[0] for left, right in zip(owned, owned[1:])), 'manual direct/ancillary membership overlaps across object owners')
    for row in output_objects:
        original = [checked_root_span(span, index['text']) for span in root_objects[row['id']]['spans']]
        complete = row['spans'] + [span for content in ancillary if content['source_parent_object'] == row['id'] for span in content['spans']]
        require(covered_characters(original, index['text']) == covered_characters(complete, index['text']), 'manual direct and ancillary ownership differs from reviewed root membership')
    proof_targets = {}
    for attribution in independent['proof_attribution']:
        proof, statement = attribution['proof'], attribution['statement']
        require(proof not in proof_targets and proof in sources and sources[proof]['kind'] == 'proof' and statement in sources and sources[statement]['kind'] == 'statement', 'manual proof attribution is duplicated or has invalid endpoint roles')
        if attribution['explicit_source_ref']:
            expected = {parsed['label_targets'].get(label) for label in sources[proof]['proof_target_labels']}
            require(statement in expected, 'explicit proof attribution lacks a named source target')
            basis = 'explicit source target, independently visually reviewed'
        else:
            require(not sources[proof]['proof_target_labels'] and not sources[proof].get('proof_heading'), 'manual proximity cannot override an explicit or unresolved named proof')
            previous = [row for row in sources.values() if row['kind'] == 'statement' and row['source_span']['start'] < sources[proof]['source_span']['start']]
            require(previous and max(previous, key=lambda row: row['source_span']['start'])['id'] == statement, 'unnamed proof does not target the nearest preceding source statement')
            basis = 'nearest preceding source statement, independently visually reviewed'
        require(root_objects[proof]['proof_targets'] == [statement], 'independent proof annotations disagree')
        proof_targets[proof] = {'target': statement, 'basis': basis}
    require(set(proof_targets) == {row['id'] for row in sources.values() if row['kind'] == 'proof'}, 'manual proof attribution inventory is incomplete')
    compared_proofs = comparison['proof_attribution_verified']
    if isinstance(compared_proofs, dict):
        compared_proofs = [compared_proofs]
    require(len(compared_proofs) == len(proof_targets) and {row['proof']: row['statement'] for row in compared_proofs} == {key: value['target'] for key, value in proof_targets.items()}, 'comparison proof attribution contradicts annotations')
    for row in output_objects:
        row['proof_targets'] = [proof_targets[row['id']]['target']] if row['id'] in proof_targets else []
        row['proof_linkage'] = proof_targets.get(row['id'], {}).get('basis')
    source_refs = {number: link for number, link in enumerate(parsed['links']) if link['kind'] == 'reference'}
    root_refs = {row['source_link']: row for row in root['references']}
    require(len(root_refs) == len(root['references']) and set(root_refs) == set(source_refs), 'root reference inventory omits or duplicates source occurrences')
    reference_rows, source_numbers = [], set()
    for row in independent['object_references']:
        matched = [number for number, link in source_refs.items() if member_identity(link['source_members']) == member_identity(row['source_members'])]
        require(len(matched) == 1 and matched[0] not in source_numbers, 'independent reference membership is ambiguous or duplicated')
        number = matched[0]; link = source_refs[number]
        targets = {parsed['label_targets'].get(label) for label in link['targets']}
        require(targets == {row['target']} and row['target'] in sources, 'manual reference target differs from its source label')
        span = checked_span(row['number_occurrence'], index['text'])
        root_row = root_refs[number]
        require(root_row['target'] == row['target'] and all(root_row['span'][key] == span[key] for key in ('start', 'end', 'native_text_sha256')), 'independent printed reference positions or targets disagree')
        source_numbers.add(number)
        reference_rows.append({'source_link': number, 'target': row['target'], **span})
    require(set(comparison['object_reference_source_links_verified']) == source_numbers, 'comparison omits independently annotated object references')
    non_objects = comparison['non_object_references_verified']
    require(len(non_objects) == len(source_refs) - len(source_numbers) and {row['source_link'] for row in non_objects} == set(source_refs) - source_numbers, 'unknown reference target roles remain outside the manual comparison')
    for row in non_objects:
        number = row['source_link']; declared = root_refs[number]
        require(not any(parsed['label_targets'].get(label) in sources for label in source_refs[number]['targets']), 'object reference cannot be relabeled as a non-object role')
        require(row['source_label'] == declared['source_target_label'], 'comparison non-object source label differs')
        checked_members(row['source_members'], files, source_refs[number]['source_members'])
        checked_members(declared['source_members'], files, source_refs[number]['source_members'])
        native = checked_root_span(declared['span'], index['text'])
        require(all(row['span'][key] == native[key] for key in ('start', 'end')) and row['printed'] == declared['printed'], 'comparison non-object reference differs from root anchor')
        checked_span({**row['span'], 'text': row['printed'], 'text_utf8_sha256': native['native_text_sha256']}, index['text'])
        require(row['target'] == declared['target'] and row['target'].startswith('section:') and declared['source_target_label'] in source_refs[number]['targets'], 'manual non-object reference role lacks reviewed source identity')
    relevant_refs = [row for row in reference_rows if sources[row['target']]['kind'] in ('equation', 'statement')]
    absent_kinds = []
    negative_review = comparison.get('negative_kind_review', {})
    for kind in ('figure', 'table'):
        if comparison['complete_inventory_verified'].get(kind) == 0:
            require(negative_review.get('pages_viewed') == sorted(pages) and isinstance(negative_review.get(kind), str) and negative_review[kind].strip() and not any(row['kind'] == kind for row in parsed['objects']), 'manual absent kind lacks complete source and visual review')
            absent_kinds.append(kind)
    overlay = {'objects': output_objects, 'automatic_candidate_retained': True, 'final_k1_publication': False,
               'verified_image_paths': verified_images,
               'reviewed_absent_kinds': absent_kinds, 'associated_content': ancillary, 'references': reference_rows, 'non_object_references': non_objects,
               'reference_coverage': {'source_occurrences': len(source_refs), 'object_occurrences': len(reference_rows), 'O4_occurrences': len(relevant_refs)},
               'metric_eligibility': {'O3': True, 'O4': True, 'O5': True, 'O6': True, 'O7': True},
               'evidence_hashes': {'candidate_sha256': sha256(candidate_raw), 'source_inventory_sha256': candidate['source_inventory_sha256'], 'packet_sha256': sha256(packet_raw), 'root_annotation_sha256': sha256(root_raw), 'independent_annotation_sha256': sha256(independent_raw), 'independent_receipt_sha256': sha256(independent_receipt_raw), 'comparison_sha256': sha256(comparison_raw)}}
    result = deepcopy(candidate); result['manual_object_overlay'] = overlay
    return result
