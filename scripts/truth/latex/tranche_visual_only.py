"""Review-bound O1/O2 projection; every other original role remains unscored.

This is deliberately separate from the visual/formal-negative tranche codec.
An original theorem, proof, algorithm or unresolved locator cannot become a
negative formal example merely because its figure/table projection is complete.
"""
from copy import deepcopy
import re

from annotations import canonical, require
from archive import sha256
from parser import argument_commands, reference_names_verified
from tex import comments
from tranche import covered, member_pages, rectangle, source_member, unique
from tranche_visual import COORDS, exact, members, source

FORMAT = 'k1-manual-visual-only-tranche-v1'
REVIEW_FORMAT = 'k1-manual-visual-only-construction-review-v1'
OMITTED_METRICS = [f'O{number}' for number in range(3, 12)]
VISUAL_KINDS = {'figure', 'table'}


def pointer(document, path):
    """A bounded canonical JSON pointer, never a filesystem path or expression."""
    require(isinstance(path, str) and path.startswith('/') and len(path) <= 2048,
            'invalid original evidence pointer')
    parts = path[1:].split('/')
    require(len(parts) <= 32, 'original evidence pointer is too deep')
    value = document
    for part in parts:
        require(re.search(r'~(?![01])', part) is None, 'invalid JSON pointer escape')
        key = part.replace('~1', '/').replace('~0', '~')
        if isinstance(value, list):
            require(re.fullmatch(r'0|[1-9][0-9]*', key) is not None
                    and int(key) < len(value), 'original evidence list index differs')
            value = value[int(key)]
        else:
            require(isinstance(value, dict) and key in value, 'original evidence pointer is absent')
            value = value[key]
    return value


def escape(value):
    return value.replace('~', '~0').replace('/', '~1')


def retained_original(original, scored):
    """Account for every top-level value and every original collection member.

    Exact immutable raw records retain all nested spans, parent/child ownership,
    uncertainty and roles. The ledger exposes those records without copying the
    multi-megabyte native-token geometry into the compact truth artifact.
    """
    require(isinstance(original, dict) and len(original) <= 1000, 'original inventory is not bounded')
    output = []
    for key, value in original.items():
        root = '/' + escape(key)
        values = list(enumerate(value)) if isinstance(value, list) else [(None, value)]
        require(len(values) <= 10000, 'original collection exceeds its bound')
        records = []
        for number, record in values:
            path = root + '/' + str(number) if number is not None else root
            row = {'pointer': path, 'record_sha256': sha256(canonical(record)),
                   'disposition': 'scored_visual' if path in scored else 'retained_unscored_original'}
            if isinstance(record, dict):
                # These are original labels, not normalized source/parser truth.
                row['original_roles'] = {name: deepcopy(record[name]) for name in
                    ('id', 'occurrence_id', 'kind', 'role', 'target_kind', 'parent', 'parent_object',
                     'child_objects', 'target', 'target_id', 'targets', 'target_keys', 'source_keys')
                    if name in record}
            records.append(row)
        output.append({'pointer': root, 'value_sha256': sha256(canonical(value)),
                       'collection_count': len(value) if isinstance(value, list) else None,
                       'records': records})
    return output


def retained_source(inventory, scored, excluded):
    """Keep every current parsed occurrence, including syntactic visual false roles."""
    result = {}
    for kind in ('objects', 'links', 'entries'):
        rows = inventory[kind]
        require(isinstance(rows, list) and len(rows) <= 10000, 'source occurrence inventory exceeds its bound')
        result[kind] = []
        for number, row in enumerate(rows):
            # Links are identified by their original ordinal/source occurrence;
            # the parser intentionally does not manufacture an object-style ID.
            identity = row.get('id')
            role = 'retained_unscored_source'
            if kind == 'objects' and identity in scored:
                role = 'scored_visual'
            elif kind == 'objects' and identity in excluded:
                role = excluded[identity]['disposition']
            result[kind].append({'source_ordinal': number, 'id': identity,
                'record_sha256': sha256(canonical(row)), 'disposition': role,
                'source_roles': {key: deepcopy(row[key]) for key in
                    ('kind', 'source_members', 'labels', 'targets', 'source_occurrence_id',
                     'proof_targets', 'unsupported_commands') if key in row}})
    return result


def original_object(original, path):
    require(re.fullmatch(r'/objects/(0|[1-9][0-9]*)', path) is not None,
            'scored original must be an object occurrence')
    row = pointer(original, path)
    require(isinstance(row, dict) and row.get('kind') in VISUAL_KINDS,
            'scored original is not a printed visual')
    return row


def original_environment(row, primary, files):
    if primary:
        values = [row['source_environment']] if 'source_environment' in row else row['source_members']
    else:
        values = [row['source']]
    require(isinstance(values, list) and 1 <= len(values) <= 256, 'original visual source is missing or unbounded')
    return [source(value, files) for value in values]


def contains(outer, inner):
    return (outer['path'] == inner['path'] and
            outer['start'] <= inner['start'] < inner['end'] <= outer['end'])


def visual_source(parsed, primary, independent, files):
    """Bind an original environment to exact current source-map pieces.

    Included text may split its parent's original interval. Every current piece
    must be inside a recorded original member; dependencies are separately
    recorded by the independent annotator. No coordinate widening is involved.
    """
    p = original_environment(primary, True, files)
    i = original_environment(independent, False, files)
    require(sum(part == i[0] for part in p) == 1, 'original visual environment identities disagree')
    pieces = parsed['source_members']
    require(isinstance(pieces, list) and 1 <= len(pieces) <= 256, 'current visual source map is unbounded')
    for piece in pieces:
        source_member(piece, files)
        require(sum(contains(parent, piece) for parent in p) == 1,
                'current source piece lacks unique original ownership')
    if len(pieces) == 1:
        exact(pieces, i, 'direct source occurrence differs from original environment')
    else:
        # The source map supplies expansion order; the independent original
        # records its literal include command and named deposited dependency.
        inclusion = source(independent['source_inclusion'], files)
        require(contains(i[0], inclusion), 'source inclusion escapes its original visual owner')
        commands = list(argument_commands(comments(files[inclusion['path']]),
                        {'input', 'include'}, inclusion['start'], inclusion['end']))
        require(len(commands) == 1 and commands[0]['start'] == inclusion['start']
                and commands[0]['end'] == inclusion['end'], 'source inclusion is not its complete literal command')
        dependency = commands[0]['value']
        if not dependency.endswith('.tex'):
            dependency += '.tex'
        require(dependency in files, 'source inclusion is not a deposited text member')
        exact(independent['source_dependency_members'], [dependency], 'independent source dependency differs')
        exact(pieces, [{'path': i[0]['path'], 'start': i[0]['start'], 'end': inclusion['start']},
                       {'path': dependency, 'start': 0, 'end': len(files[dependency])},
                       {'path': i[0]['path'], 'start': inclusion['end'], 'end': i[0]['end']}],
              'source map is not the independently recorded inclusion expansion')
        exact(p, [i[0], pieces[1]], 'primary inclusion evidence adds or omits source members')
    return pieces, i[0]


def visual_caption(primary, independent, environment, files, index):
    p = source(primary['source_caption'], files)
    i = source(independent['caption_source'], files)
    exact(p, i, 'original caption source roles disagree')
    require(contains(environment, p), 'caption escapes its original visual environment')
    commands = list(argument_commands(comments(files[environment['path']]), {'caption'},
                    environment['start'], environment['end']))
    require(len(commands) == 1 and commands[0]['end'] - 1 == p['end']
            and p['start'] == p['end'] - len(commands[0]['value']),
            'caption is not the complete authored source argument')
    a = members(primary['native_caption_members'], index)
    b = members(independent.get('caption_native_members', [independent.get('caption_native')]), index)
    points = covered(a, index)
    require(points and points == covered(b, index), 'complete original caption memberships disagree or are empty')
    require(len(member_pages(a, index)) == 1, 'caption spans multiple pages')
    return a, points


def visual_projection(construction, primary, independent, inventory, files, index):
    """Validate a separately reviewed crosswalk; no parsing or admission side effects."""
    rows = construction['visuals']
    require(isinstance(rows, list) and len(rows) <= 256, 'visual crosswalk exceeds its bound')
    parsed = unique(inventory['objects'], 'id', 'source object IDs repeat')
    claimed = {side: set() for side in ('primary', 'independent')}
    source_claims = set(); occupied = set(); objects = []
    for row in rows:
        require(set(row) == {'source_id', 'primary', 'independent', 'region', 'body_basis'},
                'visual crosswalk fields differ')
        sid = row['source_id']
        require(sid in parsed and sid not in source_claims, 'visual source occurrence is absent or reused')
        source_claims.add(sid)
        originals = []
        for side, document in [('primary', primary), ('independent', independent)]:
            path = row[side]
            require(path not in claimed[side], 'original visual occurrence is reused')
            claimed[side].add(path); originals.append(original_object(document, path))
        p, i = originals; current = parsed[sid]
        require(p['kind'] == i['kind'] == current['kind'], 'per-occurrence visual source kind differs')
        exact(p['printed_number'], i['printed_label'], 'original visual printed numbers disagree')
        require(isinstance(p['printed_number'], str) and p['printed_number'].strip(), 'printed visual number is empty')
        require(reference_names_verified(inventory, current['labels'])
                and not any(label in inventory.get('ambiguous_labels', {}) for label in current['labels']),
                'current visual label has ambiguous or unverified ownership')
        labels = i['source_labels']
        exact(labels, current['labels'], 'original visual source labels differ')
        parts, environment = visual_source(current, p, i, files)
        spans, points = visual_caption(p, i, environment, files, index)
        require(not occupied & points, 'distinct visuals reuse native caption membership'); occupied |= points
        region = row['region']
        require(isinstance(region, dict) and set(region) == {'page', 'rect'}
                and isinstance(region['rect'], dict) and set(region['rect']) == set(COORDS),
                'visual region shape differs')
        rectangle(region['page'], [region['rect'][k] for k in COORDS], index)
        require(member_pages(spans, index) == {region['page']}, 'visual region is on another caption page')
        require(isinstance(row['body_basis'], dict) and row['body_basis'], 'visual body lacks reviewed original provenance')
        objects.append({'id': sid, 'kind': current['kind'], 'labels': deepcopy(current['labels']),
            'printed_label': p['printed_number'], 'spans': spans, 'region': [deepcopy(region)],
            'source_members': deepcopy(parts),
            'text_sha256': sha256(canonical(spans)),
            'reading_index_fidelity': 'exact original caption membership; body pixels independently reviewed',
            'semantic_quote_truth': False})
    for side, document in [('primary', primary), ('independent', independent)]:
        expected = {'/objects/' + str(n) for n, item in enumerate(document['objects']) if item['kind'] in VISUAL_KINDS}
        exact(claimed[side], expected, 'original visual inventory is incomplete')
    excluded = unique(construction['source_visual_exclusions'], 'source_id', 'source visual exclusion repeats')
    expected = {sid for sid, row in parsed.items() if row['kind'] in VISUAL_KINDS} - source_claims
    exact(set(excluded), expected, 'current syntactic visual source role is unaccounted')
    for sid, row in excluded.items():
        require(set(row) == {'source_id', 'disposition', 'source_members', 'original_evidence', 'reason'},
                'source visual exclusion fields differ')
        require(row['disposition'] in {'reviewed_printed_nonvisual', 'reviewed_nonrendered_source'}
                and isinstance(row['reason'], str) and row['reason'].strip(), 'unsupported source visual exclusion')
        exact(row['source_members'], parsed[sid]['source_members'], 'excluded source occurrence differs')
        evidence = row['original_evidence']
        require(isinstance(evidence, list) and 1 <= len(evidence) <= 64, 'source exclusion lacks original evidence')
        for item in evidence:
            require(set(item) == {'artifact', 'pointer', 'record_sha256'} and item['artifact'] in ('primary', 'independent'),
                    'unsupported source exclusion evidence')
            record = pointer(primary if item['artifact'] == 'primary' else independent, item['pointer'])
            exact(sha256(canonical(record)), item['record_sha256'], 'source exclusion changes original evidence')
            require(item['pointer'] not in claimed[item['artifact']], 'scored visual cannot be excluded as nonvisual')
    ledger = {'primary': retained_original(primary, claimed['primary']),
              'independent': retained_original(independent, claimed['independent']),
              'current_source': retained_source(inventory, source_claims, excluded)}
    exact(construction['retained_inventory'], ledger, 'construction drops or changes retained unscored roles')
    counts = {kind: sum(row['kind'] == kind for row in objects) for kind in ('figure', 'table')}
    exact(construction['complete_visual_counts'], counts, 'construction visual counts differ')
    absent = [kind for kind in ('figure', 'table') if counts[kind] == 0]
    exact(construction['reviewed_absent_visual_kinds'], absent, 'visual-negative kind evidence differs')
    return {'objects': objects, 'reviewed_absent_kinds': absent, 'retained_inventory': ledger,
            'source_visual_exclusions': deepcopy(list(excluded.values())),
            'metric_eligibility': {'O1': True, 'O2': True}, 'omitted_metrics': OMITTED_METRICS.copy()}
