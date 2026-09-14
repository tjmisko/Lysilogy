"""Explicit blind visual/negative evidence; original annotation schemas stay intact.

This codec supports direct figures/tables and separately reviewed formal negatives.
It does not interpret positive mathematics or publish bibliography/ranking truth.
"""
from collections import Counter
from copy import deepcopy
import re

from annotations import canonical, document, require
from archive import read_archive, sha256
from native_exports import FORMAT as NATIVE_FORMAT, verify_native_export
from parser import argument_commands, parse_project, reference_names_verified
from tex import comments
from tranche import (covered, exact as exact_json, hash_ref, member_pages, native_members, rectangle,
                     source_member, unique, verify_original_file)

FORMAT = 'k1-manual-visual-tranche-v1'
REVIEW_FORMAT = 'k1-manual-visual-tranche-review-v1'
MAX_BUNDLE_BYTES = 64 * 1024 * 1024
ARTIFACTS = {'policy', 'selection', 'prompt', 'packet', 'source_export', 'native_export',
             'primary', 'primary_receipt', 'independent', 'independent_receipt',
             'initial_review', 'role_review', 'visual_review', 'construction',
             'object_crosswalk', 'link_crosswalk', 'review'}
PRIMARY_ROLES = ('definition_candidates', 'narrative_method_candidates',
                 'manual_procedure_candidates', 'constraint_lists', 'system_lists')
INDEPENDENT_ROLES = ('prose_procedure_candidates', 'informal_definition_candidates')
COORDS = ('x_min', 'y_min', 'x_max', 'y_max')


def exact(left, right, message):
    if isinstance(left, set) and isinstance(right, set):
        require(left == right, message)
    else:
        exact_json(left, right, message)


def source(row, files):
    member, digest = source_member(row, files, True)
    require('text' in row, 'source evidence lacks its original excerpt')
    if 'sha256' in row:
        exact(row['sha256'], digest, 'source excerpt hash differs')
    return member


def native(row, index):
    spans = native_members([row], index)
    if 'sha256' in row:
        exact(row['sha256'], spans[0]['native_text_sha256'], 'native excerpt hash differs')
    if 'pages' in row:
        exact(row['pages'], sorted(member_pages(spans, index)), 'native declared pages differ')
    return spans


def members(rows, index):
    require(isinstance(rows, list) and len(rows) <= 10000, 'native component inventory exceeds its bound')
    for row in rows:
        native(row, index)
    return native_members(rows, index)


def source_identity(row):
    return row['member'], row['start'], row['end']


def parsed_identity(row):
    require(len(row['source_members']) == 1, 'direct visual source requires one original member')
    member = row['source_members'][0]
    return member['path'], member['start'], member['end']


def current_source_binding(docs, candidate, files):
    """A historical candidate cannot supply current label-confidence claims."""
    current = parse_project(files)
    construction = docs['construction']
    exact(construction['current_source_inventory'], current, 'construction current source inventory differs')
    exact(construction['current_source_inventory_sha256'], sha256(canonical(current)), 'construction current source hash differs')
    historical = candidate['source_inventory']
    changed = sorted(k for k in set(current) | set(historical) if canonical(current.get(k)) != canonical(historical.get(k)))
    exact(construction['changed_source_inventory_fields'], changed, 'construction conceals historical/current source differences')
    # This codec supports a direct one-to-one source crosswalk. Other source
    # occurrence changes need an explicitly extended ownership contract.
    for key in ('objects', 'links'):
        def occurrences(inventory):
            return [{k: row[k] for k in ('id', 'kind', 'source_members', 'labels', 'targets') if k in row}
                    for row in inventory[key]]
        exact(occurrences(current), occurrences(historical), 'current source occurrence crosswalk differs from historical candidate')
    return current


def audit_excerpts(value, files, index):
    """Check all retained source/native excerpts, including omitted metric evidence."""
    visited = 0
    def walk(node, depth):
        nonlocal visited
        visited += 1
        require(depth <= 32 and visited <= 200000, 'annotation structure exceeds its bound')
        if isinstance(node, dict):
            if isinstance(node.get('member'), str) and {'start', 'end', 'text'} <= node.keys():
                source(node, files)
            elif {'start', 'end', 'text', 'unit'} <= node.keys() and node['unit'].startswith('UTF-16'):
                # Retained narrative context envelopes can cross pages. Admitted
                # captions, notes and reference occurrences use strict native().
                encoded = index['text'].encode('utf-16-le')
                start, end = node['start'], node['end']
                require(type(start) is int and type(end) is int and 0 <= start < end <= len(encoded)//2,
                        'retained native excerpt has invalid UTF-16 bounds')
                text = encoded[2*start:2*end].decode('utf-16-le')
                exact(node['text'], text, 'retained native excerpt differs')
                if 'sha256' in node:
                    exact(node['sha256'], sha256(text.encode()), 'retained native excerpt hash differs')
                if 'pages' in node:
                    exact(node['pages'], [p['number'] for p in index['pages'] if p['start'] < end and start < p['end']],
                          'retained native excerpt pages differ')
            for child in node.values():
                walk(child, depth + 1)
        elif isinstance(node, list):
            require(len(node) <= 10000, 'annotation list exceeds its bound')
            for child in node:
                walk(child, depth + 1)
    walk(value, 0)


def history(docs, raws, candidate, candidate_raw, pages):
    p, i, pr, ir = (docs[k] for k in ('primary', 'independent', 'primary_receipt', 'independent_receipt'))
    require(p.get('schema_version') == pr.get('schema_version') == 1
            and i.get('schema') == 'lysilogy.blind-manual-inventory.v1'
            and ir.get('schema') == 'lysilogy.blind-manual-receipt.v1', 'unsupported original visual annotation schemas')
    hash_ref(pr['inventory'], raws['primary'], 'primary receipt names another inventory')
    exact(ir['inventory_sha256'], sha256(raws['independent']), 'independent receipt names another inventory')
    exact(ir['inventory_bytes'], len(raws['independent']), 'independent inventory size differs')
    require(p['annotator_role'] == pr['annotator_role'] == 'primary'
            and i['annotation_role'].startswith('independent')
            and ir['annotation_role'].startswith('independent'), 'original annotation roles differ')
    for original in (p, i, pr, ir):
        for key in ('arxiv_id', 'paper_id', 'version'):
            exact(original['paper'][key], docs['packet'][key], 'original annotation paper/version differs')
    exact(p['protocol'], pr['protocol'], 'primary inspection history differs')
    require(p['protocol']['all_original_pages_viewed_before_source_and_native'] is True
            and ir['all_pages_before_source_native'] is True, 'complete pages were not inspected first')
    for actual in (p['protocol']['pages_seen'], pr['pages_viewed_first'], i['pages_inspected'], ir['page_inspection_order']):
        exact(actual, pages, 'original annotation omitted or reordered pages')
    require({'Raw reading-index', 'Parser/automatic inventories', 'Detector outputs', 'Any independent annotator labels'}
            <= set(p['protocol']['excluded_material_not_accessed']), 'primary blind exclusions are incomplete')
    require(ir.get('read_boundary') and 'parser/detector' in ir['read_boundary']
            and 'primary labels' in ir['read_boundary'], 'independent blind exclusions are absent')
    exact(pr['inputs'], p['inputs'], 'primary original input ledger differs')
    required = {sha256(raws[k]) for k in ('packet', 'source_export', 'native_export', 'prompt')}
    required.update(row['sha256'] for row in docs['packet']['images'])
    for ledger in (p['inputs'], ir['inputs']):
        require(isinstance(ledger, list) and len(ledger) <= 1000, 'original input ledger exceeds its bound')
        seen = {row['sha256'] for row in ledger}
        require(required <= seen, 'original annotation inputs omit an export, prompt or page')
        require(sha256(candidate_raw) not in seen and sha256(raws['construction']) not in seen,
                'later construction was relabeled as an original input')
    exact(ir['packet_identities'], docs['packet'], 'independent original packet differs')
    packet = docs['packet']
    require(not {'candidate', 'source_inventory', 'candidate_sha256', 'inventory_sha256'} & packet.keys(),
            'blind packet contains a later parser inventory')
    construction = docs['construction']
    require(construction['not_an_original_annotator_input'] is True, 'construction history is not explicit')
    hash_ref(construction['candidate'], candidate_raw, 'construction changes its automatic candidate')
    for key in ('primary', 'independent', 'packet', 'source_export', 'native_export'):
        hash_ref(construction['original_artifacts'][key], raws[key], 'construction changes an original artifact')
    exact(construction['objects'], candidate['source_inventory']['objects'], 'construction drops source objects')
    exact(construction['links'], candidate['source_inventory']['links'], 'construction drops source links')


def validate_roles(docs, files):
    p, i, review = docs['primary'], docs['independent'], docs['role_review']
    require(review['status'] == 'clear_scoped_zero_formal_kind_evidence_for_later_validation'
            and review['truth_admitted'] is False, 'formal negative review is absent')
    exact(review['formal_kind_counts_supported_by_root_visual_review'],
          {'algorithm': 0, 'numbered_equation': 0, 'proof': 0, 'statement': 0}, 'formal positive inventory requires another codec')
    for key in ('numbered_equation', 'unnumbered_display_equation', 'formal_statement', 'formal_proof', 'captioned_algorithm_or_listing'):
        exact(p['counts'][key], 0, 'primary formal/display inventory is not negative')
    for key in ('numbered_equations', 'formal_statements', 'formal_proofs', 'captioned_algorithms_or_code_listings'):
        exact(i['counts'][key], 0, 'independent formal inventory is not negative')
    require(not any(row.get('formal_statement') for row in p['headings']), 'formal heading cannot disappear into negative evidence')
    for side, original, categories in [('primary', p, PRIMARY_ROLES), ('independent', i, INDEPENDENT_ROLES)]:
        expected = {(category, row['id']): row for category in categories for row in original[category]}
        require(len(expected) == sum(len(original[k]) for k in categories), 'informal role occurrence repeats an ID')
        actual = {(row['category'], row['id']): row for row in review['retained_candidate_evidence'][side]}
        exact(sorted(actual), sorted(expected), 'role review omits or adds an informal candidate')
        require(len(actual) == len(review['retained_candidate_evidence'][side]), 'role review repeats a candidate')
        for key, row in actual.items():
            require(row['role'] and row['source_spans'], 'role review lacks source justification')
            retained = expected[key]
            spans = []
            for field, value in retained.items():
                if isinstance(value, dict) and {'member', 'start', 'end', 'text'} <= value.keys():
                    member = source(value, files)
                    spans.append({'field': field, 'member': member['path'], 'start': member['start'], 'end': member['end'],
                                  'text_sha256': sha256(files[member['path']][member['start']:member['end']].encode())})
            exact(row['source_spans'], spans, 'role review changes an original source occurrence')
    return deepcopy(review['retained_candidate_evidence'])


def table_note_claims(docs):
    """Bind original numbered ancillary groups through the reviewed crosswalk."""
    rows = docs['object_crosswalk']['table_notes']
    require(isinstance(rows, list) and len(rows) <= 100, 'table-note crosswalk exceeds its bound')
    claims = unique(rows, 'primary_id', 'table-note primary owner repeats')
    expected = {r['parent'] for r in docs['primary']['attached_table_notes']}
    exact(set(claims), expected, 'table-note crosswalk omits or invents an owner')
    groups = {k for k in docs['independent']['ancillary'] if re.fullmatch(r'table_[1-9][0-9]*_notes', k)}
    scopes = {k for k in docs['visual_review'] if re.fullmatch(r'table_[1-9][0-9]*_note_scope', k)}
    used_groups = set(); used_scopes = set()
    for row in rows:
        require(set(row) == {'source_id', 'primary_id', 'independent_id', 'independent_notes_key', 'review_scope_key'},
                'unsupported table-note crosswalk fields')
        group, scope = row['independent_notes_key'], row['review_scope_key']
        require(isinstance(group, str) and re.fullmatch(r'table_[1-9][0-9]*_notes', group)
                and scope == group[:-1] + '_scope' and group in groups and scope in scopes,
                'table-note crosswalk names an unsupported original group')
        require(group not in used_groups and scope not in used_scopes, 'table-note group is reused')
        used_groups.add(group); used_scopes.add(scope)
    exact(used_groups, groups, 'table-note crosswalk omits an independent ancillary group')
    exact(used_scopes, scopes, 'table-note crosswalk omits a reviewed scope')
    return claims


def table_components(p, i, docs, files, index, page, claim=None):
    """Compose exactly recorded core/notes/markers, never a surrounding text envelope."""
    core = members(i['full_visual_body']['native_members'], index)
    full = members(p['native_body_members'], index)
    notes = [row for row in docs['primary']['attached_table_notes'] if row['parent'] == p['id']]
    if not notes:
        require(claim is None, 'table-note crosswalk invents attached notes')
        exact(covered(core, index), covered(full, index), 'complete table body memberships disagree')
        require(member_pages(core, index) == {page}, 'table body crosses its visual page')
        return []
    require(claim is not None and claim['primary_id'] == p['id'] and claim['independent_id'] == i['id'],
            'table-note crosswalk changes the original owner')
    original_notes = docs['independent']['ancillary'][claim['independent_notes_key']]
    require(len(notes) == len(original_notes) and len(notes) <= 100, 'attached note inventory differs')
    exact(covered(members(p['native_core_members'], index), index), covered(core, index), 'table core memberships differ')
    review = docs['visual_review'][claim['review_scope_key']]
    require(review['core_plus_notes_plus_markers_equals_primary'] is True and review['decision'], 'full table-note scope is not reviewed')
    markers = review['original_superscript_markers']
    require(len(markers) == len(notes), 'table-note marker inventory differs')
    occupied = covered(core, index)
    components = []
    used_notes = set(); used_markers = set()
    for note in notes:
        original_source = source(note['source_note'], files)
        owner = source(p['source_environment'], files)
        require(original_source['path'] == owner['path'] and owner['start'] <= original_source['start'] < original_source['end'] <= owner['end'],
                'table note escapes its original source owner')
        matches = [row for row in original_notes if row['source']['member'] == original_source['path']
                   and row['source']['end'] == original_source['end']]
        require(len(matches) == 1 and matches[0]['printed_label'] not in used_notes, 'table note has no unique independent source')
        other = matches[0]; label = other['printed_label']; used_notes.add(label)
        require(isinstance(label, str) and label.isascii() and label.isdigit(), 'unsupported table-note marker')
        visible = source(other['source'], files)
        prefix = files[visible['path']][original_source['start']:visible['start']]
        exact(prefix, '$^' + label + '$', 'table note source prefix is not the printed marker')
        text_spans = native(other['native'], index)
        spans = members(note['native_body_members'], index)
        # A superscript can be emitted later in native reading order. Its
        # reviewed exact token must belong to this separately recorded note.
        candidates = [m for m in markers if m['text'] == label
                      and any(s['start'] <= m['start'] < m['end'] <= s['end'] for s in spans)]
        require(len(candidates) == 1, 'table note has no unique original superscript marker')
        marker = candidates[0]; marker_spans = native(marker, index)
        require(marker['start'] not in used_markers, 'table note reuses a marker'); used_markers.add(marker['start'])
        exact(marker['token'], next((t for t in index['tokens'] if t['start'] == marker['start'] and t['end'] == marker['end']), None),
              'table superscript token differs from native geometry')
        spans = members(note['native_body_members'], index)
        points = covered(text_spans, index) | covered(marker_spans, index)
        exact(points, covered(spans, index), 'table note body adds or drops native membership')
        require(not occupied & points and member_pages(spans, index) == {page}, 'table-note ownership overlaps or crosses pages')
        occupied |= points
        for callout in note['source_row_callouts']:
            c = source(callout, files)
            require(original_source['path'] == c['path'] == p['source_environment']['member']
                    and p['source_environment']['start'] <= c['start'] < c['end'] <= p['source_environment']['end'],
                    'table-note callout escapes its source owner')
            exact(callout['text'], prefix, 'table-note row marker differs')
        callouts = members(note['native_row_callouts'], index)
        require(covered(callouts, index) <= covered(core, index) and member_pages(callouts, index) <= {page},
                'table note row callout escapes its core owner')
        require(len(callouts) == len(note['source_row_callouts']), 'table note callout inventory differs')
        components.append({'kind': 'attached_table_note', 'original_primary_id': note['id'], 'original_independent_label': label,
                           'source_members': [original_source], 'spans': spans, 'marker_spans': marker_spans,
                           'text_spans': text_spans, 'source_row_callouts': deepcopy(note['source_row_callouts']),
                           'native_row_callouts': callouts})
    exact(occupied, covered(full, index), 'table core plus notes does not equal the complete body')
    exact(len(covered(core, index)), review['independent_core_nonwhitespace'], 'reviewed table core size differs')
    exact(len(occupied), review['primary_full_body_nonwhitespace'], 'reviewed full table size differs')
    exact(used_notes, {row['printed_label'] for row in original_notes}, 'an independent table note was omitted')
    return components


def validate_objects(docs, candidate, files, index):
    p, i = docs['primary'], docs['independent']
    primary = unique(p['objects'], 'id', 'primary object ID repeats')
    independent = unique(i['objects'], 'id', 'independent object ID repeats')
    parsed = unique(candidate['source_inventory']['objects'], 'id', 'parsed source occurrence repeats')
    claims = table_note_claims(docs)
    require(len(primary) == len(independent) == len(parsed), 'complete visual inventories differ')
    require(all(row['kind'] in ('figure', 'table') for row in list(primary.values()) + list(independent.values()) + list(parsed.values())),
            'positive nonvisual object requires a supported codec')
    source_spans = [parsed_identity(row) for row in parsed.values()]
    for position, (path, start, end) in enumerate(source_spans):
        require(all(path != other_path or end <= other_start or other_end <= start
                    for other_path, other_start, other_end in source_spans[position + 1:]),
                'nested or overlapping visual source requires another ownership codec')
    counts = dict.fromkeys(('figure', 'table'), 0) | dict(Counter(row['kind'] for row in parsed.values()))
    for kind, plural in [('figure', 'figures'), ('table', 'tables')]:
        exact(p['counts'][kind], counts[kind], 'primary visual count differs')
        exact(i['counts'][plural], counts[kind], 'independent visual count differs')
    regions = {(r['kind'], r['printed_number'], r['page']): r for r in docs['visual_review']['regions']}
    require(len(regions) == len(docs['visual_review']['regions']) == len(parsed), 'visual review region inventory differs')
    require(docs['visual_review']['verdict'] == 'clear_full_body_regions_for_later_validated_assembly', 'visual review is not clear')
    output = []; associated = []; pmap = {}; imap = {}; used = set()
    for iid, row in independent.items():
        original = source(row['source'], files)
        pmatches = [v for v in primary.values() if source_identity(v['source_environment']) == source_identity(row['source'])]
        matches = [v for v in parsed.values() if parsed_identity(v) == source_identity(row['source'])]
        require(len(matches) == len(pmatches) == 1, 'visual object lacks unique original source correspondence')
        other, parsed_row = pmatches[0], matches[0]; sid = parsed_row['id']
        require(sid not in used and other['id'] not in pmap, 'visual source or primary occurrence reused'); used.add(sid)
        pmap[other['id']] = sid; imap[iid] = sid
        exact(row['kind'], other['kind'], 'original visual kinds disagree')
        exact(row['kind'], parsed_row['kind'], 'visual kind differs from its source occurrence')
        require(len(row['source_labels']) == 1, 'direct visual object needs a unique source label')
        exact(row['source_labels'], parsed_row['labels'], 'visual source labels disagree')
        exact(other['source_label'], row['source_labels'][0], 'primary visual label differs')
        exact(other['printed_number'], row['printed_label'], 'printed visual labels disagree')
        source(other['source_environment'], files)
        caption = source(row['caption_source'], files)
        exact(caption, source(other['source_caption'], files), 'complete source caption membership differs')
        require(caption['path'] == original['path'] and original['start'] <= caption['start'] < caption['end'] <= original['end'],
                'caption lies outside its source object')
        commands = list(argument_commands(comments(files[original['path']]), {'caption'}, original['start'], original['end']))
        require(len(commands) == 1 and commands[0]['end'] - 1 == caption['end']
                and caption['start'] == caption['end'] - len(commands[0]['value']), 'source caption is not the complete active argument')
        spans = native(row['caption_native'], index)
        exact(covered(spans, index), covered(members(other['native_caption_members'], index), index), 'complete native captions disagree')
        body = row['full_visual_body']; page = body['page']
        require(member_pages(spans, index) == {page} and row['pages'] == [page], 'caption has another visual page')
        key = (row['kind'], row['printed_label'], page); require(key in regions, 'visual review omits an object')
        chosen = regions[key]
        image = next((v for v in docs['packet']['images'] if v['page'] == page), None)
        require(image is not None and chosen['source_image_sha256'] == image['sha256'] and chosen['full_body_basis'],
                'visual review changed its original page or full-body basis')
        require(image['dpi'] == 96, 'this visual codec requires the reviewed 96dpi originals')
        region = rectangle(page, [chosen['body_pdf_points'][k] for k in COORDS], index, chosen['body_pixels_96dpi'], 96)
        rectangle(page, [body['region'][k] for k in COORDS], index)
        rectangle(other['body_region']['page'], [other['body_region'][k] for k in COORDS], index)
        exact(other['body_region']['page'], page, 'original visual region pages disagree')
        if row['kind'] == 'table':
            claim = claims.get(other['id'])
            require(claim is None or claim['source_id'] == sid, 'table notes change their parsed source owner')
            for component in table_components(other, row, docs, files, index, page, claim):
                associated.append({**component, 'source_owner': sid})
        else:
            require(other['id'] not in claims, 'table notes cannot attach to a figure')
        output.append({'id': sid, 'kind': row['kind'], 'labels': row['source_labels'], 'printed_label': row['printed_label'],
                       'spans': spans, 'region': region, 'source_members': [original],
                       'manual_evidence_format': FORMAT})
    require(used == set(parsed), 'manual visual inventory omits a source occurrence')
    claimed = set()
    for obj in output:
        points = covered(obj['spans'], index)
        require(not claimed & points, 'visual objects share caption membership'); claimed |= points
    return output, associated, pmap, imap, dict(counts)


def validate_links(docs, candidate, files, index, pmap, imap):
    p, i = docs['primary'], docs['independent']; inventory = candidate['source_inventory']
    local = [r for r in i['references'] if r['kind'] != 'external_section_reference']
    external = [r for r in i['references'] if r['kind'] == 'external_section_reference']
    exact(external, docs['role_review']['independent_external_reference_retained'], 'external locator was omitted or changed')
    for row in external:
        require(row['target_subsection'] is None and row['target_ambiguities'], 'unknown external subsection was resolved without evidence')
        source(row['source'], files); native(row['native'], index)
    references = []; sections = []; crosswalk = []; claimed = set(); native_claimed = set()
    parsed_refs = [(n, r) for n, r in enumerate(inventory['links']) if r['kind'] == 'reference']
    require(len(local) == len(p['references']) == len(parsed_refs), 'complete local reference inventories differ')
    for row in local:
        matches = [(n, r) for n, r in parsed_refs if parsed_identity(r) == source_identity(row['source_command'])]
        originals = [r for r in p['references'] if source_identity(r['source_command']) == source_identity(row['source_command'])]
        require(len(matches) == len(originals) == 1, 'reference lacks unique original source occurrence')
        n, parsed = matches[0]; other = originals[0]
        require(n not in claimed and row['target_ambiguities'] == [], 'local reference is reused or unresolved'); claimed.add(n)
        source(row['source_command'], files); source(other['source_command'], files)
        exact(parsed['targets'], [row['source_label']], 'reference source targets differ')
        require(reference_names_verified(inventory, parsed['targets'])
                and not any(k in inventory.get('ambiguous_labels', {}) for k in parsed['targets']), 'reference source target is ambiguous')
        exact(other['source_target_label'], row['source_label'], 'primary reference label differs')
        whole = native(row['native'], index)
        exact(whole, native(other['native_occurrence'], index), 'reference occurrence membership differs')
        number = native(other['native_number'], index)
        require(covered(number, index) and covered(number, index) <= covered(whole, index)
                and other['native_number']['text'] == other['printed_number'],
                'reference number is not an exact part of the printed phrase')
        points = covered(number, index)
        require(not native_claimed & points, 'distinct source links reuse a native occurrence')
        native_claimed |= points
        target = inventory['label_targets'].get(row['source_label'])
        if row['kind'] == 'object_reference':
            require(imap.get(row['target_id']) == pmap.get(other['target']) == target, 'reference visual target differs')
            destination = next(o for o in p['objects'] if o['id'] == other['target'])
            exact(other['printed_number'], destination['printed_number'], 'reference number differs from its visual destination')
            references.append({'source_link': n, 'target': target, 'spans': number, 'occurrence_spans': whole})
        else:
            require(target is None, 'visual object label cannot be reclassified as a section')
            require(row['kind'] == 'section_reference' and other['target_kind'] == 'section', 'unsupported non-object reference role')
            matches = [s for s in i['nonobject_destinations'] if s['id'] == row['target_id']]
            require(len(matches) == 1 and other['target'] == 'section:' + row['source_label'], 'section target lacks original evidence')
            destination = matches[0]; heading = source(destination['source_heading'], files); label = source(destination['source_label'], files)
            exact(destination['source_label']['text'], '\\label{' + row['source_label'] + '}', 'section destination changes its literal label')
            occurrences = [o for o in inventory['label_occurrences'] if o['label'] == row['source_label']]
            require(len(occurrences) == 1 and occurrences[0]['owner']['role'] == 'section', 'section label has no unique current section owner')
            occurrence = occurrences[0]; owner = occurrence['owner']
            exact(occurrence['source_members'], [label], 'section label source occurrence differs')
            require(heading['path'] == label['path'] and heading['start'] == owner['start']
                    and heading['end'] <= label['start'] < label['end'] <= owner['end'], 'section heading does not own its label')
            commands = list(argument_commands(comments(files[heading['path']]), {owner['command']}, heading['start'], heading['end']))
            require(len(commands) == 1 and commands[0]['start'] == heading['start'] and commands[0]['end'] == heading['end'],
                    'section heading is not its complete source command')
            native(destination['native_heading'], index)
            exact(destination['printed_label'], other['printed_number'], 'printed section number differs')
            sections.append({'source_link': n, 'target': other['target'], 'spans': number,
                             'occurrence_spans': whole, 'source_label': row['source_label'],
                             'source_members': [source(destination['source_label'], files)]})
        crosswalk.append({'source_link': n, 'kind': 'reference', 'primary_id': other['id'], 'independent_id': row['id']})
    parsed_cites = [(n, r) for n, r in enumerate(inventory['links']) if r['kind'] == 'citation']
    require(len(parsed_cites) == len(p['citations']) == len(i['citations']), 'omitted citation evidence lost source occurrences')
    for n, parsed in parsed_cites:
        ps = [r for r in p['citations'] if source_identity(r['source_command']) == parsed_identity(parsed)]
        ins = [r for r in i['citations'] if source_identity(r['source']) == parsed_identity(parsed)]
        require(len(ps) == len(ins) == 1, 'citation lacks both original source occurrences')
        a, b = ps[0], ins[0]; source(a['source_command'], files); source(b['source'], files)
        exact(a['target_keys'], parsed['targets'], 'primary citation keys differ')
        exact(b['source_keys'], parsed['targets'], 'independent citation keys differ')
        exact(native(a['native_occurrence'], index), native(b['native'], index), 'citation reading-order occurrence differs')
        points = covered(native(b['native'], index), index)
        require(not native_claimed & points, 'distinct source links reuse a native occurrence')
        native_claimed |= points
        crosswalk.append({'source_link': n, 'kind': 'citation', 'primary_id': a['id'], 'independent_id': b['id']})
    exact(sorted(crosswalk, key=lambda r: r['source_link']), docs['link_crosswalk']['links'], 'complete link crosswalk differs')
    require(len(crosswalk) == len(inventory['links']), 'source link kind is unsupported or omitted')
    return references, sections, deepcopy(external)


def validate_visual(candidate_raw, index_raw, source_raw, raws, verified_images):
    """Pure review-bound validation. It cannot publish or revise original evidence."""
    require(set(raws) == ARTIFACTS and sum(map(len, raws.values())) <= MAX_BUNDLE_BYTES, 'visual tranche evidence inventory/size differs')
    docs = {k: document(v) if k != 'prompt' else v.decode('utf-8') for k, v in raws.items()}
    candidate = document(candidate_raw); index = document(index_raw)['index']; packet = docs['packet']
    require(candidate['index']['sha256'] == sha256(index_raw) and candidate['source_sha256'] == sha256(source_raw), 'visual source/index identity differs')
    exact(candidate['source_inventory_sha256'], sha256(canonical(candidate['source_inventory'])), 'source inventory hash differs')
    for key in ('arxiv_id', 'paper_id', 'index', 'stratum'):
        exact(packet[key], candidate[key], 'blind packet names another frozen candidate')
    require(type(packet['version']) is int and packet['version'] > 0, 'invalid pinned arXiv version')
    for kind in ('pdf', 'source'):
        exact(packet[kind]['sha256'], candidate[kind + '_sha256'], 'blind packet names another corpus artifact')
    pages = [row['number'] for row in index['pages']]
    history(docs, raws, candidate, candidate_raw, pages)
    files, archive_members = read_archive(source_raw)
    require(not any('^^' in text for text in files.values()), 'manual source cannot verify pre-tokenization substitution')
    exact(docs['source_export']['source_sha256'], sha256(source_raw), 'source export names another archive')
    exact(docs['source_export']['text_members'], files, 'source export changed deposited source')
    exact(docs['source_export']['members'], archive_members, 'source export omitted archive members')
    current_inventory = current_source_binding(docs, candidate, files)
    binding_candidate = {**candidate, 'source_inventory': current_inventory}
    verify_native_export(index_raw, raws['native_export'], candidate['paper_id'],
                         {'format': NATIVE_FORMAT, 'sha256': sha256(raws['native_export']),
                          'bytes': len(raws['native_export']), 'index_sha256': sha256(index_raw)})
    exact({row['path']: row['sha256'] for row in packet['images']}, verified_images, 'original image paths/hashes differ')
    exact([row['page'] for row in packet['images']], pages, 'original image order/inventory differs')
    for image, page in zip(packet['images'], index['pages']):
        require(type(image['dpi']) is int and image['dpi'] == 96, 'original image DPI differs from reviewed format')
        for pixel, native_key, declared in [('pixel_width', 'width', 'pdf_width_points'), ('pixel_height', 'height', 'pdf_height_points')]:
            require(type(image[pixel]) is int and image[pixel] > 0 and image[pixel]*72/image['dpi'] == page[native_key]
                    and image[declared] == page[native_key], 'original image dimensions differ from native page')
    for name in ('primary', 'independent'):
        audit_excerpts(docs[name], files, index)
    for name in ('visual_review', 'role_review'):
        review = docs[name]
        exact(review.get('all_original_pages_viewed', review.get('root_full_original_pages_viewed')), pages, 'root review lacks full-page inspection')
        for key in ('primary', 'independent', 'source_export', 'native_export'):
            require(sha256(raws[key]) in review['inputs'].values(), 'root review names different original evidence')
    roles = validate_roles(docs, files)
    objects, associated, pmap, imap, counts = validate_objects(docs, binding_candidate, files, index)
    expected = [{'primary_id': pid, 'independent_id': next(iid for iid, target in imap.items() if target == sid), 'source_id': sid}
                for pid, sid in pmap.items()]
    exact(sorted(expected, key=lambda r: r['source_id']), docs['object_crosswalk']['objects'], 'object crosswalk omits or changes original ownership')
    exact(docs['object_crosswalk']['source_inventory_sha256'], candidate['source_inventory_sha256'], 'object crosswalk changes source inventory')
    refs, sections, external = validate_links(docs, binding_candidate, files, index, pmap, imap)
    review = docs['review']
    require(review.get('format') == REVIEW_FORMAT and review.get('verdict') == 'clear_complete_visual_negative_inventory'
            and review.get('findings') == [], 'independent construction review is not clear')
    exact(review['artifacts'], {k: sha256(v) for k, v in raws.items() if k != 'review'}, 'construction review changes its evidence closure')
    exact(review['candidate_sha256'], sha256(candidate_raw), 'construction review changes automatic candidate')
    exact(review['pages_inspected'], pages, 'construction reviewer omitted original pages')
    exact(review['complete_kind_counts'], {**{k: 0 for k in ('equation', 'statement', 'proof', 'algorithm')}, **counts}, 'construction kind counts differ')
    exact(review['reference_counts'], {'local': len(refs) + len(sections), 'visual': len(refs), 'section': len(sections), 'external': len(external), 'O4': 0}, 'construction reference counts differ')
    identities = review['annotators']; require(set(identities) == {'primary', 'independent'}
        and all(isinstance(v, str) and v.strip() for v in identities.values())
        and len(set(identities.values())) == 2 and isinstance(review.get('reviewer'), str)
        and review['reviewer'].strip() and review['reviewer'] not in identities.values(), 'construction review lacks independent identities')
    require(review['later_crosswalk_reviewed_after_annotation_freeze'] is True, 'post-freeze review order is absent')
    common = {'evidence_format': FORMAT, 'evidence_hashes': {k: sha256(v) for k, v in raws.items()},
              'automatic_candidate_retained': True, 'final_k1_publication': False,
              'omitted_metrics': {'O8': 'bibliography codec not supplied', 'O9': 'bibliography codec not supplied',
                                  'O10': 'bibliography codec not supplied', 'O11': 'no independent ranking panel'},
              'retained_role_candidates': roles, 'source_diagnostics_retained': deepcopy(candidate['source_inventory']['coverage'])}
    common['current_source_inventory_sha256'] = sha256(canonical(current_inventory))
    common['current_source_diagnostics'] = deepcopy(current_inventory['coverage'])
    common['historical_source_difference_fields'] = docs['construction']['changed_source_inventory_fields']
    result = deepcopy(candidate)
    result['manual_figure_table_overlay'] = {**deepcopy(common), 'objects': objects, 'metric_eligibility': {'O1': True, 'O2': True}}
    result['manual_object_overlay'] = {**common, 'objects': [], 'references': [], 'other_object_references': refs,
        'non_object_references': sections, 'associated_content': associated + [{'kind': 'external_reference', 'evidence': row} for row in external],
        'reviewed_absent_kinds': [k for k in ('figure', 'table') if counts.get(k, 0) == 0],
        'metric_eligibility': {k: True for k in ('O3', 'O4', 'O5', 'O6', 'O7')}}
    return result


def attach_visual(cache, corpus_root, data_root, candidate_raw, paper, mapped, relative):
    """Bind only declared cache evidence and the original frozen corpus identities."""
    from builder import fingerprint_file, safe_file
    from manual import bounded
    manifest_raw = bounded(cache, relative + '/manifest.json'); manifest = document(manifest_raw)
    require(manifest.get('format') == FORMAT and type(manifest.get('schema_version')) is int
            and manifest['schema_version'] == 1 and set(manifest['artifacts']) == ARTIFACTS, 'unsupported visual tranche manifest')
    raws = {}; total = 0
    for key, row in manifest['artifacts'].items():
        require(set(row) == {'path', 'sha256', 'bytes'}, 'unsupported visual evidence declaration')
        raw = bounded(cache, row['path']); total += len(raw)
        require(total <= MAX_BUNDLE_BYTES, 'visual evidence exceeds its cumulative byte bound')
        hash_ref(row, raw, 'visual evidence hash/size differs'); raws[key] = raw
    docs = {k: document(v) for k, v in raws.items() if k != 'prompt'}
    candidate = document(candidate_raw); packet = docs['packet']
    exact(candidate['index'], mapped['index'], 'visual candidate names another mapped index')
    exact(candidate['paper_id'], mapped['paper_id'], 'visual candidate names another mapped paper')
    exact(candidate['arxiv_id'], paper['arxiv_id'], 'visual candidate changes arXiv identity')
    exact(packet['version'], paper['version'], 'visual packet changes source version')
    exact(packet['stratum'], paper['stratum'], 'visual packet changes stratum')
    index_raw = bounded(data_root, mapped['index']['path'])
    require(sha256(index_raw) == mapped['index']['sha256'] and mapped['pdf_sha256'] == paper['pdf']['sha256'], 'mapped PDF/index bytes differ')
    source_raw = None
    for kind in ('pdf', 'source'):
        path = safe_file(corpus_root, paper[kind]['path']); digest, size = fingerprint_file(path, cap=64*1024*1024)
        require(digest == paper[kind]['sha256'] == candidate[kind + '_sha256'] and size == paper[kind]['bytes'], 'frozen corpus artifact changed')
        exact(packet[kind], paper[kind], 'blind packet changes corpus receipt')
        if kind == 'source':
            source_raw = path.read_bytes(); require(len(source_raw) == size and sha256(source_raw) == digest, 'source changed during read')
    annotation_root = safe_file(cache, manifest['artifacts']['policy']['path']).parent
    original_rows = docs['primary']['inputs'] + docs['independent_receipt']['inputs']
    original_rows += [docs['primary_receipt']['inventory']]
    original_rows += [dict(path=path, sha256=digest) for key in ('visual_review', 'role_review') for path, digest in docs[key]['inputs'].items()]
    original_rows += [dict(path=path, **binding) for path, binding in docs['initial_review']['bindings'].items()]
    for row in original_rows:
        verify_original_file(cache, annotation_root, row)
    images = {row['path']: verify_original_file(cache, annotation_root, row) for row in packet['images']}
    exact(packet['selection_sha256'], sha256(raws['selection']), 'visual tranche changes frozen selection')
    exact(packet['policy_sha256'], sha256(raws['policy']), 'visual tranche changes frozen policy')
    selected = [r for r in docs['selection']['selected'] if r['arxiv_id'] == paper['arxiv_id']]
    require(len(selected) == 1, 'visual paper absent or repeated in frozen selection')
    exact(selected[0]['mapping'], mapped, 'visual selection names another mapping')
    result = validate_visual(candidate_raw, index_raw, source_raw, raws, images)
    for key, row in manifest['artifacts'].items():
        require(bounded(cache, row['path']) == raws[key], 'visual evidence changed during validation')
    for row in original_rows + packet['images']:
        verify_original_file(cache, annotation_root, row)
    require(bounded(cache, relative + '/manifest.json') == manifest_raw, 'visual manifest changed during validation')
    result['manual_assembly'] = {'format': FORMAT, 'manifest_sha256': sha256(manifest_raw), 'candidate_sha256': sha256(candidate_raw),
        'final_k1_publication': False, 'omitted_manual_kinds': ['bib_entry'], 'omitted_metrics': ['O8', 'O9', 'O10', 'O11'],
        'source_inventory_policy': 'automatic exclusions retained; complete visual/formal-negative inventories separately reviewed'}
    return result
