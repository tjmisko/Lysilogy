"""Complete, independently reconciled bibliography labels; no detector predictions."""
from copy import deepcopy
from collections import Counter
import re

from annotations import canonical, document, require
from archive import read_archive, sha256
from object_annotations import checked_members, checked_span, checked_root_span, member_identity

FIELDS = ('first_author', 'title', 'year')


def unique(rows, key, expected, message):
    mapped = {row[key]: row for row in rows}
    require(len(rows) == len(mapped) and set(mapped) == set(expected), message)
    return mapped


def apply_bibliography_overlay(candidate_raw, index_raw, source_raw, original_packet_raw,
                               packet_raw, root_raw, independent_raw, receipt_raw,
                               comparison_raw, verified_images):
    candidate, wrapper, original, packet, root, independent, receipt, comparison = [document(raw) for raw in (candidate_raw, index_raw, original_packet_raw, packet_raw, root_raw, independent_raw, receipt_raw, comparison_raw)]
    require(comparison.get('schema_version') == 1 and comparison.get('verdict') == 'clear_complete_bibliography_overlay' and comparison.get('findings') == [], 'bibliography comparison is not independently cleared')
    for key, raw in [('root_annotation_sha256', root_raw), ('independent_annotation_sha256', independent_raw), ('independent_receipt_sha256', receipt_raw), ('bibliography_packet_sha256', packet_raw)]:
        require(comparison.get(key) == sha256(raw), 'bibliography comparison binds other input bytes')
    require(root['annotator'] != independent['annotator'] and root['annotator'] and independent['annotator'], 'bibliography annotators must be distinct')
    require(root['attestation']['other_annotator_labels_read'] is False and root['attestation']['detector_outputs_read'] is False and all(independent['independence'][key] is False for key in ('bibliography_root_labels_read', 'root_bibliography_code_read', 'detector_outputs_read', 'provider_data_read')), 'bibliography annotation is not independently blind')
    require(receipt['annotation']['sha256'] == sha256(independent_raw) and receipt['annotation']['bytes'] == len(independent_raw) and receipt['packet_sha256'] == sha256(packet_raw), 'bibliography execution receipt differs')
    require(root['original_packet_sha256'] == packet['original_packet_sha256'] == sha256(original_packet_raw), 'bibliography original packet differs')
    for key in ('arxiv_id', 'paper_id'):
        require(root[key] == independent[key] == packet[key] == original[key] == comparison[key] == candidate[key], 'bibliography paper identity differs')
    for key in ('pdf_sha256', 'source_sha256', 'index', 'source_inventory_sha256'):
        require(root[key] == candidate[key], 'bibliography root provenance differs')
    require(sha256(index_raw) == candidate['index']['sha256'] and sha256(source_raw) == candidate['source_sha256'], 'bibliography actual source/index changed')
    require(root['candidate_sha256'] == packet['candidate_sha256'] == original['candidate_sha256'] == sha256(candidate_raw), 'bibliography candidate differs')
    require(root['bibliography_packet_sha256'] == sha256(packet_raw), 'bibliography packet differs')
    require(packet['inventory_sha256'] == original['inventory_sha256'] == candidate['source_inventory_sha256'] == sha256(canonical(candidate['source_inventory'])), 'bibliography source inventory differs')
    require(packet['index'] == original['index'] == candidate['index'] and packet['pdf']['sha256'] == candidate['pdf_sha256'] and packet['source']['sha256'] == candidate['source_sha256'], 'bibliography packet artifacts differ')
    inputs = independent['inputs']
    require(inputs['packet']['sha256'] == sha256(packet_raw) and inputs['candidate_sha256_from_packet'] == sha256(candidate_raw) and inputs['inventory_sha256_from_packet'] == candidate['source_inventory_sha256'], 'independent bibliography used another packet/inventory')
    require(inputs['pdf']['sha256'] == candidate['pdf_sha256'] and inputs['source']['sha256'] == candidate['source_sha256'] and inputs['reading_index']['sha256'] == sha256(index_raw), 'independent bibliography artifact hashes differ')
    files, members = read_archive(source_raw)
    require(inputs['source_members'] == members, 'bibliography archive member inventory differs')
    index = wrapper['index']; text = index['text']; pages = {row['number']: row for row in index['pages']}
    require(packet['images'] == original['images'] == root['page_images'] and len(packet['images']) == len(pages), 'bibliography original page inventory differs')
    require(len(inputs['images']) == len(pages) and len({row['path'] for row in inputs['images']}) == len(pages), 'independent bibliography page images are incomplete')
    require(Counter(row['sha256'] for row in inputs['images']) == Counter(row['sha256'] for row in packet['images']), 'independent bibliography used different original renders')
    images = packet['images'] + inputs['images'] + [root['bibliography_detail_image']] + receipt['detail_images']
    require(set(verified_images) == {row['path'] for row in images} and all(verified_images[row['path']] == row['sha256'] for row in images), 'bibliography images differ from actual file paths')
    require(receipt['detail_render']['viewed_both'] is True and receipt['detail_render']['source_pdf_sha256'] == candidate['pdf_sha256'], 'bibliography detail review is incomplete')
    parsed = candidate['source_inventory']; sources = {row['id']: row for row in parsed['entries']}
    require(packet['entries'] == [{'id': row['id'], 'source_members': row['source_members']} for row in parsed['entries']], 'bibliography packet omits source entries')
    citations = {number: row for number, row in enumerate(parsed['links']) if row['kind'] == 'citation'}
    require(packet['citations'] == [{'source_link': number, 'command': row['command'], 'source_members': row['source_members'], 'targets': row['targets']} for number, row in citations.items()], 'bibliography packet omits source citation occurrences')
    rows = unique(independent['entries'], 'id', sources, 'independent bibliography inventory is incomplete')
    roots = unique(root['entries'], 'id', sources, 'root bibliography inventory is incomplete')
    fields_review = unique(comparison['field_values_and_anchors_verified'], 'id', sources, 'reviewed bibliography fields are incomplete')
    require(len(comparison['entries_verified']) == len(sources) and set(comparison['entries_verified']) == set(sources), 'reviewed entry inventory is incomplete')
    entries, known = [], Counter()
    owned = []
    for identifier, row in rows.items():
        source, prior = sources[identifier], roots[identifier]
        provenance = checked_members(row['source_members'], files, source['source_members'])
        checked_members(prior['source_members'], files, source['source_members'])
        spans = [checked_span(span, text) for span in row['members']]
        require(spans and spans == [checked_root_span(span, text) for span in prior['spans']], 'complete bibliography membership differs between annotators')
        require(row['page'] in pages and all(pages[row['page']]['start'] <= span['start'] < span['end'] <= pages[row['page']]['end'] for span in spans), 'bibliography members lie outside their page')
        owned.extend((span['start'], span['end']) for span in spans)
        require(row['printed_key'] == prior['printed_number'] and re.fullmatch(r'[1-9][0-9]*', row['printed_key']), 'bibliography printed numbering disagrees')
        labels, fields = {}, {}
        require(set(row['fields']) == set(prior['fields']) == set(fields_review[identifier]['fields']) == set(FIELDS), 'bibliography requested field roles are incomplete')
        for name in FIELDS:
            field, previous, reviewed = row['fields'][name], prior['fields'][name], fields_review[identifier]['fields'][name]
            value = field['value']
            require(field['status'] == 'known' and isinstance(value, str) and value.strip() and value == previous['value'] == reviewed['value'] == row['field_labels'][name], 'bibliography field value is unreviewed or conflicting')
            if name == 'year': require(re.fullmatch(r'[12][0-9]{3}[a-z]?', value), 'bibliography year is not an explicit printed year')
            native = checked_span(field['index_span'], text)
            require(native == checked_root_span(previous['printed_span'], text) and all(reviewed['span'][key] == native[key] for key in ('start', 'end')) and reviewed['native_text_sha256'] == native['native_text_sha256'], 'bibliography field native anchors disagree')
            require(any(span['start'] <= native['start'] < native['end'] <= span['end'] for span in spans), 'bibliography field lies outside its owning entry')
            source_field = checked_members([field['source_member']], files)[0]
            require(any(member['path'] == source_field['path'] and member['start'] <= source_field['start'] < source_field['end'] <= member['end'] for member in provenance), 'bibliography field source lies outside its owning entry')
            payload = files[source_field['path']][source_field['start']:source_field['end']]
            # Source member bounds may include balanced presentation braces; the
            # independently read payload is a role-delimited substring, never a
            # field inferred from arbitrary bibliography prose.
            require(field['source_payload'] in payload and field['source_payload'] == previous['source_role']['value'] and reviewed['source_payload_sha256'] == sha256(payload.encode()), 'bibliography source-role payload contradicts reviewed bytes')
            role = previous['source_role']; require(role['source_command_start'] <= source_field['start'] < source_field['end'] <= role['source_command_end'], 'bibliography field is outside its reviewed source command')
            require(any(member['start'] <= role['source_command_start'] < role['source_command_end'] <= member['end'] for member in provenance if member['path'] == source_field['path']), 'bibliography source command is outside its entry')
            allowed = {'first_author': {'bibfield{author}/bibinfo{person}', 'bibinfo{person}'}, 'title': {'showarticletitle', 'bibinfo{title}', 'bibinfo{booktitle}'}, 'year': {'bibinfo{year}'}}
            require(role['command'] in allowed[name] and field['source_role'] == '\\' + role['command'].split('/')[-1], 'bibliography field uses an incompatible source role')
            command_text = files[source_field['path']][role['source_command_start']:role['source_command_end']]
            require(command_text.startswith('\\' + role['command'].split('/')[0]) and '\\' + role['command'].split('/')[-1] in command_text, 'bibliography declared role differs from actual source command')
            require(reviewed['disposition'].startswith('accepted;'), 'bibliography field review is not accepted')
            labels[name] = value; known[name] += 1
            fields[name] = {'value': value, 'native_span': native, 'source_member': source_field, 'source_role': field['source_role'], 'root_source_role': role}
        entries.append({'id': identifier, 'printed_key': row['printed_key'], 'spans': spans, 'field_labels': labels, 'field_provenance': fields, 'source_members': provenance})
    owned.sort(); require(all(left[1] <= right[0] for left, right in zip(owned, owned[1:])), 'bibliography entries overlap')
    require(len({row['printed_key'] for row in entries}) == len(entries), 'bibliography printed keys are duplicated')
    independently = unique(independent['citations'], 'source_link', citations, 'independent citation inventory is incomplete')
    root_cites = unique(root['citations'], 'source_link', citations, 'root citation inventory is incomplete')
    reviews = unique(comparison['citation_occurrences_verified'], 'source_link', citations, 'reviewed citation inventory is incomplete')
    require(len(comparison['citation_source_links_verified']) == len(citations) and set(comparison['citation_source_links_verified']) == set(citations), 'comparison source citation inventory differs')
    groups, mentions, seen = [], [], set()
    key_targets = {int(row['printed_key']): row['id'] for row in entries}
    for number, row in independently.items():
        source, previous, reviewed = citations[number], root_cites[number], reviews[number]
        provenance = checked_members(row['source_members'], files, source['source_members'])
        checked_members(previous['source_members'], files, source['source_members'])
        require(row['command'] == source['command'] and row['source_target_order'] == source['targets'] and set(row['targets']) == set(source['targets']) and len(row['targets']) == len(set(row['targets'])), 'citation target inventory/order differs from deposited source')
        require(all(target in sources for target in row['targets']) and row['targets'] == previous['targets'] == reviewed['targets'], 'citation reviewed target identity differs')
        native = checked_span(row['span'], text)
        require([native] == [checked_root_span(span, text) for span in previous['spans']], 'citation printed membership differs between annotators')
        require(all(reviewed['span'][key] == native[key] for key in ('start', 'end')) and reviewed['printed'] == row['span']['text'] == previous['printed_marker'] and reviewed['disposition'] == 'accepted' and reviewed['location_role'] == row['location_role'], 'citation comparison contradicts its printed occurrence')
        marker = row['span']['text']; require(re.fullmatch(r'\[\d+(?:,\s*\d+)*\]', marker), 'unsupported manual citation marker syntax')
        numbers = [int(value) for value in re.findall(r'\d+', marker)]
        require(numbers == row['printed_numbers'] and [key_targets.get(value) for value in numbers] == row['targets'], 'printed citation numbering differs from source target mapping')
        position = (native['start'], native['end']); require(position not in seen, 'two source citations claim the same printed occurrence'); seen.add(position)
        groups.append({'source_link': number, **native, 'targets': row['targets'], 'source_target_order': row['source_target_order'], 'source_members': provenance, 'location_role': row['location_role']})
        mentions.extend({'source_link': number, 'start': native['start'], 'end': native['end'], 'target': target} for target in row['targets'])
    pairs = Counter((row['source_link'], row['target']) for row in mentions)
    require(pairs == Counter((row['source_link'], row['target']) for row in comparison['citation_target_pairs_verified']), 'reviewed citation pairs omit or duplicate targets')
    count = len(entries); group_count = len(groups); pair_count = len(mentions)
    expected_counts = {'entries': count, 'known_field_values': sum(known.values()), 'exact_native_field_anchors': sum(known.values()), 'citation_groups': group_count, 'citation_target_pairs': pair_count, 'unknown_requested_fields': 0, 'disagreements': 0}
    require(comparison['counts'] == expected_counts, 'bibliography comparison counts contradict complete labels')
    require(root['complete_inventory'] == {'bibliography_entries': count, 'citation_commands': group_count, 'citation_target_pairs': pair_count, **{name + '_labels': known[name] for name in FIELDS}}, 'root bibliography complete counts differ')
    inventory = independent['complete_inventory']
    require(inventory['coverage_complete_for_this_paper'] is True and inventory['bibliography_entries'] == count and inventory['citation_groups'] == group_count and inventory['citation_target_mentions'] == pair_count and inventory['known_fields'] == dict(known) and inventory['unknown_fields'] == {name: 0 for name in FIELDS}, 'independent bibliography complete counts differ')
    overlay = {'entries': entries, 'mentions': mentions, 'citation_groups': groups, 'counts': expected_counts, 'metric_eligibility': {'O8': True, 'O9': True, 'O10': True}, 'alignment': {'quality': 1.0, 'method': 'complete independent source/PDF visual transcription, two annotators and accepted reconciliation'}, 'final_k1_publication': False, 'automatic_candidate_retained': True, 'verified_image_paths': verified_images, 'evidence_hashes': {'candidate_sha256': sha256(candidate_raw), 'source_inventory_sha256': candidate['source_inventory_sha256'], 'packet_sha256': sha256(packet_raw), 'original_packet_sha256': sha256(original_packet_raw), 'root_annotation_sha256': sha256(root_raw), 'independent_annotation_sha256': sha256(independent_raw), 'independent_receipt_sha256': sha256(receipt_raw), 'comparison_sha256': sha256(comparison_raw)}}
    result = deepcopy(candidate); result['manual_bibliography_overlay'] = overlay
    return result
