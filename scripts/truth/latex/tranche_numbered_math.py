"""Complete numbered mathematics only, with every other original role retained.

The two blind inventories and later judgments remain separate immutable inputs.
This codec never promotes automatic source confidence or supplies O4--O11 truth.
"""
from copy import deepcopy
import importlib
import math
from pathlib import Path

from annotations import canonical, document, require
from archive import read_archive, sha256
from native_exports import FORMAT as NATIVE_FORMAT, verify_native_export
from parser import parse_project, reference_names_verified
from tex import COMMAND, comments, group
from tranche import covered, exact, hash_ref, member_pages, native_members, rectangle
from tranche_visual_only import pointer, retained_original, retained_source

FORMAT = 'k1-manual-numbered-math-tranche-v1'
REVIEW_FORMAT = 'k1-manual-numbered-math-construction-review-v1'
OMITTED_METRICS = [f'O{n}' for n in range(1, 12) if n != 3]
ARTIFACTS = {'proposal', 'construction', 'review', 'packet', 'policy', 'selection',
             'prompt', 'assignment', 'checkpoint', 'producer_history', 'dispositions',
             'matrix', 'source_scope', 'source_scope_review', 'endpoint_addendum', 'proposal_review'}
MAX_BUNDLE_BYTES = 64 * 1024 * 1024
COORDS = ('x_min', 'y_min', 'x_max', 'y_max')
OVERLAYS = ('manual_figure_table_overlay', 'manual_object_overlay',
            'manual_visual_only_overlay', 'manual_numbered_math_overlay',
            'manual_bibliography_overlay', 'independent_panel')
PRODUCER_MODULES = {'align.py', 'annotations.py', 'archive.py', 'bibliography_annotations.py',
    'builder.py', 'corpus.py', 'manual.py', 'object_annotations.py', 'panel.py', 'parser.py',
    'release.py', 'tex.py', 'native_exports.py', 'tranche.py', 'tranche_visual.py',
    'tranche_visual_only.py', 'release_layout.py', 'release_process.py', 'release_replay.py',
    'tranche_numbered_math.py'}


def producer_modules():
    """Hash the fixed modules actually loaded by current or retained publication."""
    result = {}
    for name in sorted(PRODUCER_MODULES):
        path = Path(importlib.import_module(name[:-3]).__file__)
        require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 1024 * 1024,
                'numbered producer module is not a bounded source file')
        result[name] = sha256(path.read_bytes())
    return result


def receipt_inputs(receipt):
    values = receipt.get('inputs', receipt.get('input_bindings'))
    require(isinstance(values, (dict, list)), 'original input ledger is absent')
    if isinstance(values, dict):
        metadata = {'source_member_inventory', 'selection_sha256', 'policy_sha256'}
        require(set(values) <= metadata | {'assignment', 'dispatch', 'prompt', 'rendered_packet',
                'source_export', 'native_export', 'images', 'pdf_identity_from_packet',
                'source_archive_identity_from_packet', 'original_index_identity_from_packet_not_opened'},
                'unsupported original input ledger role')
        values = [value for key, value in values.items() if key not in metadata]
    rows = []; visited = 0
    def visit(value, depth):
        nonlocal visited
        visited += 1
        require(depth <= 8 and visited <= 1000, 'original input ledger exceeds its bound')
        if isinstance(value, dict) and {'path', 'sha256', 'bytes'} <= value.keys():
            rows.append(value)
        elif isinstance(value, (dict, list)):
            for child in value.values() if isinstance(value, dict) else value:
                visit(child, depth + 1)
        else:
            raise ValueError('unsupported original input ledger field')
    visit(values, 0)
    return rows


def original_source(row, files, members, *, primary):
    value = row['source']
    path, start, end = value['member'], value['start'], value['end']
    require(path in files and type(start) is int and type(end) is int
            and 0 <= start < end <= len(files[path]), 'numbered source extent is invalid')
    raw = files[path][start:end].encode('utf-8')
    exact(value['text'], files[path][start:end], 'numbered original source excerpt differs')
    # These original protocols deliberately use different hash domains.
    expected = sha256(raw) if primary else members[path]['sha256']
    exact(value['sha256'], expected, 'numbered original source hash domain differs')
    return {'path': path, 'start': start, 'end': end}


def body_members(row, *, primary):
    keys = ('body_native_members', 'native_body_members') if primary else (
        'native_expression_members', 'native_body_members')
    present = [row[key] for key in keys if key in row]
    require(len(present) == 1, 'original numbered body is missing or ambiguous')
    return present[0]


def number_members(row, *, primary):
    if not primary:
        return [row['native_number']]
    keys = ('printed_number_native_members', 'native_number_members')
    present = [row[key] for key in keys if key in row]
    require(len(present) == 1, 'original printed number is missing or ambiguous')
    return present[0]


def original_native(rows, index):
    result = native_members(rows, index)
    for original, selected in zip(rows, result):
        if 'sha256' in original:
            exact(original['sha256'], selected['native_text_sha256'], 'original native hash differs')
        if 'pages' in original:
            exact(original['pages'], sorted(member_pages([selected], index)), 'original native page list differs')
    return result


def record(evidence, claim):
    require(isinstance(claim, dict) and set(claim) == {'artifact', 'pointer', 'record_sha256'},
            'numbered evidence reference must select an exact original record')
    require(claim['artifact'] in evidence, 'numbered evidence artifact is absent')
    value = pointer(evidence[claim['artifact']], claim['pointer'])
    exact(sha256(canonical(value)), claim['record_sha256'], 'numbered evidence record differs')
    return value


def retention(primary, independent, inventory, claims):
    originals = {}
    for side, original in [('primary', primary), ('independent', independent)]:
        ledger = retained_original(original, {row[side]['pointer'] for row in claims})
        for collection in ledger:
            for row in collection['records']:
                if row['disposition'] == 'scored_visual':
                    row['disposition'] = 'scored_numbered_equation'
        originals[side] = ledger
    source = retained_source(inventory, {row['proposed_id'] for row in claims}, {})
    for row in source['objects']:
        if row['disposition'] == 'scored_visual':
                row['disposition'] = 'scored_numbered_equation'
    endpoints = []; object_ids = {row['id'] for row in inventory['objects']}
    entries = {row['id']: n for n, row in enumerate(inventory['entries'])}
    require(len(object_ids) == len(inventory['objects']) and len(entries) == len(inventory['entries']),
            'retained source owner identities repeat')
    for label, owner in inventory['label_targets'].items():
        require(owner in object_ids, 'source label names an absent retained owner')
    for n, link in enumerate(inventory['links']):
        for target in link['targets']:
            owner = entries.get(target) if link['kind'] == 'citation' else inventory['label_targets'].get(target)
            require(owner is not None, 'source link names an absent retained owner')
            endpoints.append({'source_link_ordinal': n, 'target': target,
                'owner': '/entries/' + str(owner) if link['kind'] == 'citation' else owner,
                'disposition': 'retained_unscored_source_endpoint'})
    return {'primary': originals['primary'], 'independent': originals['independent'], 'source': source,
            'source_endpoints': endpoints,
            'source_metadata': {key: deepcopy(value) for key, value in inventory.items()
                                if key not in ('objects', 'links', 'entries')}}


def source_scope(inventory, historical, files, construction):
    """Retain warnings and bind a separate finite source-scope judgment.

    A source/printed expression judgment is not this inventory judgment. Unknown
    dynamic controls and label ambiguity have no override in this first codec.
    Other diagnostics stay exact and need the distinct construction review.
    """
    exact(construction['current_source_inventory'], inventory, 'current numbered source differs')
    exact(construction['current_source_inventory_sha256'], sha256(canonical(inventory)),
          'current numbered source hash differs')
    changed = sorted(key for key in set(inventory) | set(historical)
                     if canonical(inventory.get(key)) != canonical(historical.get(key)))
    exact(construction['changed_source_inventory_fields'], changed, 'source changes are concealed')
    require(not changed, 'this fixed-source codec requires the entire historical inventory unchanged')
    for kind in ('objects', 'links'):
        fields = ('id', 'kind', 'source_members', 'labels', 'targets')
        project = lambda value: [{k: row[k] for k in fields if k in row} for row in value[kind]]
        exact(project(inventory), project(historical), 'current source occurrence identities changed')
    require(not any('^^' in text for text in files.values()), 'unsupported lexical substitution')
    coverage = inventory['coverage']
    for key in ('raw_lexical_substitutions', 'unparsed_source_roles', 'unverified_stored_arguments'):
        require(not coverage.get(key), 'unsupported hidden numbered inventory: ' + key)
    require(not coverage.get('unsupported_kind_inventory'), 'unsupported numbered kind inventory')
    require(not inventory.get('ambiguous_labels') and not inventory.get('unverified_label_names'),
            'numbered source has ambiguous or unverified label ownership')
    forbidden = ('conditional', 'catcode', 'csname', 'input', 'include', 'counter', 'definition', 'token')
    semantics = coverage.get('unsupported_source_semantics', {})
    require(not any(any(word in key.lower() for word in forbidden) for key in semantics),
            'unsupported dynamic inventory semantics cannot be reviewed away')
    scope = construction['source_scope']
    require(scope.get('basis') == 'complete_original_numbered_inventory_and_finite_source_scope'
            and scope.get('automatic_guards_unchanged') is True,
            'numbered construction lacks a separate source-inventory basis')
    exact(scope['retained_current_coverage'], coverage, 'source scope drops current warnings')
    require(isinstance(scope['observations'], list) and 1 <= len(scope['observations']) <= 128,
            'finite source-scope observations are absent or unbounded')
    for observation in scope['observations']:
        require(set(observation) == {'source_members', 'disposition', 'reason'}
                and observation['disposition'] in ('numbered_inventory', 'retained_other_role', 'local_style_scope')
                and isinstance(observation['reason'], str) and observation['reason'].strip(),
                'source-scope observation is unsupported')
        require(isinstance(observation['source_members'], list)
                and 1 <= len(observation['source_members']) <= 128, 'source scope lacks exact occurrences')
        for value in observation['source_members']:
            path, start, end = value['path'], value['start'], value['end']
            require(path in files and type(start) is int and type(end) is int
                    and 0 <= start < end <= len(files[path]), 'source scope extent differs')
            exact(value['text_sha256'], sha256(files[path][start:end].encode()), 'source scope text differs')
    return changed


def reviewed_source_scope(pid, docs, raws, files, members, inventory, source_raw):
    """Replay a distinct judgment of these whole bytes, never a command whitelist."""
    packet, review = docs['source_scope'], docs['source_scope_review']
    hash_ref(review['packet'], raws['source_scope'], 'finite source-scope review changes its packet')
    require(review['verdict'] == 'clear_finite_source_scope_for_exact_two_frozen_inputs'
            and review['findings'] == [] and review['truth_admission'] is False
            and review['metric_eligibility_granted'] == []
            and review['reviewer'] != packet['producer'], 'finite source scope lacks distinct clearance')
    rows = [row for row in packet['papers'] if row['arxiv_id'] == pid]
    checked = [row for row in review['rows'] if row['arxiv_id'] == pid]
    require(len(rows) == len(checked) == 1, 'finite source scope omits or repeats paper')
    row, checked = rows[0], checked[0]
    exact(row['member_inventory'], members, 'finite source scope changes a deposited member or encoding')
    digest = sha256(source_raw)
    require(digest in {binding['sha256'] for binding in packet['bindings']}
            and digest in {binding['sha256'] for binding in review['bindings']},
            'finite source scope does not bind the whole current archive')
    exact(row['current_source_inventory_sha256'], sha256(canonical(inventory)), 'source scope metadata changed')
    exact(checked['current_source_sha256'], row['current_source_inventory_sha256'], 'review changes current source')
    exact(docs['construction']['source_scope'], row['source_scope'], 'construction substitutes source scope')
    exact(checked['automatic_metric_flags_unchanged'], {f'O{n}': False for n in range(1, 12)},
          'source review changes automatic eligibility')
    guards = inventory['coverage']['unsupported_source_semantics']
    exact({item['diagnostic']: item['original_count'] for item in row['guard_literal_inventory']}, guards,
          'finite guard inventory drops or changes a diagnostic')
    require(set(checked['guard_judgments']) == set(guards)
            and all(isinstance(value, str) and value.strip() for value in checked['guard_judgments'].values()),
            'every literal guard needs its own finite reason')
    # The reviewer selects every recorded source span, including complete style
    # definitions and forwarding arguments. The entire member inventory above
    # also prevents unrecorded inserted content from inheriting this review.
    observed = []
    def visit(value, path, depth=0):
        require(depth <= 16, 'finite source-scope tree exceeds its bound')
        if isinstance(value, dict):
            if {'path', 'start', 'end', 'text_sha256'} <= value.keys():
                member, start, end = value['path'], value['start'], value['end']
                require(member in files and type(start) is int and type(end) is int
                        and 0 <= start < end <= len(files[member]), 'reviewed source span is invalid')
                text = files[member][start:end]
                exact(value['text_sha256'], sha256(text.encode()), 'reviewed source span bytes changed')
                if 'text' in value:
                    exact(value['text'], text, 'reviewed source span literal changed')
                observed.append({'pointer': path, 'record_sha256': sha256(canonical(value))})
                return
            for key, child in value.items():
                visit(child, path + '/' + key.replace('~', '~0').replace('/', '~1'), depth + 1)
        elif isinstance(value, list):
            for number, child in enumerate(value):
                visit(child, path + '/' + str(number), depth + 1)
    visit(row, '')
    exact(checked['source_spans_verified'], observed, 'source review omits a definition, invocation or argument')
    for guard in row['guard_literal_inventory']:
        kind, name = guard['diagnostic'].split(':', 1)
        if kind == 'uninterpreted_local_style':
            expected = [(name, 0, len(files[name]))]
        else:
            expected = [(path, match.start(), match.end()) for path, text in sorted(files.items())
                        if Path(path).suffix in ('.tex', '.sty', '.bbl')
                        for match in COMMAND.finditer(comments(text)) if match[1].rstrip('*') == name]
        actual = [(item['source']['path'], item['source']['start'], item['source']['end'])
                  for item in guard['literal_occurrences']]
        exact(sorted(actual), sorted(expected), 'finite source scope omits a guard literal occurrence')
    for argument in checked['actual_complete_arguments']:
        text = files[argument['path']]; scan = comments(text); at = argument['after_command']
        require(text[:at].endswith('\\' + argument['command']), 'reviewed argument changes its invocation')
        optional, at = group(scan, at, '[', ']', False)
        exact(optional, argument['optional_argument'], 'reviewed optional argument changed')
        for value in argument['arguments']:
            _, end = group(scan, at)
            require(end == value['end'] and text[value['start']:value['start'] + 1] == '{'
                    and not text[at:value['start']].strip(), 'reviewed argument is not complete')
            exact(text[value['start']:end], value['text'], 'reviewed actual argument changed')
            exact(sha256(value['text'].encode()), value['sha256'], 'reviewed actual argument hash differs')
            at = end
        exact(at, argument['end'], 'reviewed invocation omits an actual argument')


def original_endpoints(primary, independent, addendum, pid):
    """Retain original namespaces and explicitly record an absent unscored owner."""
    output = []
    scalar = {'parent', 'parent_id', 'parent_object', 'source_parent_object', 'target_id', 'destination_id', 'proof_target'}
    multiple = {'children', 'child_ids', 'child_objects', 'source_child_objects', 'targets', 'target_entry_ids'}
    for side, original in [('primary', primary), ('independent', independent)]:
        owners = {}
        for name, rows in original.items():
            if not isinstance(rows, list):
                continue
            for n, row in enumerate(rows):
                if isinstance(row, dict) and isinstance(row.get('id'), str):
                    require(row['id'] not in owners, 'original owner identity repeats within its namespace')
                    owners[row['id']] = '/' + name + '/' + str(n)
        def visit(row, path, record_path, depth=0):
            require(depth <= 16, 'original endpoint tree exceeds its bound')
            if isinstance(row, dict):
                for key, value in row.items():
                    values = [value] if key in scalar else value if key in multiple else None
                    if values is not None:
                        require(isinstance(values, list), 'original endpoint list is malformed')
                        for target in values:
                            if target is None:
                                continue
                            require(isinstance(target, str) and target, 'original endpoint is not an identity')
                            result = {'namespace': side, 'pointer': path, 'field': key, 'target': target,
                                      'owner_pointer': owners.get(target), 'disposition': 'retained_unscored_endpoint'}
                            if target not in owners:
                                require('original_endpoint' in addendum, 'undeclared dangling original endpoint')
                                exception = addendum['original_endpoint']
                                require(pid == '1911.08525' and side == exception['artifact']
                                        and path == exception['pointer'] and key == exception['field']
                                        and target == exception['literal_target'], 'undeclared dangling original endpoint')
                                exact(sha256(canonical(pointer(original, record_path))), exception['record_sha256'],
                                      'unresolved original endpoint record changed')
                                exact(pointer(original, record_path), exception['source_and_native_record_unchanged'],
                                      'unresolved original endpoint was rewritten')
                                result['disposition'] = exception['status']
                            output.append(result)
                    elif isinstance(value, (dict, list)):
                        visit(value, path + '/' + key, record_path, depth + 1)
            elif isinstance(row, list):
                for n, child in enumerate(row):
                    visit(child, path + '/' + str(n), record_path, depth + 1)
        for name, rows in original.items():
            if isinstance(rows, list):
                for n, row in enumerate(rows):
                    path = '/' + name + '/' + str(n)
                    visit(row, path, path)
    return sorted(output, key=canonical)


def project_numbered(paper, evidence, inventory, files, archive_members, index, construction):
    """Construct a whole O3 inventory; one missing member rejects the paper."""
    pid = paper['arxiv_id']; primary = evidence[pid + ':primary']; independent = evidence[pid + ':independent']
    claims = paper['rows']
    require(isinstance(claims, list) and 1 <= len(claims) <= 10000, 'numbered inventory is empty or unbounded')
    originals = {'primary': {'/objects/' + str(n): row for n, row in enumerate(primary['objects'])
                            if row['kind'] == 'equation'},
                 'independent': {'/equations/' + str(n): row for n, row in enumerate(independent['equations'])}}
    parsed = {row['id']: row for row in inventory['objects']}
    require(len(parsed) == len(inventory['objects']), 'source identities repeat')
    members = {row['path']: row for row in archive_members}
    used = {'primary': set(), 'independent': set(), 'source': set()}; occupied = set(); result = []
    for claim in claims:
        p, i, old_source = (record(evidence, claim[key]) for key in ('primary', 'independent', 'automatic_source_object'))
        for side, value in [('primary', p), ('independent', i)]:
            require(claim[side]['artifact'] == pid + ':' + side
                    and claim[side]['pointer'] in originals[side]
                    and claim[side]['pointer'] not in used[side], 'numbered original reused or misclassified')
            used[side].add(claim[side]['pointer'])
            exact(value['kind'], 'equation' if side == 'primary' else 'numbered_equation',
                  'original numbered equation role differs')
        sid = claim['proposed_id']; require(sid in parsed and sid not in used['source'], 'numbered source reused or absent')
        current = parsed[sid]; used['source'].add(sid)
        require(current['kind'] == old_source['kind'] == claim['kind'] == 'equation'
                and current['id'] == old_source['id'] == sid, 'numbered source identity differs')
        exact(current['source_members'], old_source['source_members'], 'numbered source members changed')
        require(reference_names_verified(inventory, current['labels']), 'numbered source labels are unverified')
        psource = original_source(p, files, members, primary=True)
        isource = original_source(i, files, members, primary=False)
        exact(psource, isource, 'paired numbered source extents disagree')
        exact(psource, claim['manual_source_member'], 'construction changes whole numbered source')
        exact(claim['manual_source_excerpt_sha256_utf8'], sha256(files[psource['path']][psource['start']:psource['end']].encode()),
              'numbered whole source text hash differs')
        parts = current['source_members']; require(len(parts) == 1, 'unsupported multipart numbered source')
        exact(parts, claim['automatic_source_members'], 'numbered automatic source changes')
        part = parts[0]
        require(part['path'] == psource['path'] and psource['start'] <= part['start'] < part['end'] <= psource['end'],
                'automatic row is outside its complete original display')
        relation = ('exact' if part == psource else
                    'reviewed_contained_automatic_occurrence_in_whole_manual_display')
        exact(claim['source_relation'], relation, 'numbered contained occurrence relation differs')
        label = p.get('printed_label', p.get('label'))
        require(isinstance(label, str) and label.strip(), 'numbered printed label is absent')
        exact(i['printed_label'], label, 'original printed numbers disagree')
        exact(claim['printed_number'], label, 'construction changes printed number')
        pm = original_native(body_members(p, primary=True), index)
        im = original_native(body_members(i, primary=False), index)
        require(pm and covered(pm, index) == covered(im, index), 'complete numbered native bodies disagree')
        chosen = native_members(claim['proposed_body_native_members'], index)
        exact(chosen, pm, 'construction changes selected raw original body')
        pn = original_native(number_members(p, primary=True), index)
        inn = original_native(number_members(i, primary=False), index)
        exact(pn, inn, 'original printed number memberships disagree')
        exact(native_members(claim['proposed_number_native_members'], index), pn, 'selected printed number changed')
        encoded = index['text'].encode('utf-16-le')
        printed = ''.join(encoded[x['start']*2:x['end']*2].decode('utf-16-le') for x in pn)
        exact(printed.strip(), '(' + label + ')', 'native membership is not its exact printed number')
        points, number_points = covered(pm, index), covered(pn, index)
        require(number_points and not points & number_points and not occupied & (points | number_points),
                'numbered body/number or distinct objects reuse native ownership')
        occupied |= points | number_points
        geometry = record(evidence, claim['geometry']); membership = record(evidence, claim['membership'])
        exact(membership['primary_id'], p['id'], 'membership changes primary occurrence')
        exact(membership['independent_id'], i['id'], 'membership changes independent occurrence')
        exact(membership['raw_record_hashes'], {'primary': sha256(canonical(p)), 'independent': sha256(canonical(i))},
              'membership changes original record identities')
        require(membership['number_membership_exact'] is True
                and not membership['primary_only_non_whitespace_positions']
                and not membership['independent_only_non_whitespace_positions'],
                'native membership difference is not independently reviewed whitespace')
        printed_review = record(evidence, claim['source_printed'])
        if 'original_primary' in printed_review:
            exact(printed_review['original_primary'], p, 'source/printed review changes primary record')
            exact(printed_review['original_independent'], i, 'source/printed review changes independent record')
        else:
            exact(printed_review['paper'], pid, 'source/printed review names another paper')
            exact(printed_review['printed_label'], label, 'source/printed review names another display')
            exact(printed_review['original_source_proposals'], {'primary': p['source'], 'independent': i['source']},
                  'source/printed review changes whole original source')
            exact(printed_review['reviewed_geometry_exact_copy'], geometry, 'source/printed review changes geometry')
        exact(geometry['arxiv_id'], pid, 'geometry names another paper')
        exact(geometry['printed_label'], label, 'geometry names another printed number')
        require(printed_review.get('source_vs_printed_expression_identity') == 'clear_for_this_complete_display'
                or printed_review.get('source_printed_identity') == 'consistent for this bounded original display',
                'complete source/printed identity remains unresolved')
        require(not printed_review.get('unresolved_source_vs_printed_expression_ambiguities')
                and (printed_review['semantic_quote_truth'] is False if 'semantic_quote_truth' in printed_review
                     else printed_review.get('full_math_fidelity_certified') is False),
                'source/printed judgment has unsupported fidelity claims')
        regions = []
        for key, spans, target in [('body', pm, 'proposed_body_region'), ('printed_number', pn, 'proposed_number_region')]:
            selected = claim[target]; reviewed = geometry[key]['root_region']
            original_p = p['body_region'] if key == 'body' else p.get('printed_number_region', p.get('number_region'))
            original_i = i['body_regions'] if key == 'body' else i['number_regions']
            require(len(original_i) == 1, 'numbered original geometry is incomplete or multipart')
            exact(geometry[key]['original_proposals'], [original_p, original_i[0]],
                  'numbered geometry changes an original proposal')
            exact(selected, reviewed, 'numbered selected geometry differs from its reviewed decision')
            require(selected['origin'] == 'top-left' and selected['unit'] == 'PDF points',
                    'numbered region changes coordinate domain')
            exact([selected['rect'][k] for k in COORDS],
                  [value * .75 for value in geometry[key]['one_pixel_margin_pixels']],
                  'numbered geometry changes original pixel conversion')
            rect = rectangle(selected['page'], [selected['rect'][k] for k in COORDS], index)
            require(member_pages(spans, index) == {rect['page']}, 'numbered geometry has another native page owner')
            regions.append(rect)
        parent_ids = [row['id'] for row in inventory['objects'] if row['id'] != sid and any(
            value['path'] == psource['path'] and value['start'] <= psource['start'] < psource['end'] <= value['end']
            for value in row['source_members'])]
        require(len(parent_ids) <= 1, 'numbered owner is ambiguous')
        declared_parent = construction['ownership'][sid]['source_parent_object']
        exact(declared_parent, parent_ids[0] if parent_ids else None, 'numbered retained owner differs')
        exact(construction['ownership'][sid]['source_child_objects'], [], 'nested numbered children need another format')
        result.append({'id': sid, 'kind': 'equation', 'labels': deepcopy(current['labels']), 'printed_heading': label,
                       'spans': pm, 'region': [regions[0]], 'printed_number_spans': pn,
                       'printed_number_regions': [regions[1]], 'source_members': [psource],
                       'source_parent_object': declared_parent, 'source_child_objects': [],
                       'reading_index_fidelity': claim['native_fidelity'], 'semantic_quote_truth': False,
                       'automatic_alignment_changed': False})
    for side in ('primary', 'independent'):
        require(used[side] == set(originals[side]), 'whole original numbered inventory is incomplete')
    require(used['source'] == {key for key, row in parsed.items() if row['kind'] == 'equation'},
            'whole current source numbered inventory is incomplete')
    require(set(construction['ownership']) == used['source'], 'numbered ownership inventory differs')
    exact(paper['complete_original_numbered_equation_counts'],
          dict(primary=len(result), independent=len(result), source_objects=len(result)), 'numbered counts differ')
    ledger = retention(primary, independent, inventory, claims)
    exact(construction['retained_inventory'], ledger, 'construction drops or changes unscored original/source records')
    return result, ledger


def validate_history(docs, evidence, raws, evidence_raws, pages):
    """Bind actual first-pass identities and paths, never infer them from labels."""
    packet = docs['packet']; pid = packet['arxiv_id']; assignment = docs['assignment']
    expected = {'primary': assignment['primary_agent'], 'independent': assignment['independent_agent']}
    require(len(set(expected.values())) == 2 and all(isinstance(v, str) and v for v in expected.values()),
            'numbered original producers are not distinct')
    assigned = [r for r in assignment['packets'] if r['arxiv_id'] == pid]
    require(len(assigned) == 1, 'numbered paper lacks unique original assignment')
    hash_ref(assigned[0]['packet'], raws['packet'], 'assignment changes original packet')
    exact(assigned[0]['annotation_paths'], packet['annotation_paths'], 'assignment changes original output paths')
    for side in ('primary', 'independent'):
        original = evidence[pid + ':' + side]; receipt = evidence[pid + ':' + side + '_receipt']
        frozen = [row for row in docs['checkpoint']['records'] if row['arxiv_id'] == pid and row['role'] == side]
        require(len(frozen) == 1, 'original freeze omits or repeats a producer')
        for name, suffix in [('inventory-v1.json', ''), ('receipt-v1.json', '_receipt')]:
            hash_ref(frozen[0]['artifacts'][name], evidence_raws[pid + ':' + side + suffix],
                     'original annotation differs from the immutable checkpoint')
        require(original['arxiv_id'] == receipt['arxiv_id'] == pid, 'original paper identity differs')
        actual = receipt.get('producer', receipt.get('annotator'))
        exact(actual, expected[side], 'original receipt producer differs from assignment')
        hash_ref(receipt['inventory'], evidence_raws[pid + ':' + side], 'original receipt inventory differs')
        exact(receipt['inventory']['path'], packet['annotation_paths'][side], 'original inventory path differs')
        required = {sha256(raws['packet']), sha256(raws['prompt']), sha256(evidence_raws[pid + ':source']),
                    sha256(evidence_raws[pid + ':native'])} | {r['sha256'] for r in packet['images']}
        inputs = receipt_inputs(receipt)
        require(required <= {r['sha256'] for r in inputs},
                'original inputs omit source/native/prompt/pages')
        require(sha256(raws['construction']) not in {r['sha256'] for r in inputs},
                'post-freeze construction became a first-pass input')
        if side == 'primary':
            exact(receipt['inputs']['selection_sha256'], sha256(raws['selection']), 'original selection hash differs')
            exact(receipt['inputs']['policy_sha256'], sha256(raws['policy']), 'original policy hash differs')
            exact(receipt['inputs']['source_member_inventory'], evidence[pid + ':source']['members'],
                  'original receipt changes source member metadata')
            history = receipt['inspection_history']
            require(history['all_original_pages_viewed_before_source_and_native_transcription'] is True
                    and history['peer_labels_read'] is False
                    and history['production_LaTeX_object_parser_used'] is False,
                    'primary pages-first blind history differs')
            exact(history['pages_viewed'], pages, 'primary original page inventory differs')
        else:
            exact(receipt['pages_actually_viewed'], pages, 'independent original page inventory differs')
            require(receipt.get('first_full_page_inspection_before_source_native') is True
                    or receipt.get('viewing_order') == 'All8complete original renders viewed before any source/native export transcription.',
                    'independent pages-first history differs')
    history = docs['producer_history']
    require(history['format'] == 'k1-numbered-original-producer-closure-v1', 'producer closure format differs')
    exact(history['assignment_sha256'], sha256(raws['assignment']), 'producer closure changes assignment')
    exact(history['checkpoint_sha256'], sha256(raws['checkpoint']), 'producer closure changes original freeze')
    exact(history['confirmed_producers'], expected, 'producer closure changes original identities')
    require(history['reviewer'] not in expected.values() and history['verdict'] == 'clear_original_producer_history',
            'original producer closure is not independently clear')
    return expected


def validate_numbered_math(candidate_raw, index_raw, source_raw, raws, evidence_raws, images, implementation):
    require(set(raws) == ARTIFACTS and len(evidence_raws) <= 128
            and sum(map(len, list(raws.values()) + list(evidence_raws.values()))) <= MAX_BUNDLE_BYTES,
            'numbered evidence inventory exceeds its bound')
    docs = {key: value.decode() if key == 'prompt' else document(value) for key, value in raws.items()}
    evidence = {key: document(value) for key, value in evidence_raws.items()}
    require(set(docs['proposal']['artifacts']) == set(evidence_raws), 'proposal evidence inventory differs')
    for key, binding in docs['proposal']['artifacts'].items():
        hash_ref(binding, evidence_raws[key], 'proposal evidence bytes differ')
    candidate = document(candidate_raw); index = document(index_raw)['index']; packet = docs['packet']; pid = candidate['arxiv_id']
    require(not any(key in candidate for key in OVERLAYS), 'numbered-only cannot mix manual overlays')
    exact(candidate['index']['sha256'], sha256(index_raw), 'numbered native index differs')
    exact(candidate['source_sha256'], sha256(source_raw), 'numbered source archive differs')
    exact(candidate['source_inventory_sha256'], sha256(canonical(candidate['source_inventory'])), 'historical source inventory hash differs')
    for key in ('arxiv_id', 'paper_id', 'stratum'):
        exact(packet[key], candidate[key], 'numbered packet identity differs')
    exact(packet['index']['sha256'], sha256(index_raw), 'original packet native hash differs')
    for kind in ('source', 'pdf'):
        exact(packet[kind]['sha256'], candidate[kind + '_sha256'], 'original packet artifact differs')
    pages = [row['number'] for row in index['pages']]
    require(1 <= len(pages) <= 32 and pages == list(range(1, len(pages) + 1)), 'numbered page inventory differs')
    producers = validate_history(docs, evidence, raws, evidence_raws, pages)
    exact(images, {row['path']: row['sha256'] for row in packet['images']}, 'numbered original images differ')
    exact([row['page'] for row in packet['images']], pages, 'numbered original image order differs')
    for image, page in zip(packet['images'], index['pages']):
        require(image['dpi'] == 96 and type(image['dpi']) is int, 'numbered image DPI differs')
        for pixel, native in [('pixel_width', 'width'), ('pixel_height', 'height')]:
            require(image[pixel] == math.ceil(page[native] * 4 / 3), 'numbered original image dimensions differ')
    files, members = read_archive(source_raw)
    exported = evidence[pid + ':source']
    exact(exported['source_sha256'], sha256(source_raw), 'source export archive differs')
    exact(exported['text_members'], files, 'source export changes decoded deposited text')
    exact(exported['members'], members, 'source export changes original encoding/member hashes')
    native_raw = evidence_raws[pid + ':native']
    verify_native_export(index_raw, native_raw, candidate['paper_id'], {'format': NATIVE_FORMAT,
                         'sha256': sha256(native_raw), 'bytes': len(native_raw), 'index_sha256': sha256(index_raw)})
    construction = docs['construction']
    require(set(implementation) == PRODUCER_MODULES, 'numbered producer module inventory differs')
    exact(construction['producer_modules'], implementation, 'numbered construction producer bytes changed')
    require(construction['format'] == FORMAT and construction['not_an_original_annotator_input'] is True,
            'numbered construction history is absent')
    exact(construction['candidate_sha256'], sha256(candidate_raw), 'construction changes candidate')
    exact(construction['proposal_sha256'], sha256(raws['proposal']), 'construction changes proposal')
    inventory = parse_project(files)
    changed = source_scope(inventory, candidate['source_inventory'], files, construction)
    reviewed_source_scope(pid, docs, raws, files, members, inventory, source_raw)
    for binding in docs['source_scope_review']['module_bindings']:
        exact(implementation[Path(binding['path']).name], binding['sha256'], 'finite source reviewer inspected another parser')
    proposals = [p for p in docs['proposal']['papers'] if p['arxiv_id'] == pid]
    require(len(proposals) == 1, 'numbered proposal omits or repeats paper')
    paper = proposals[0]; exact(paper['paper_id'], candidate['paper_id'], 'proposal mapped identity differs')
    objects, ledger = project_numbered(paper, evidence, inventory, files, members, index, construction)
    proposal_review = docs['proposal_review']; addendum = docs['endpoint_addendum']
    hash_ref(proposal_review['proposal'], raws['proposal'], 'mechanical review changes proposal')
    hash_ref(proposal_review['endpoint_addendum'], raws['endpoint_addendum'], 'mechanical review changes unscored endpoint')
    require(proposal_review['verdict'] == 'clear_within_mechanical_scope'
            and proposal_review['reviewer'] != proposal_review['producer']
            and all(row['status'] == 'resolved_by_additive_explicit_unscored_disposition'
                    and row['correction_sha256'] == sha256(raws['endpoint_addendum'])
                    for row in proposal_review['findings']), 'proposal has unresolved mechanical findings')
    endpoint_rows = original_endpoints(evidence[pid + ':primary'], evidence[pid + ':independent'], addendum, pid)
    exact(construction['original_endpoint_dispositions'], endpoint_rows, 'construction drops original endpoint dispositions')
    ledger['original_endpoint_dispositions'] = endpoint_rows
    dispositions = docs['dispositions']
    exact(dispositions['original_order'], docs['checkpoint']['original_order'], 'fourteen original dispositions were reordered')
    require(len(dispositions['rows']) == len(dispositions['original_order']) == 14, 'fourteen dispositions are incomplete')
    exact([row['arxiv_id'] for row in dispositions['rows']], dispositions['original_order'], 'disposition identities differ')
    exact([row['retained_matrix_row'] for row in dispositions['rows']], docs['matrix']['rows'],
          'fourteen dispositions drop source failures, unknowns or metric deficits')
    for row in dispositions['rows']:
        exact(row['retained_matrix_row']['arxiv_id'], row['arxiv_id'], 'disposition changes retained identity')
        exact(row['original_checkpoint_records'], [value for value in docs['checkpoint']['records']
              if value['arxiv_id'] == row['arxiv_id']], 'disposition changes original freeze records')
    selected = [row for row in dispositions['rows'] if row['construction_scope'] == 'proposed_complete_O3']
    require(len(selected) == 2 and pid in {row['arxiv_id'] for row in selected}, 'numbered fixed construction subset differs')
    exact([row['arxiv_id'] for row in selected], [row['arxiv_id'] for row in docs['proposal']['papers']],
          'numbered construction changes fixed proposal subset')
    require(all(row['metric_eligibility_granted'] == [] for row in dispositions['rows']), 'dispositions grant unsupported eligibility')
    review = docs['review']
    require(review['format'] == REVIEW_FORMAT and review['verdict'] == 'clear_complete_numbered_inventory'
            and review['findings'] == [], 'numbered construction lacks distinct clearance')
    exact(review['artifacts'], {k: sha256(v) for k, v in raws.items() if k != 'review'}, 'construction review evidence closure differs')
    exact(review['proposal_artifacts'], {k: sha256(v) for k, v in evidence_raws.items()}, 'construction review proposal closure differs')
    exact(review['annotators'], producers, 'numbered construction reviewer changes annotators')
    require(isinstance(construction['producer'], str) and construction['producer'] not in producers.values()
            and isinstance(review['reviewer'], str) and review['reviewer']
            and review['reviewer'] not in {*producers.values(), construction['producer']},
            'numbered construction review is not distinct')
    exact(review['construction_producer'], construction['producer'], 'review changes construction producer')
    exact(review['source_scope_sha256'], sha256(canonical(construction['source_scope'])), 'finite source scope is unreviewed')
    exact(review['complete_numbered_count'], len(objects), 'reviewed whole numbered count differs')
    exact(review['candidate_sha256'], sha256(candidate_raw), 'review names another candidate')
    exact(review['current_source_inventory_sha256'], sha256(canonical(inventory)), 'review names another current source')
    exact(review['omitted_metrics'], OMITTED_METRICS, 'numbered review grants other metrics')
    require(review['later_construction_reviewed_after_original_freeze'] is True, 'review timing is not explicit')
    result = deepcopy(candidate)
    result['manual_numbered_math_overlay'] = {'objects': objects, 'metric_eligibility': {'O3': True},
        'omitted_metrics': OMITTED_METRICS.copy(), 'retained_inventory': ledger,
        'fourteen_paper_dispositions': deepcopy(dispositions), 'reviewed_absent_kinds': [],
        'historical_source_inventory_sha256': candidate['source_inventory_sha256'],
        'current_source_inventory_sha256': sha256(canonical(inventory)), 'historical_source_difference_fields': changed,
        'current_source_diagnostics': deepcopy(inventory['coverage']), 'source_scope': deepcopy(construction['source_scope']),
        'evidence_format': FORMAT, 'evidence_hashes': {k: sha256(v) for k, v in raws.items()},
        'proposal_evidence_hashes': {k: sha256(v) for k, v in evidence_raws.items()}, 'final_k1_publication': False}
    return result


def attach_numbered_math(cache, corpus_root, data_root, candidate_raw, paper, mapped, relative):
    from builder import fingerprint_file, safe_file
    from manual import bounded
    manifest_raw = bounded(cache, relative + '/manifest.json'); manifest = document(manifest_raw)
    require(manifest.get('format') == FORMAT and manifest.get('schema_version') == 1
            and set(manifest['artifacts']) == ARTIFACTS, 'unsupported numbered manifest')
    raws = {}; total = 0
    for key, binding in manifest['artifacts'].items():
        require(set(binding) == {'path', 'sha256', 'bytes'}, 'numbered artifact declaration differs')
        raw = bounded(cache, binding['path']); total += len(raw)
        require(total <= MAX_BUNDLE_BYTES, 'numbered bundle exceeds cumulative bound')
        hash_ref(binding, raw, 'numbered artifact bytes differ'); raws[key] = raw
    proposal = document(raws['proposal']); evidence_raws = {}
    require(isinstance(proposal['artifacts'], dict) and len(proposal['artifacts']) <= 128,
            'proposal artifact closure exceeds its bound')
    for key, binding in proposal['artifacts'].items():
        path = Path(binding['path']); require(path.is_relative_to(cache), 'proposal artifact escapes cache')
        raw = bounded(cache, str(path.relative_to(cache))); total += len(raw)
        require(total <= MAX_BUNDLE_BYTES, 'proposal evidence exceeds cumulative bound')
        hash_ref(binding, raw, 'proposal artifact bytes differ'); evidence_raws[key] = raw
    candidate = document(candidate_raw); packet = document(raws['packet'])
    exact(candidate['index'], mapped['index'], 'numbered candidate changes frozen native mapping')
    exact(candidate['paper_id'], mapped['paper_id'], 'numbered candidate changes mapped paper')
    exact(candidate['arxiv_id'], paper['arxiv_id'], 'numbered candidate changes original arXiv paper')
    exact(packet['version'], paper['version'], 'numbered original version differs')
    exact(packet['stratum'], paper['stratum'], 'numbered original stratum differs')
    index_raw = bounded(data_root, mapped['index']['path'])
    exact(packet['index']['path'], str(safe_file(data_root, mapped['index']['path'])),
          'original packet names another actual native path')
    exact(sha256(index_raw), mapped['index']['sha256'], 'numbered mapped native bytes differ')
    exact(mapped['pdf_sha256'], paper['pdf']['sha256'], 'numbered mapped PDF differs')
    source_raw = None
    for kind in ('pdf', 'source'):
        path = safe_file(corpus_root, paper[kind]['path']); digest, size = fingerprint_file(path, cap=64*1024*1024)
        require(digest == paper[kind]['sha256'] == packet[kind]['sha256'] == candidate[kind + '_sha256']
                and size == paper[kind]['bytes'] == packet[kind]['bytes'] and packet[kind]['path'] == str(path),
                'numbered original corpus path or bytes differ')
        if kind == 'source':
            source_raw = path.read_bytes(); require(len(source_raw) == size and sha256(source_raw) == digest,
                                                   'numbered source changed during read')
    original_files = []
    for side in ('primary', 'independent'):
        receipt = document(evidence_raws[candidate['arxiv_id'] + ':' + side + '_receipt'])
        original_files += receipt_inputs(receipt) + [receipt['inventory']]
    original_files += packet['images']
    verified = {}
    for binding in original_files:
        path = Path(binding['path'])
        owner = corpus_root if path.is_relative_to(corpus_root) else cache
        require(path.is_relative_to(owner), 'original evidence escapes its dedicated roots')
        actual, size = fingerprint_file(safe_file(owner, str(path.relative_to(owner))), cap=32*1024*1024)
        require(actual == binding['sha256'] and size == binding['bytes'], 'actual original evidence bytes differ')
        verified[str(path)] = actual
    exact(packet['selection_sha256'], sha256(raws['selection']), 'numbered selection changes')
    exact(packet['policy_sha256'], sha256(raws['policy']), 'numbered policy changes')
    implementation = producer_modules()
    result = validate_numbered_math(candidate_raw, index_raw, source_raw, raws, evidence_raws,
                                   {row['path']: verified[row['path']] for row in packet['images']}, implementation)
    for key, binding in manifest['artifacts'].items():
        exact(bounded(cache, binding['path']), raws[key], 'numbered artifact changed during validation')
    for key, binding in proposal['artifacts'].items():
        exact(bounded(cache, str(Path(binding['path']).relative_to(cache))), evidence_raws[key],
              'numbered proposal changed during validation')
    exact(bounded(cache, relative + '/manifest.json'), manifest_raw, 'numbered manifest changed during validation')
    for binding in original_files:
        path = Path(binding['path']); owner = corpus_root if path.is_relative_to(corpus_root) else cache
        actual, size = fingerprint_file(safe_file(owner, str(path.relative_to(owner))), cap=32*1024*1024)
        require(actual == binding['sha256'] and size == binding['bytes'], 'original evidence changed during validation')
    exact(bounded(data_root, mapped['index']['path']), index_raw, 'native index changed during validation')
    for kind in ('pdf', 'source'):
        actual, size = fingerprint_file(safe_file(corpus_root, paper[kind]['path']), cap=64*1024*1024)
        require(actual == paper[kind]['sha256'] and size == paper[kind]['bytes'],
                'original corpus artifact changed during validation')
    exact(producer_modules(), implementation, 'numbered producer changed during construction')
    result['manual_assembly'] = {'format': FORMAT, 'manifest_sha256': sha256(manifest_raw),
        'candidate_sha256': sha256(candidate_raw), 'final_k1_publication': False,
        'omitted_manual_kinds': ['figure', 'table', 'statement', 'proof', 'algorithm', 'bib_entry'],
        'omitted_metrics': OMITTED_METRICS.copy(),
        'source_inventory_policy': 'automatic guards unchanged; complete numbered inventory and finite source scope independently reviewed'}
    return result
