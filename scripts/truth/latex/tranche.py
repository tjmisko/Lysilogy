"""Versioned complete-paper manual evidence, separate from automatic source support.

The two original blind inventories are inputs, never rewritten as if they had
seen a later candidate. A separate reviewed construction record binds them.
"""
from collections import Counter
from copy import deepcopy
import math
from pathlib import Path

from annotations import canonical, document, require
from archive import read_archive, sha256
from native_exports import FORMAT as NATIVE_FORMAT, verify_native_export

FORMAT = 'k1-manual-tranche-v1'
REVIEW_FORMAT = 'k1-manual-tranche-review-v1'
MAX_BUNDLE_BYTES = 64 * 1024 * 1024
KINDS = {'figure', 'table', 'equation', 'statement', 'proof', 'algorithm'}
# Positive formal statements/proofs/algorithms need their own complete ownership
# codec. This format rejects those inputs; it can establish reviewed negatives.
POSITIVE_KINDS = {'figure', 'table', 'equation'}
ARTIFACTS = {'policy', 'selection', 'packet', 'source_export', 'native_export',
             'primary', 'primary_receipt', 'independent', 'independent_receipt',
             'independent_corrected', 'correction_receipt', 'math', 'math_receipt',
             'initial_review', 'role_review', 'visual_review', 'math_review',
             'construction', 'object_crosswalk', 'link_crosswalk', 'review'}


def exact(left, right, message):
    require(canonical(left) == canonical(right), message)


def unique(rows, key, message):
    require(isinstance(rows, list) and len(rows) <= 10000, message)
    result = {row[key]: row for row in rows}
    require(len(result) == len(rows), message)
    return result


def source_member(row, files, primary=False):
    member = {'path': row['member'] if primary else row['path'],
              'start': row['start'], 'end': row['end']}
    start, end = member['start'], member['end']
    require(member['path'] in files and type(start) is int and type(end) is int
            and 0 <= start < end <= len(files[member['path']]), 'manual source occurrence is invalid')
    raw = files[member['path']][start:end].encode()
    if 'member_sha256' in row:
        exact(row['member_sha256'], sha256(files[member['path']].encode()), 'manual source member hash differs')
    if 'text' in row:
        exact(row['text'].encode().hex(), raw.hex(), 'manual source excerpt differs')
    return member, sha256(raw)


def native_members(rows, index):
    require(isinstance(rows, list) and len(rows) <= 10000, 'manual native membership exceeds its bound')
    encoded = index['text'].encode('utf-16-le')
    pages = {page['number']: page for page in index['pages']}
    output = []
    for row in rows:
        start, end = row['start'], row['end']
        require(type(start) is int and type(end) is int and 0 <= start < end <= len(encoded) // 2,
                'manual native span is invalid')
        actual = encoded[start * 2:end * 2].decode('utf-16-le')
        if 'text' in row:
            exact(row['text'], actual, 'manual native excerpt differs')
        for key in ('text_sha256', 'text_utf8_sha256'):
            if key in row:
                exact(row[key], sha256(actual.encode()), 'manual native text hash differs')
        owner = [page['number'] for page in pages.values() if page['start'] <= start < end <= page['end']]
        require(len(owner) == 1, 'manual span crosses a PDF page')
        if 'page' in row:
            exact(row['page'], owner[0], 'manual native page differs')
        output.append({'start': start, 'end': end, 'native_text_sha256': sha256(actual.encode())})
    ordered = sorted(output, key=lambda row: row['start'])
    exact(output, ordered, 'manual native spans are reordered')
    require(all(a['end'] <= b['start'] for a, b in zip(output, output[1:])),
            'manual native spans overlap')
    return output


def covered(rows, index):
    encoded = index['text'].encode('utf-16-le')
    require(sum(row['end'] - row['start'] for row in rows) <= 500000,
            'manual membership exceeds its cumulative bound')
    result = set()
    for row in rows:
        position = row['start']
        for character in encoded[2 * row['start']:2 * row['end']].decode('utf-16-le'):
            if not character.isspace():
                result.add(position)
            position += len(character.encode('utf-16-le')) // 2
    return result


def rectangle(page, values, index, pixels=None, dpi=96):
    pages = {row['number']: row for row in index['pages']}
    require(type(page) is int and page in pages and len(values) == 4, 'manual region page/shape is invalid')
    require(all(type(value) in (int, float) and math.isfinite(value) for value in values),
            'manual rectangle is nonfinite or mistyped')
    left, top, right, bottom = values
    require(0 <= left < right <= pages[page]['width'] and 0 <= top < bottom <= pages[page]['height'],
            'manual rectangle exceeds its PDF page')
    if pixels is not None:
        require(len(pixels) == 4 and all(type(v) in (int, float) and math.isfinite(v) for v in pixels)
                and all(abs(value - pixel * 72 / dpi) < 1e-8 for value, pixel in zip(values, pixels)),
                'manual pixel conversion differs')
    return {'page': page, 'rect': dict(zip(('x_min', 'y_min', 'x_max', 'y_max'), values))}


def hash_ref(row, raw, message):
    require(row.get('sha256') == sha256(raw), message)
    if 'bytes' in row:
        require(type(row['bytes']) is int and row['bytes'] == len(raw), message)


def history(docs, raws, candidate_raw):
    """Validate original blind inputs independently from later construction."""
    packet, primary, initial = (docs[name] for name in ('packet', 'primary', 'independent'))
    pr, ir = docs['primary_receipt'], docs['independent_receipt']
    hash_ref(pr['inventory'], raws['primary'], 'primary receipt binds another inventory')
    hash_ref(ir['inventory'], raws['independent'], 'independent receipt binds another inventory')
    require(primary['annotator_role'] == pr['annotator_role'] == 'primary', 'primary role is absent')
    require(ir['annotator'] in initial['annotator'], 'independent annotator identity differs')
    exact(primary['protocol'], pr['blindness'], 'primary blindness receipt differs')
    require(primary['protocol']['all_original_pages_viewed_before_source_and_native'] is True,
            'primary enumeration did not precede source/native inspection')
    require({'Detector outputs', 'Automatic/parser inventories', 'Other annotator labels'}
            <= set(primary['protocol']['excluded_material_not_accessed']), 'primary blind exclusions are incomplete')
    require(initial.get('inspection_order') and ir.get('all_pages_inspected'),
            'independent complete inspection history is absent')
    exact(pr['inputs'], primary['inputs'], 'primary input histories differ')
    seen = {row['sha256'] for row in primary['inputs']}
    require({sha256(raws[name]) for name in ('packet', 'source_export', 'native_export')} <= seen,
            'primary did not read the original blind exports')
    require({row['sha256'] for row in packet['images']} <= seen, 'primary image inputs are incomplete')
    for name, key in [('packet', 'packet'), ('source_export', 'source_export'), ('native_export', 'native_export')]:
        hash_ref(ir[key], raws[name], 'independent blind input differs')
    # The initial inputs must not be retrofitted with this later construction.
    require('candidate' not in packet and 'source_inventory' not in packet
            and 'candidate_sha256' not in packet and 'inventory_sha256' not in packet,
            'initial blind packet claims a later candidate inventory')
    require(sha256(candidate_raw) not in seen and sha256(raws['construction']) not in seen,
            'later construction was relabeled as an original input')
    corrected = docs['independent_corrected']
    hash_ref(docs['correction_receipt']['inventory'], raws['independent_corrected'],
             'correction receipt binds another inventory')
    # This declared correction codec permits bibliography-only changes. Objects,
    # references, role inventories and blind inspection history must be identical.
    exact({k: v for k, v in initial.items() if k != 'bibliography'},
          {k: v for k, v in corrected.items() if k != 'bibliography'},
          'field correction changed object/reference/inspection evidence')
    supplement, receipt = docs['math'], docs['math_receipt']
    require(supplement['original_inventory_sha256'] == sha256(raws['independent'])
            and supplement['field_corrected_inventory_v2_sha256'] == sha256(raws['independent_corrected']),
            'math supplement lost its original correction chain')
    require(receipt['supplement_sha256'] == sha256(raws['math'])
            and type(receipt['supplement_bytes']) is int and receipt['supplement_bytes'] == len(raws['math']),
            'math supplement receipt differs')
    require(supplement['annotator'] == receipt['annotator'] == ir['annotator'],
            'math supplement was assigned to another annotator')
    construction = docs['construction']
    require(construction['not_an_original_annotator_input'] is True, 'later construction history is not explicit')
    hash_ref(construction['original_blind_packet'], raws['packet'], 'construction changes original packet')
    hash_ref(construction['primary_annotation'], raws['primary'], 'construction changes primary inventory')
    hash_ref(construction['independent_annotation_v2'], raws['independent_corrected'], 'construction changes corrected inventory')
    hash_ref(construction['candidate'], candidate_raw, 'construction names another candidate')


def validate_review(docs, raws, candidate):
    review = docs['review']
    require(review.get('format') == REVIEW_FORMAT and review.get('verdict') == 'clear_complete_visual_math_inventory'
            and review.get('findings') == [], 'manual tranche lacks an independently clear construction review')
    exact(review['artifacts'], {key: sha256(value) for key, value in raws.items() if key != 'review'},
          'construction review does not bind the complete evidence closure')
    require(review['source_inventory_sha256'] == candidate['source_inventory_sha256'],
            'construction reviewer verified another source inventory')
    identities = review['annotators']
    require(set(identities) == {'primary', 'independent'} and all(isinstance(v, str) and v.strip() for v in identities.values())
            and len(set(identities.values())) == 2 and isinstance(review['reviewer'], str)
            and review['reviewer'].strip() and review['reviewer'] not in identities.values(),
            'manual tranche review/annotator identities are not independent')
    require(identities['independent'] == docs['independent_receipt']['annotator'],
            'construction review names another independent annotator')
    require(review.get('initial_enumeration_before_source_and_native') is True
            and review.get('later_crosswalk_reviewed_after_annotation_freeze') is True,
            'construction review lacks the actual two-stage history')
    return review


def validate_objects(docs, raws, candidate, files, index):
    primary = unique(docs['primary']['objects'], 'id', 'primary object inventory repeats an ID')
    independent = unique(docs['independent_corrected']['objects'], 'id', 'independent object inventory repeats an ID')
    parsed = unique(candidate['source_inventory']['objects'], 'id', 'source object inventory repeats an ID')
    require(len(primary) == len(independent) == len(parsed), 'complete source/manual object counts differ')
    memberships = [span for row in primary.values() for key in ('native_body_members', 'native_caption_members') for span in row.get(key, [])]
    memberships += [span for row in independent.values() for key in ('body_native_spans', 'caption_native_spans', 'number_native_spans') for span in row.get(key, [])]
    require(all(type(span['start']) is int and type(span['end']) is int and 0 <= span['start'] < span['end'] for span in memberships)
            and sum(span['end'] - span['start'] for span in memberships) <= 1000000,
            'complete manual object membership exceeds its paper-wide bound')
    require(all(row['kind'] in POSITIVE_KINDS for row in independent.values()),
            'positive formal-object ownership is not supported by this evidence codec')
    counts = {kind: sum(row['kind'] == kind for row in independent.values()) for kind in sorted(KINDS)}
    exact(docs['review']['complete_kind_counts'], counts, 'reviewed complete kind counts differ')
    for kind, p_key, i_key in [('figure', 'figure', 'figure'), ('table', 'table', 'table'),
                               ('equation', 'numbered_equation', 'numbered_equation')]:
        exact(docs['primary']['counts'][p_key], counts[kind], 'primary positive count differs from complete inventory')
        exact(docs['independent_corrected']['counts'][i_key], counts[kind], 'independent positive count differs from complete inventory')
    exact(docs['visual_review']['primary_inventory_sha256'], sha256(raws['primary']), 'visual review names another primary')
    exact(docs['visual_review']['independent_inventory_sha256'], sha256(raws['independent']), 'visual review names another independent inventory')
    require(docs['visual_review']['verdict'] == 'clear_full_body_regions_for_later_validated_assembly',
            'full visual geometry is not independently reconciled')
    require(docs['math_review']['status'] == 'clear_independent_math_region_supplement_not_truth_admission',
            'math geometry is not independently reconciled')
    exact(docs['math_review']['supplement_sha256'], sha256(raws['math']), 'math review changes supplement')
    exact(docs['math_review']['supplement_receipt_sha256'], sha256(raws['math_receipt']), 'math review changes receipt')
    math_objects = unique(docs['math']['objects'], 'id', 'math supplement repeats an ID')
    math_review = unique(docs['math_review']['rows'], 'id', 'math review repeats an ID')
    expected_math = {key for key, row in independent.items() if row['kind'] == 'equation'}
    require(set(math_objects) == set(math_review) == expected_math, 'math geometry inventory is incomplete')
    visuals = {}; output_visual = []; output_math = []; id_map = {}; primary_map = {}; source_ids = set()
    for row in docs['visual_review']['regions']:
        key = (row['kind'], row['printed_number'], row['page'])
        require(key not in visuals, 'visual geometry repeats an object')
        visuals[key] = row
    used_visuals = set()
    for identifier, row in independent.items():
        member, text_hash = source_member(row['source']['member'], files)
        exact(row['source']['text_sha256'], text_hash, 'independent source text hash differs')
        exact(row['source']['member_sha256'], sha256(files[member['path']].encode()), 'independent source member hash differs')
        source = [s for s in parsed.values() if s['kind'] == row['kind'] and s['source_members'] == [member]]
        other = [p for p in primary.values() if p['kind'] == row['kind']
                 and source_member(p['source_environment'], files, True)[0] == member]
        require(len(source) == len(other) == 1, 'object lacks a unique two-inventory source occurrence')
        source, other = source[0], other[0]
        require(source['id'] not in source_ids and other['id'] not in primary_map, 'manual occurrence reused')
        source_ids.add(source['id']); id_map[identifier] = source['id']; primary_map[other['id']] = source['id']
        exact(row['source_labels'], source['labels'], 'manual object labels differ from source')
        exact(row['printed_label'], other['printed_number'], 'printed object labels disagree')
        # This first codec handles direct objects only. Nested/ancillary formal
        # ownership is rejected, not silently discarded from completeness.
        require(row['parent_object'] is None and row['nested_objects'] == [], 'nested manual ownership needs a supported codec')
        require(not any(s['source_span']['start'] < source['source_span']['start']
                        < source['source_span']['end'] < s['source_span']['end'] for s in parsed.values()),
                'nested source object cannot lose ownership')
        common = {'id': source['id'], 'kind': row['kind'], 'labels': source['labels'],
                  'source_members': [member], 'automatic_alignment_changed': False,
                  'manual_evidence_format': FORMAT}
        if row['kind'] in ('figure', 'table'):
            spans = native_members(row['caption_native_spans'], index)
            previous = native_members(other['native_caption_members'], index)
            require(spans and covered(spans, index) == covered(previous, index), 'complete caption memberships disagree')
            caption, caption_hash = source_member(row['caption_source']['member'], files)
            primary_caption, _ = source_member(other['source_caption'], files, True)
            exact(caption, primary_caption, 'source caption membership disagrees')
            require(caption['path'] == member['path'] and member['start'] <= caption['start'] < caption['end'] <= member['end'],
                    'caption lies outside its source object')
            exact(row['caption_source']['text_sha256'], caption_hash, 'caption source text differs')
            exact(row['caption_source']['member_sha256'], sha256(files[caption['path']].encode()), 'caption member hash differs')
            from parser import argument_commands
            commands = list(argument_commands(files[member['path']], {'caption'}, member['start'], member['end']))
            matching = [command for command in commands if command['start'] == row['caption_command_source']['start']
                        and command['end'] == row['caption_command_source']['end']]
            require(len(matching) == 1 and caption['end'] == matching[0]['end'] - 1
                    and caption['start'] == caption['end'] - len(matching[0]['value']),
                    'caption membership is not the complete declared source argument')
            if row['kind'] == 'table':
                require(covered(native_members(row['body_native_spans'], index), index)
                        == covered(native_members(other['native_body_members'], index), index), 'table body membership disagrees')
            native_region = row['visual_body_region']; page = native_region['page']
            require(len(native_region['rects']) == 1, 'multipart visual bodies need a supported codec')
            rectangle(page, [native_region['rects'][0][k] for k in ('x_min', 'y_min', 'x_max', 'y_max')], index)
            original = other['body_region']
            rectangle(original['page'], [original[k] for k in ('x_min', 'y_min', 'x_max', 'y_max')], index)
            key = (row['kind'], row['printed_label'], page)
            require(key in visuals and key not in used_visuals, 'visual review omits/repeats a source occurrence')
            used_visuals.add(key); chosen = visuals[key]
            require(chosen.get('full_body_basis'), 'selected visual region lacks a review basis')
            region = rectangle(page, [chosen['body_pdf_points'][k] for k in ('x_min', 'y_min', 'x_max', 'y_max')],
                               index, chosen['body_pixels_192dpi'], 192)
            require(row['pages'] == [page], 'visual object names different pages')
            require(all(next(p for p in index['pages'] if p['number'] == page)['start'] <= s['start'] < s['end']
                        <= next(p for p in index['pages'] if p['number'] == page)['end'] for s in spans),
                    'caption is outside its visual object page')
            output_visual.append({**common, 'printed_label': row['printed_label'], 'spans': spans,
                                  'text_sha256': source['text_sha256'], 'region': [region],
                                  'alignment_method': 'two complete blind inventories and reviewed post-freeze source/visual crosswalk'})
        else:
            supplement = math_objects[identifier]
            exact(supplement['original_object_sha256'], sha256(canonical(row)), 'math supplement changes its original object')
            exact(supplement['source'], row['source'], 'math supplement source differs')
            require(supplement['unresolved_expression_ambiguities'] == [] and supplement['source_vs_printed_judgment'],
                    'mathematical source/visual identity is unresolved')
            exact(supplement['source_text'], files[member['path']][member['start']:member['end']], 'math source excerpt differs')
            spans = native_members(row['body_native_spans'], index)
            require(spans and covered(spans, index) == covered(native_members(other['native_body_members'], index), index),
                    'complete mathematical body memberships disagree')
            native = supplement['existing_native_memberships']
            exact(native_members(native['body_native_spans'], index), spans, 'math supplement changes native body')
            numbers = native_members(row['number_native_spans'], index)
            exact(native_members(native['number_native_spans'], index), numbers, 'math supplement changes native number')
            require(covered(numbers, index) == covered(native_members([other['native_printed_number']], index), index),
                    'printed equation number memberships disagree')
            require(not covered(spans, index) & covered(numbers, index), 'equation body includes its printed number')
            comparison = math_review[identifier]
            require(comparison.get('source_native_memberships_unchanged') is True and comparison.get('root_visual_disposition'),
                    'math review contradicts its complete source/native disposition')
            exact(comparison['selected_body_boxes'], supplement['visual_body_boxes'], 'reviewed math body geometry differs')
            exact(comparison['selected_number_boxes'], supplement['printed_number_boxes'], 'reviewed math number geometry differs')
            regions = [rectangle(r['page'], r['rect'], index, r['pixels_96dpi']) for r in supplement['visual_body_boxes']]
            number_regions = [rectangle(r['page'], r['rect'], index, r['pixels_96dpi']) for r in supplement['printed_number_boxes']]
            require(regions and number_regions and {r['page'] for r in regions} == set(row['pages']), 'math page inventory differs')
            require(all(any(page['number'] == region['page'] and page['start'] <= span['start'] < span['end'] <= page['end']
                            for region in regions for page in index['pages']) for span in spans),
                    'mathematical membership lies outside its visual pages')
            output_math.append({**common, 'printed_heading': row['printed_label'], 'spans': spans, 'region': regions,
                                'printed_number_spans': numbers, 'printed_number_regions': number_regions,
                                'source_parent_object': None, 'source_child_objects': [], 'semantic_quote_truth': False,
                                'reading_index_fidelity': row['native_fidelity']})
    require(used_visuals == set(visuals), 'visual review contains unmatched objects')
    require(source_ids == set(parsed), 'manual inventory omits a parsed source occurrence')
    claimed = set()
    for row in output_visual + output_math:
        points = covered(row['spans'], index)
        require(not claimed & points, 'distinct manual objects claim the same caption/body membership')
        claimed.update(points)
    return output_visual, output_math, id_map, primary_map, counts


def validate_roles(docs, files):
    independent = docs['independent_corrected']; primary = docs['primary']; review = docs['role_review']
    require(review['verdict'] == 'clear_scope_classification_for_later_independent_negative_cohort_validation',
            'formal role review is not clear')
    exact(review['formal_scope_counts'], {'captioned_algorithm_or_code_listing': 0, 'proof': 0, 'theorem_like_statement': 0},
          'formal positive kinds require a supported ownership codec')
    for left, right in [('formal_statement', 'formal_statement'), ('proof', 'formal_proof'), ('captioned_algorithm', 'formal_algorithm')]:
        exact(primary['counts'][left], 0, 'primary formal inventory is not negative')
        exact(independent['counts'][right], 0, 'independent formal inventory is not negative')
    require(all(row.get('formal_statement') is False for row in primary['headings']),
            'a primary formal heading cannot disappear into a negative cohort')
    expected = unique(independent['statement_role_candidates'] + independent['manual_procedures_and_role_candidates'],
                      'id', 'formal role candidates repeat an ID')
    actual = unique(review['retained_role_candidates'], 'independent_candidate_id', 'role review repeats a candidate')
    require(set(actual) == set(expected), 'formal role review omits/adds a candidate')
    p_candidates = unique(primary['headings'] + primary['manual_lists'] + primary['narrative_procedure_candidates'],
                          'id', 'primary role inventory repeats an ID')
    used_primary = set()
    for identifier, original in expected.items():
        row = actual[identifier]
        require(row['candidate_retained'] is True and row['reason'], 'unresolved formal role disposition')
        require(row['proposed_disposition'] in {'outside_E1.4_statement_scope', 'outside_E1.5_algorithm_listing_scope'},
                'formal role is not explicitly outside the confirmed scope')
        member, digest = source_member(original['source']['member'], files)
        exact(original['source']['text_sha256'], digest, 'role candidate source excerpt differs')
        exact({'path': row['source_member'], 'start': row['source_span']['start'], 'end': row['source_span']['end']},
              member, 'reviewed role changed its source occurrence')
        exact(row['source_excerpt_sha256'], digest, 'reviewed role text differs')
        key = row['primary_candidate_id']
        require(key in p_candidates and key not in used_primary, 'role lacks unique primary correspondence')
        used_primary.add(key); other = p_candidates[key]
        field = next((key for key in ('source_environment', 'source_passage', 'source_heading') if key in other), None)
        require(field is not None, 'primary role lacks a source occurrence')
        p_member, _ = source_member(other[field], files, True)
        require(p_member['path'] == member['path'] and p_member['start'] == member['start']
                and p_member['end'] <= member['end'], 'primary/independent role occurrence differs')
    require({r['id'] for r in primary['manual_lists'] + primary['narrative_procedure_candidates']} <= used_primary,
            'primary procedure candidate was omitted')
    return deepcopy(review['retained_role_candidates'])


def validate_references(docs, candidate, files, index, id_map, primary_map):
    independent = docs['independent_corrected']; primary = docs['primary']
    links = {n: row for n, row in enumerate(candidate['source_inventory']['links']) if row['kind'] == 'reference'}
    require(len(independent['references']) == len(primary['references']) == len(links),
            'complete reference occurrence counts differ')
    exact(primary['counts']['source_reference'], len(links), 'primary declared reference inventory differs')
    exact(independent['counts']['explicit_reference_occurrences'], len(links), 'independent declared reference inventory differs')
    sources = {row['id']: row for row in candidate['source_inventory']['objects']}
    sections = unique(independent['section_destinations'], 'id', 'section destination repeats an ID')
    seen = set(); primary_seen = set(); references = []; visual = []; non_objects = []
    for row in independent['references']:
        require(row['unresolved_targets'] == [], 'manual reference has an unresolved target')
        member, digest = source_member(row['source']['member'], files)
        exact(row['source']['text_sha256'], digest, 'manual reference source hash differs')
        matches = [(n, l) for n, l in links.items() if l['source_members'] == [member]]
        other = [r for r in primary['references'] if source_member(r['source_occurrence'], files, True)[0] == member]
        require(len(matches) == len(other) == 1, 'manual reference lacks unique source/primary correspondence')
        (number, source), other = matches[0], other[0]
        require(number not in seen and other['id'] not in primary_seen, 'manual reference occurrence reused')
        seen.add(number); primary_seen.add(other['id'])
        exact(source['targets'], [row['source_label']], 'reference source target differs')
        exact(other['target_label'], row['source_label'], 'reference label inventories disagree')
        spans = native_members([row['native_number_span']], index)
        exact(spans, native_members([other['native_number']], index), 'reference numeric occurrence disagrees')
        exact(native_members([row['native_phrase_span']], index), native_members([other['native_occurrence']], index),
              'reference phrase occurrence disagrees')
        declared = row['destination_id']; expected = candidate['source_inventory']['label_targets'].get(row['source_label'])
        common = {'source_link': number, 'source_members': [member], 'span': spans[0]}
        if declared in id_map:
            target = id_map[declared]
            require(expected == target and primary_map.get(other['target_id']) == target,
                    'reference destination differs from source/both inventories')
            exact(row['destination_kind'], sources[target]['kind'], 'reference target kind is laundered')
            exact(other['target_kind'], sources[target]['kind'], 'primary reference target kind differs')
            bound = {**common, 'target': target, 'target_kind': sources[target]['kind']}
            (references if sources[target]['kind'] == 'equation' else visual).append(bound)
        else:
            require(expected not in sources and declared in sections and row['destination_kind'] == other['target_kind'] == 'section'
                    and declared == other['target_id'], 'object reference cannot become a non-object role')
            destination = sections[declared]; span, digest = source_member(destination['source']['member'], files)
            exact(destination['source']['text_sha256'], digest, 'section destination source differs')
            literal = files[span['path']][span['start']:span['end']]
            require('\\label{' + row['source_label'] + '}' in literal, 'section destination lacks the exact source label')
            non_objects.append({**common, 'target': declared, 'source_label': row['source_label'], 'destination_source_member': span})
    require(seen == set(links), 'source reference inventory is incomplete')
    return references, visual, non_objects


def validate_crosswalk(docs, candidate, id_map, primary_map):
    parsed = candidate['source_inventory']; objects = {row['id']: row for row in parsed['objects']}
    crosswalk = docs['object_crosswalk']
    exact(crosswalk['source_inventory_canonical_sha256'], candidate['source_inventory_sha256'], 'post-freeze source inventory differs')
    rows = unique(crosswalk['crosswalk'], 'independent_id', 'post-freeze object crosswalk repeats an occurrence')
    require(set(rows) == set(id_map) and {row['primary_id'] for row in rows.values()} == set(primary_map),
            'post-freeze crosswalk changes a complete annotation inventory')
    for key, row in rows.items():
        target = id_map[key]
        exact(row['parsed_ids'], [target], 'post-freeze crosswalk changes source identity')
        require(primary_map[row['primary_id']] == target and row['kind'] == objects[target]['kind'],
                'post-freeze crosswalk changes source kind or primary identity')
        exact([row['original_source_member']], objects[target]['source_members'], 'post-freeze crosswalk changes source span')
    require(crosswalk['unmatched_parsed_object_ids'] == [], 'post-freeze source inventory has an unexplained object')
    links = docs['link_crosswalk']['links']
    require(len(links) == len(parsed['links']), 'post-freeze link inventory is incomplete')
    seen = set()
    for row in links:
        n = row['source_link']
        require(type(n) is int and 0 <= n < len(parsed['links']) and n not in seen, 'post-freeze source link is invalid or repeated')
        seen.add(n); original = parsed['links'][n]
        exact([row['source_member']], original['source_members'], 'post-freeze link source span differs')
        exact(row['source_targets'], original['targets'], 'post-freeze link targets differ')
        exact(row['kind'], original['kind'], 'post-freeze link kind differs')
        field = 'citations' if row['kind'] == 'citation' else 'references'
        independent = [r for r in docs['independent_corrected'][field] if r['id'] == row['independent_id']]
        primary = [r for r in docs['primary'][field] if r['id'] == row['primary_id']]
        require(len(independent) == len(primary) == 1, 'post-freeze link lacks both original annotation IDs')
        member = independent[0]['source']['member']
        exact({key: member[key] for key in ('path', 'start', 'end')}, row['source_member'], 'independent link identity was reassigned')
        member = primary[0]['source_occurrence']
        exact({'path': member['member'], 'start': member['start'], 'end': member['end']}, row['source_member'],
              'primary link identity was reassigned')


def validate_tranche(candidate_raw, index_raw, source_raw, raws, verified_images):
    """Pure validation of a declared evidence closure; no live annotation or detector."""
    require(set(raws) == ARTIFACTS, 'manual tranche evidence inventory differs from its declared format')
    require(sum(len(raw) for raw in raws.values()) <= MAX_BUNDLE_BYTES, 'manual tranche exceeds its cumulative byte bound')
    docs = {key: document(raw) for key, raw in raws.items()}
    candidate = document(candidate_raw); wrapper = document(index_raw); index = wrapper['index']
    require(candidate['index']['sha256'] == sha256(index_raw) and candidate['source_sha256'] == sha256(source_raw),
            'manual tranche source/index artifacts differ')
    require(candidate['source_inventory_sha256'] == sha256(canonical(candidate['source_inventory'])),
            'manual tranche source inventory differs')
    packet = docs['packet']; construction = docs['construction']
    for key in ('arxiv_id', 'paper_id', 'index'):
        exact(packet[key], candidate[key], 'blind packet differs from frozen candidate identity')
        exact(construction[key], candidate[key], 'construction differs from frozen candidate identity')
    for kind in ('pdf', 'source'):
        exact(packet[kind]['sha256'], candidate[kind + '_sha256'], 'blind packet uses another corpus artifact')
        exact(construction[kind], packet[kind], 'construction changes a frozen corpus artifact')
    exact(packet['version'], construction['version'], 'construction changes the frozen source version')
    require(type(packet['version']) is int and packet['version'] > 0, 'manual source version is invalid')
    for key in ('arxiv_id', 'paper_id', 'version'):
        exact(docs['independent'][key], packet[key], 'independent inventory names another paper/version')
        exact(docs['primary']['paper'][key], packet[key], 'primary inventory names another paper/version')
        exact(docs['math']['paper'][key], packet[key], 'math supplement names another paper/version')
    exact(packet['stratum'], candidate['stratum'], 'manual packet changes stratum')
    history(docs, raws, candidate_raw)
    review = validate_review(docs, raws, candidate)
    files, members = read_archive(source_raw)
    exact(docs['source_export']['source_sha256'], sha256(source_raw), 'blind source export names another archive')
    exact(docs['source_export']['text_members'], files, 'blind source export changed a deposited member')
    exact(docs['source_export']['members'], members, 'blind source member inventory is incomplete')
    native_declaration = {'format': NATIVE_FORMAT, 'sha256': sha256(raws['native_export']),
                          'bytes': len(raws['native_export']), 'index_sha256': sha256(index_raw)}
    verify_native_export(index_raw, raws['native_export'], candidate['paper_id'], native_declaration)
    pages = [row['number'] for row in index['pages']]
    exact(review['pages_inspected'], pages, 'construction reviewer did not inspect every original page')
    exact(docs['primary']['protocol']['pages_seen'], pages, 'primary page inventory is incomplete')
    exact(docs['independent']['pages_inspected'], pages, 'independent page inventory is incomplete')
    exact(docs['independent_receipt']['all_pages_inspected'], pages, 'independent receipt page inventory is incomplete')
    exact(docs['independent_receipt']['images'], packet['images'], 'independent original images differ')
    require(len(packet['images']) == len(pages), 'packet image inventory is incomplete')
    exact({row['path']: row['sha256'] for row in packet['images']}, verified_images,
          'original image paths/hashes were not independently read')
    exact([row['page'] for row in packet['images']], pages, 'original images repeat or change page order')
    exact(construction['objects'], candidate['source_inventory']['objects'], 'construction drops source objects')
    exact(construction['links'], candidate['source_inventory']['links'], 'construction drops source links')
    visual, math_objects, id_map, primary_map, counts = validate_objects(docs, raws, candidate, files, index)
    validate_crosswalk(docs, candidate, id_map, primary_map)
    roles = validate_roles(docs, files)
    references, other, non_objects = validate_references(docs, candidate, files, index, id_map, primary_map)
    exact(review['reference_counts'], {'all': len(references) + len(other) + len(non_objects),
                                      'O4': len(references), 'visual': len(other), 'non_object': len(non_objects)},
          'reviewed reference denominator differs')
    binding = {key: sha256(raw) for key, raw in raws.items()}
    common = {'evidence_format': FORMAT, 'evidence_hashes': binding,
              'automatic_candidate_retained': True, 'final_k1_publication': False,
              'inventory_basis': 'two complete blind original-PDF inventories and separately reviewed later source/native crosswalk',
              'source_diagnostics_retained': deepcopy(candidate['source_inventory']['coverage']),
              'omitted_metrics': {'O8': 'bibliography codec not supplied', 'O9': 'bibliography codec not supplied',
                                  'O10': 'bibliography codec not supplied', 'O11': 'no independent ranking panel'},
              'retained_role_candidates': roles}
    output = deepcopy(candidate)
    output['manual_figure_table_overlay'] = {**deepcopy(common), 'objects': visual, 'metric_eligibility': {'O1': True, 'O2': True}}
    output['manual_object_overlay'] = {**deepcopy(common), 'objects': math_objects, 'references': references,
                                       'other_object_references': other, 'non_object_references': non_objects,
                                       'associated_content': [], 'reviewed_absent_kinds': [k for k in ('figure', 'table') if counts[k] == 0],
                                       'reference_coverage': {'source_occurrences': len(references) + len(other) + len(non_objects),
                                                              'object_occurrences': len(references) + len(other), 'O4_occurrences': len(references)},
                                       'metric_eligibility': {key: True for key in ('O3', 'O4', 'O5', 'O6', 'O7')}}
    return output


def attach_tranche(cache, corpus_root, data_root, candidate_raw, paper, mapped, relative):
    """Read only the explicitly declared current-format bundle and fixed roots."""
    from builder import fingerprint_file, safe_file
    from manual import bounded
    manifest_raw = bounded(cache, relative + '/manifest.json')
    manifest = document(manifest_raw)
    require(manifest.get('format') == FORMAT and type(manifest.get('schema_version')) is int
            and manifest['schema_version'] == 1, 'unsupported manual tranche format')
    require(set(manifest['artifacts']) == ARTIFACTS, 'manual tranche manifest is incomplete')
    raws = {}; records = {}; total_bytes = 0
    for key, row in manifest['artifacts'].items():
        require(set(row) == {'path', 'sha256', 'bytes'}, 'unsupported manual artifact declaration')
        raw = bounded(cache, row['path'])
        total_bytes += len(raw)
        require(total_bytes <= MAX_BUNDLE_BYTES, 'manual tranche exceeds its cumulative byte bound')
        hash_ref(row, raw, 'manual tranche artifact bytes differ')
        raws[key] = raw; records[key] = document(raw)
    candidate = document(candidate_raw)
    exact(candidate['index'], mapped['index'], 'tranche candidate names another native index')
    exact(candidate['paper_id'], mapped['paper_id'], 'tranche candidate names another PaperId')
    exact(candidate['arxiv_id'], paper['arxiv_id'], 'tranche candidate names another arXiv paper')
    packet = records['packet']
    exact(packet['version'], paper['version'], 'tranche packet changes frozen source version')
    exact(packet['stratum'], paper['stratum'], 'tranche packet changes frozen stratum')
    index_raw = bounded(data_root, mapped['index']['path'])
    require(sha256(index_raw) == mapped['index']['sha256'] and mapped['pdf_sha256'] == paper['pdf']['sha256'],
            'tranche mapped native PDF/index hashes differ')
    source_raw = None
    for kind in ('pdf', 'source'):
        path = safe_file(corpus_root, paper[kind]['path'])
        digest, size = fingerprint_file(path, cap=64 * 1024 * 1024)
        require(digest == paper[kind]['sha256'] == candidate[kind + '_sha256'] and size == paper[kind]['bytes'],
                'tranche corpus artifact differs from frozen receipts')
        exact(packet[kind], paper[kind], 'tranche packet changes corpus receipt')
        if kind == 'source':
            source_raw = path.read_bytes()
            require(len(source_raw) == size and sha256(source_raw) == digest, 'source changed during tranche read')
    # Every relative path recorded by original annotators resolves against their
    # original annotation root, never the later construction bundle directory.
    annotation_root = safe_file(cache, manifest['artifacts']['policy']['path']).parent
    def checked_original(row, expected_raw=None):
        path = Path(row['path'])
        absolute = path if path.is_absolute() else annotation_root / path
        relative_path = str(absolute.relative_to(cache))
        path = safe_file(cache, relative_path)
        digest, size = fingerprint_file(path, cap=32 * 1024 * 1024)
        require(digest == row['sha256'] and ('bytes' not in row or type(row['bytes']) is int and size == row['bytes']),
                'original annotation path bytes differ')
        if expected_raw is not None:
            require(digest == sha256(expected_raw), 'original annotation path names another input')
        return digest
    for row in records['primary']['inputs']:
        checked_original(row)
    for key in ('packet', 'source_export', 'native_export'):
        checked_original(records['independent_receipt'][key], raws[key])
    for row in records['correction_receipt']['original_files_preserved']:
        checked_original(row)
    preserved = {row['sha256'] for row in records['correction_receipt']['original_files_preserved']}
    require({sha256(raws['independent']), sha256(raws['independent_receipt'])} <= preserved,
            'correction history omits the original annotation/receipt')
    for row in records['math_receipt']['inputs']:
        checked_original(row)
    checked_original(records['primary_receipt']['inventory'], raws['primary'])
    checked_original(records['independent_receipt']['inventory'], raws['independent'])
    checked_original(records['correction_receipt']['inventory'], raws['independent_corrected'])
    images = {row['path']: checked_original(row) for row in packet['images']}
    require(len(images) == len(packet['images']), 'original image path repeated')
    exact(packet['selection_sha256'], sha256(raws['selection']), 'tranche changed frozen availability selection')
    exact(packet['policy_sha256'], sha256(raws['policy']), 'tranche changed frozen availability policy')
    selected = [row for row in records['selection']['selected'] if row['arxiv_id'] == paper['arxiv_id']]
    require(len(selected) == 1, 'tranche paper is absent or duplicated in frozen selection')
    exact(selected[0]['mapping'], mapped, 'tranche selection uses another frozen mapping')
    result = validate_tranche(candidate_raw, index_raw, source_raw, raws, images)
    # Verify evidence remains fixed through the entire bounded read/validation.
    for key, row in manifest['artifacts'].items():
        require(bounded(cache, row['path']) == raws[key], 'tranche evidence changed during validation')
    require(bounded(cache, relative + '/manifest.json') == manifest_raw, 'tranche manifest changed during validation')
    result['manual_assembly'] = {'format': FORMAT, 'manifest_sha256': sha256(manifest_raw),
                                 'candidate_sha256': sha256(candidate_raw), 'final_k1_publication': False,
                                 'omitted_manual_kinds': ['bib_entry'],
                                 'omitted_metrics': ['O8', 'O9', 'O10', 'O11'],
                                 'source_inventory_policy': 'automatic source exclusions retained; complete manual inventories separately reviewed'}
    return result
