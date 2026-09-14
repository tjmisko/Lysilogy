"""Review-bound O1/O2 projection; every other original role remains unscored.

This is deliberately separate from the visual/formal-negative tranche codec.
An original theorem, proof, algorithm or unresolved locator cannot become a
negative formal example merely because its figure/table projection is complete.
"""
from copy import deepcopy
import math
from pathlib import Path
import re

from annotations import canonical, document, require
from archive import read_archive, sha256
from native_exports import FORMAT as NATIVE_FORMAT, verify_native_export
from parser import argument_commands, reference_names_verified
from tex import COMMAND, SYMBOLS, comments, definition_regions, group
from tranche import covered, hash_ref, member_pages, rectangle, source_member, unique, verify_original_file
from tranche_visual import (COORDS, MAX_BUNDLE_BYTES, assignment_records, audit_excerpts,
                            current_source_binding, exact, members, source)

FORMAT = 'k1-manual-visual-only-tranche-v1'
REVIEW_FORMAT = 'k1-manual-visual-only-construction-review-v1'
OMITTED_METRICS = [f'O{number}' for number in range(3, 12)]
VISUAL_KINDS = {'figure', 'table'}
ARTIFACTS = {'policy', 'selection', 'prompt', 'packet', 'source_export', 'native_export',
             'primary', 'primary_receipt', 'independent', 'independent_receipt',
             'geometry', 'geometry_approval', 'construction', 'supplement_history',
             'assignment_proposal', 'assignment_request', 'assignment_history', 'identity_review', 'review'}


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
    for key, value in sorted(original.items()):
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
                    ('id', 'occurrence_id', 'kind', 'role', 'role_note', 'role_notes', 'role_evidence',
                     'source_role', 'target_kind', 'parent', 'parent_id', 'parent_object',
                     'source_parent_object', 'children', 'child_objects', 'child_ids',
                     'source_child_objects', 'proof_target', 'proof_target_ids', 'proof_targets',
                     'proof_linkage', 'proof_target_basis', 'target_basis',
                     'target', 'target_id', 'target_ids', 'target_occurrence_ids', 'targets',
                     'target_keys', 'candidate_targets', 'source_keys', 'source_labels',
                     'source_keys_in_authored_order', 'source_target_keys_in_order',
                     'source_target_key', 'source_target_label', 'source_target_labels',
                     'target_keys_in_printed_order', 'printed_target_order', 'resolution',
                     'ambiguity', 'ambiguities', 'target_ambiguities', 'certainty', 'semantic_caution')
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


def reviewed_body(claim, primary, independent, geometry, index):
    """Replay the recorded geometry format, including both unchanged originals."""
    basis = claim['body_basis']
    require(isinstance(basis, dict) and set(basis) == {'pointer', 'record_sha256'},
            'body basis must select one exact recorded geometry decision')
    row = pointer(geometry, basis['pointer'])
    exact(sha256(canonical(row)), basis['record_sha256'], 'body decision bytes differ')
    exact(row['primary_id'], primary['id'], 'body decision changes primary occurrence')
    exact(row['independent_id'], independent.get('id', independent.get('occurrence_id')),
          'body decision changes independent occurrence')
    exact(row['kind'], primary['kind'], 'body decision changes visual kind')
    exact(row.get('printed_number', row.get('printed_label')), primary['printed_number'],
          'body decision changes printed label')
    if type(geometry.get('schema')) is int and geometry['schema'] == 1:
        require(re.fullmatch(r'/objects/(0|[1-9][0-9]*)', basis['pointer']) is not None,
                'geometry pointer is not an object decision')
        body = row['body']
        exact(body['original_primary_region'], primary['body_region'], 'geometry replaces primary body')
        other = independent['visual_body']
        exact(body['original_independent_region']['region'], other['region'], 'geometry replaces independent body')
        exact(body['original_independent_region']['page'], other['page'], 'independent geometry page differs')
        chosen = body['chosen_region']; pixels = [body['chosen_pixels_96dpi'][k] for k in COORDS]
    elif geometry.get('schema') == 'independent-geometry-reconciliation-v1':
        require(re.fullmatch(r'/objects/(0|[1-9][0-9]*)', basis['pointer']) is not None,
                'geometry pointer is not an object decision')
        exact(row['original_primary_body_region'], primary['body_region'], 'geometry replaces primary body')
        exact(row['original_independent_body_region'], independent['full_visual_body']['region'],
              'geometry replaces independent body')
        chosen = row['chosen_full_body_region']; pixels = [row['chosen_original_96dpi_pixel_edges'][k] for k in COORDS]
    else:
        require(geometry.get('format') == 'k1-manual-visual-region-reconciliation-v1'
                and re.fullmatch(r'/regions/(0|[1-9][0-9]*)', basis['pointer']) is not None,
                'unsupported original geometry decision format')
        exact(row['original_primary_body'], primary['body_regions'], 'geometry replaces primary body')
        exact(row['original_independent_body'], independent['body_regions'], 'geometry replaces independent body')
        chosen = {'page': row['page'], **row['body_pdf_points']}; pixels = row['body_pixels_96dpi']
    require(chosen['page'] == row['page'], 'chosen geometry changes recorded page')
    normalized = rectangle(chosen['page'], [chosen[k] for k in COORDS], index, pixels=pixels)
    exact(claim['region'], normalized, 'projected body differs from exact recorded decision')


def associated_notes(primary, independent, visuals, files, index):
    """Retain the separately recorded tablenotes owner without a formal overlay."""
    by_primary = {pointer(primary, row['primary'])['id']: row for row in visuals}
    original = [row for row in primary.get('ancillary_members', []) if row.get('parent_id') in by_primary]
    result = []; occupied = set(); used = set()
    caption_points = set().union(*(covered(members(pointer(primary, row['primary'])['native_caption_members'], index), index)
                                  for row in visuals))
    for claim in visuals:
        p = pointer(primary, claim['primary']); i = pointer(independent, claim['independent'])
        notes = i.get('ancillary', [])
        require(isinstance(notes, list) and len(notes) <= 100, 'visual ancillary collection exceeds its bound')
        matching = [row for row in original if row['parent_id'] == p['id']]
        require(len(notes) == len(matching), 'attached visual note inventory differs')
        if not notes: continue
        require(p['kind'] == 'table', 'unsupported attached non-table content')
        owner = source(i['source'], files)
        for note in notes:
            require(note['role'] == 'table_note', 'unsupported visual ancillary role')
            visible = source(note['source'], files)
            candidates = [row for row in matching if contains(source(row['source'], files), visible)]
            require(len(candidates) == 1, 'attached note lacks unique paired source membership')
            other = candidates[0]; key = other['id']
            require(key not in used, 'attached note is reused'); used.add(key)
            full = source(other['source'], files)
            require(contains(owner, full), 'attached note escapes table source ownership')
            raw = files[full['path']]
            prefix = raw[full['start']:visible['start']]; suffix = raw[visible['end']:full['end']]
            require(re.fullmatch(r'\\begin\{tablenotes\}\s*\\footnotesize\s*\\item\s*', prefix) is not None
                    and re.fullmatch(r'\s*\\end\{tablenotes\}', suffix) is not None,
                    'attached note has unsupported hidden or additional source payload')
            a = members(other['native_members'], index); b = members(note['native_members'], index)
            points = covered(a, index)
            require(points and points == covered(b, index), 'attached note native memberships disagree')
            require(not points & occupied and member_pages(a, index) == {claim['region']['page']},
                    'attached note is reused or belongs to another page')
            require(not points & caption_points, 'attached note reuses caption membership')
            rect = claim['region']['rect']; positioned = set()
            for token in index['tokens']:
                overlap = points & set(range(token['start'], token['end']))
                if not overlap: continue
                require(token['page'] == claim['region']['page'] and token['rects'],
                        'attached note has missing or wrong-page native geometry')
                for box in token['rects']:
                    require(set(box) == set(COORDS), 'attached note token rectangle differs')
                    rectangle(token['page'], [box[key] for key in COORDS], index)
                    require(rect['x_min'] <= box['x_min'] < box['x_max'] <= rect['x_max']
                            and rect['y_min'] <= box['y_min'] < box['y_max'] <= rect['y_max'],
                            'attached note token geometry escapes reviewed full body')
                positioned |= overlap
            exact(positioned, points, 'attached note has unpositioned native membership')
            occupied |= points
            result.append({'kind': 'attached_table_note', 'parent': claim['source_id'],
                           'source_members': [full], 'visible_source_members': [visible], 'spans': a,
                           'original_primary_id': key, 'metric_eligibility': {},
                           'scope': 'Included in reviewed full visual body; not an independent formal object.'})
    exact(used, {row['id'] for row in original}, 'attached note ownership is incomplete')
    return result


def literal_macro_declarations(name, files):
    """Find definitions in position-preserving comment-masked text, even dormant bodies."""
    control = re.escape('\\' + name) + (r'(?![A-Za-z@])' if name[-1].isalpha() else '')
    target = r'(?:\{\s*' + control + r'\s*\}|' + control + ')'
    pattern = re.compile(r'\\(?:newcommand|renewcommand|providecommand|DeclareRobustCommand)\*?\s*' + target
                         + r'|\\(?:def|gdef|edef|xdef|let|futurelet)\s*' + control
                         + r'|\\(?:newenvironment|renewenvironment)\*?\s*\{' + re.escape(name) + r'\}')
    return [(path, match.start()) for path, raw in files.items() for match in pattern.finditer(raw)]


def nonvisual_source_scope(path, files, inventory):
    active = inventory.get('coverage', {}).get('expanded_files', [path])
    require(isinstance(active, list) and 1 <= len(active) <= 256 and all(path in files for path in active),
            'nonvisual macro source closure is unsupported')
    dynamic = {'csname', 'catcode', 'lccode', 'uccode', 'lowercase', 'uppercase', 'scantokens',
               'read', 'readline', 'let', 'futurelet', 'afterassignment', 'aftergroup',
               'begingroup', 'endgroup', 'newif', 'else', 'fi', 'unless'}
    require(not any(match[1] in dynamic or match[1].startswith('if')
                    for path in active for match in COMMAND.finditer(files[path])),
            'nonvisual macro has unverified dynamic source binding')
    assignment = dynamic | {'newcommand', 'renewcommand', 'providecommand', 'DeclareRobustCommand',
                            'def', 'gdef', 'edef', 'xdef', 'newenvironment', 'renewenvironment'}
    for path in active:
        for start, end in definition_regions(files[path]):
            first = COMMAND.match(files[path], start)
            require(first is not None and not any(token[1].rstrip('*') in assignment
                    for token in COMMAND.finditer(files[path], first.end(), end)),
                    'nonvisual source has deferred or aliased assignment semantics')
    return active


def literal_macro_scope(name, definition, files, inventory, *, braced_invocations=False):
    exact(literal_macro_declarations(name, files), [(definition['path'], definition['start'])],
          'nonvisual macro has another or unsupported definition')
    active = nonvisual_source_scope(definition['path'], files, inventory)
    for path in active:
        for token in COMMAND.finditer(files[path]):
            if not braced_invocations or token[1] != name or path == definition['path'] and definition['start'] <= token.start() < definition['end']:
                continue
            # A control sequence used as another command's name/parameter is
            # not evidence of an ordinary invocation of this empty consumer.
            require(files[path][token.end():].lstrip().startswith('{'),
                    'nonvisual macro has an unexplained control-sequence use')
    prefix = files[definition['path']][:definition['start']]
    depth = 0; at = 0
    while at < len(prefix):
        if prefix[at] == '\\':
            token = COMMAND.match(prefix, at)
            require(token is not None, 'nonvisual macro has unsupported source tokenization')
            at = token.end(); continue
        if prefix[at] == '{': depth += 1
        elif prefix[at] == '}': depth -= 1
        require(depth >= 0, 'nonvisual macro has unbalanced source scope')
        at += 1
    require(depth == 0, 'nonvisual macro definition is inside an unproven source scope')


def nonvisual_float_context(occurrence, inner, files, inventory):
    """Check finite text/math/rule framing outside one paired formal occurrence.

    This is a source-role guard, not TeX execution or mathematical fidelity.
    Unknown commands, additional environments and parameterized aliases reject.
    Only unique earlier local zero-argument aliases with the same finite grammar
    can extend the literal commands; original full-page review remains required.
    """
    raw = files[occurrence['path']]
    nonvisual_source_scope(occurrence['path'], files, inventory)
    prefix = raw[occurrence['start']:inner['start']]
    suffix = raw[inner['end']:occurrence['end']]
    opening = re.match(r'\\begin\{(figure\*?|table\*?)\}(?:\[[htbp!H ]{1,32}\])?', prefix)
    require(opening is not None, 'nonvisual source lacks direct float framing')
    ending = re.search(re.escape('\\end{' + opening[1] + '}') + r'\s*$', suffix)
    require(ending is not None, 'nonvisual source float framing differs')
    context = prefix[opening.end():] + suffix[:ending.start()]
    allowed = set(SYMBOLS) | {'textbf', 'textit', 'emph', 'mbox', 'text', 'mathrm', 'mathbf', 'mathit',
        'mathbb', 'mathcal', 'mathsf', 'operatorname', 'ensuremath', 'bf', 'it', 'rm', 'cal',
        'centering', 'hrule', 'vspace', 'hspace', 'label', 'quad', 'qquad', 'hfill', 'vfill',
        'ne', 'in', 'notin', 'forall', 'exists', ',', ';', ':', '!', ' ', '\\'}
    budget = [512]; declaration_cache = {}
    def expand(text, depth):
        require(depth <= 8 and len(text) <= 16384, 'nonvisual context expansion exceeds its bound')
        output = []; end = 0
        for token in COMMAND.finditer(text):
            output.append(text[end:token.start()]); end = token.end()
            budget[0] -= 1
            require(budget[0] >= 0, 'nonvisual context command budget exceeded')
            name = token[1]
            if name not in declaration_cache: declaration_cache[name] = literal_macro_declarations(name, files)
            declarations = declaration_cache[name]
            if not declarations:
                require(name in allowed, 'nonvisual float contains unsupported extra source payload')
                output.append(token[0])
                continue
            require(name not in allowed and len(declarations) == 1, 'nonvisual context primitive is redefined')
            path, start = declarations[0]
            require(path == occurrence['path'] and start < occurrence['start'], 'nonvisual context alias is not an earlier local definition')
            source_text = files[path]
            head = re.match(r'\\newcommand\{\s*' + re.escape('\\' + name) + r'\s*\}\s*', source_text[start:])
            require(head is not None, 'nonvisual context alias has unsupported definition syntax')
            body, body_end = group(source_text, start + head.end())
            require(body_end <= occurrence['start'] and '#' not in body, 'nonvisual context alias is parameterized or out of scope')
            literal_macro_scope(name, {'path': path, 'start': start, 'end': body_end}, files, inventory)
            output.append(expand(body, depth + 1))
            require(sum(map(len, output)) <= 16384, 'nonvisual composed context exceeds its bound')
        output.append(text[end:]); result = ''.join(output)
        require(len(result) <= 16384, 'nonvisual composed context exceeds its bound')
        return result
    expanded = expand(context, 0)
    require(re.search(r'\\hrule\s*(?:width|height|depth)', expanded, re.IGNORECASE) is None,
            'nonvisual context rule has unsupported drawing parameters')


def visual_projection(construction, primary, independent, inventory, files, index, geometry):
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
        primary_labels = [p['source_label']] if 'source_label' in p else [item['value'] for item in p['source_labels']]
        exact(primary_labels, labels, 'primary visual label roles differ')
        parts, environment = visual_source(current, p, i, files)
        spans, points = visual_caption(p, i, environment, files, index)
        require(not occupied & points, 'distinct visuals reuse native caption membership'); occupied |= points
        region = row['region']
        require(isinstance(region, dict) and set(region) == {'page', 'rect'}
                and isinstance(region['rect'], dict) and set(region['rect']) == set(COORDS),
                'visual region shape differs')
        rectangle(region['page'], [region['rect'][k] for k in COORDS], index)
        require(member_pages(spans, index) == {region['page']}, 'visual region is on another caption page')
        reviewed_body(row, p, i, geometry, index)
        objects.append({'id': sid, 'kind': current['kind'], 'labels': deepcopy(current['labels']),
            'printed_label': p['printed_number'], 'spans': spans, 'region': [deepcopy(region)],
            'source_members': deepcopy(parts),
            'reading_index_fidelity': 'exact original caption membership; body pixels independently reviewed',
            'semantic_quote_truth': False})
    for side, document in [('primary', primary), ('independent', independent)]:
        expected = {'/objects/' + str(n) for n, item in enumerate(document['objects']) if item['kind'] in VISUAL_KINDS}
        exact(claimed[side], expected, 'original visual inventory is incomplete')
    excluded = unique(construction['source_visual_exclusions'], 'source_id', 'source visual exclusion repeats')
    expected = {sid for sid, row in parsed.items() if row['kind'] in VISUAL_KINDS} - source_claims
    exact(set(excluded), expected, 'current syntactic visual source role is unaccounted')
    source_comments = {path: comments(raw) for path, raw in files.items()} if excluded else {}
    for sid, row in excluded.items():
        require(set(row) == {'source_id', 'disposition', 'source_members', 'original_evidence', 'reason', 'source_role_evidence'},
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
        roles = row['source_role_evidence']
        require(len(parsed[sid]['source_members']) == 1, 'nonvisual source disposition requires one direct occurrence')
        occurrence = parsed[sid]['source_members'][0]
        if row['disposition'] == 'reviewed_printed_nonvisual':
            require(set(roles) == {'primary_object', 'independent_object'}, 'nonvisual role must bind both originals')
            paired = []
            for side, original in [('primary', primary), ('independent', independent)]:
                path = roles[side + '_object']
                require(re.fullmatch(r'/objects/(0|[1-9][0-9]*)', path) is not None, 'nonvisual role lacks original object')
                item = pointer(original, path)
                require(item['kind'] not in VISUAL_KINDS, 'printed visual cannot be relabeled as nonvisual')
                span = source(item['source_environment'] if side == 'primary' else item['source'], files)
                require(contains(occurrence, span), 'nonvisual original does not belong to the syntactic source object')
                paired.append((span, item['kind'], item.get('printed_number' if side == 'primary' else 'printed_label')))
            exact(paired[0], paired[1], 'nonvisual originals identify different source occurrences or roles')
            nonvisual_float_context(occurrence, paired[0][0], source_comments, inventory)
        else:
            require(set(roles) == {'wrapper', 'empty_macro_definition'}, 'nonrendered role lacks exact source context')
            wrapper = source(roles['wrapper'], files); definition = source(roles['empty_macro_definition'], files)
            require(contains(wrapper, occurrence) and definition['path'] == wrapper['path']
                    and definition['end'] <= wrapper['start'], 'nonrendered source context does not own its occurrence')
            raw = files[wrapper['path']]
            match = re.fullmatch(r'\\newcommand\{\\([A-Za-z]+)\}\[1\]\{\}', raw[definition['start']:definition['end']])
            require(match is not None, 'unsupported original empty-macro declaration')
            literal_macro_scope(match[1], definition, source_comments, inventory, braced_invocations=True)
            require(re.fullmatch(re.escape('\\' + match[1] + '{') + r'\s*', raw[wrapper['start']:occurrence['start']]) is not None
                    and re.fullmatch(r'\s*\}', raw[occurrence['end']:wrapper['end']]) is not None,
                    'nonrendered wrapper contains extra source payload')
    ledger = {'primary': retained_original(primary, claimed['primary']),
              'independent': retained_original(independent, claimed['independent']),
              'current_source': retained_source(inventory, source_claims, excluded)}
    exact(construction['retained_inventory'], ledger, 'construction drops or changes retained unscored roles')
    counts = {kind: sum(row['kind'] == kind for row in objects) for kind in ('figure', 'table')}
    exact(construction['complete_visual_counts'], counts, 'construction visual counts differ')
    absent = [kind for kind in ('figure', 'table') if counts[kind] == 0]
    exact(construction['reviewed_absent_visual_kinds'], absent, 'visual-negative kind evidence differs')
    return {'objects': objects, 'reviewed_absent_kinds': absent, 'retained_inventory': ledger,
            'associated_content': associated_notes(primary, independent, rows, files, index),
            'source_visual_exclusions': deepcopy(list(excluded.values())),
            'metric_eligibility': {'O1': True, 'O2': True}, 'omitted_metrics': OMITTED_METRICS.copy()}


def original_history(docs, raws, pages, verified_history):
    """Ground identities in the separately confirmed frozen assignment history."""
    packet = docs['packet']; identity = docs['identity_review']; request = docs['assignment_request']
    require(identity.get('format') == 'k1-manual-producer-map-review-v1'
            and identity.get('verdict') == 'clear_grounded_original_producers'
            and identity.get('findings') == [], 'original producers lack grounded separate confirmation')
    hash_ref(identity['request'], raws['assignment_request'], 'identity confirmation changes request')
    for field in ('assignment_proposal', 'assignment_history'):
        exact(identity[field + '_sha256'], sha256(raws[field]), 'identity confirmation changes historical assignment')
    proposal = docs['assignment_proposal']
    require(type(proposal.get('schema_version')) is int and proposal['schema_version'] == 1
            and isinstance(proposal.get('assignment_status'), str)
            and proposal['assignment_status'].startswith('proposed'), 'unsupported original assignment proposal')
    exact(proposal['selection_sha256'], sha256(raws['selection']), 'assignment proposal changes frozen selection')
    hash_ref(proposal['prompt'], raws['prompt'], 'assignment proposal changes blind prompt')
    hash_ref(docs['assignment_history']['assignment_proposal'], raws['assignment_proposal'],
             'running history changes original assignment proposal')
    hash_ref(request['assignment_proposal'], raws['assignment_proposal'], 'identity request changes proposal')
    hash_ref(request['prior_running_assignment_record'], raws['assignment_history'], 'identity request changes running record')
    exact(identity['git_history'], request['git_history'], 'identity confirmation changes retained Git history')
    expected_history = {row['retained_path']: row['sha256'] for row in docs['assignment_history']['records']}
    expected_history[request['git_history']['retained_path']] = request['git_history']['sha256']
    exact(verified_history, expected_history, 'actual retained assignment history differs')
    producers = {side: proposal[side + '_agent'] for side in ('primary', 'independent')}
    require(all(isinstance(value, str) and value.strip() for value in producers.values())
            and len(set(producers.values())) == 2, 'original producer identities are missing or repeated')
    exact(identity['confirmed_producers'], producers, 'grounded producer identities differ')
    exact(identity['reviewer'], proposal['reconciler'], 'grounded reviewer differs')
    require(isinstance(identity['reviewer'], str) and identity['reviewer'].strip()
            and identity['reviewer'] not in producers.values(), 'identity confirmation is not separate from both annotators')
    assigned = [row for row in proposal['papers'] if row['arxiv_id'] == packet['arxiv_id']]
    require(len(assigned) == 1, 'original paper assignment is missing or repeated')
    for side in ('primary', 'independent'):
        exact(assigned[0][side + '_output'], packet['blind_' + side + '_output'], 'original output ownership differs')
    records = [row for row in identity['papers'] if row['arxiv_id'] == packet['arxiv_id']]
    requested = [row for row in request['papers'] if row['arxiv_id'] == packet['arxiv_id']]
    require(len(records) == len(requested) == 1, 'producer confirmation omits or repeats this paper')
    exact(records[0]['original_artifacts'], requested[0]['original_artifacts'], 'identity confirmation changes original bindings')
    exact(records[0]['history_occurrences'], requested[0]['phase_exact_hash_occurrences'], 'identity confirmation changes historical excerpts')
    for name in ('primary', 'independent', 'primary_receipt', 'independent_receipt'):
        hash_ref(records[0]['original_artifacts'][name], raws[name], 'producer confirmation names another original')
    p, i, pr, ir = (docs[name] for name in ('primary', 'independent', 'primary_receipt', 'independent_receipt'))
    require(type(p.get('schema_version')) is int and p['schema_version'] == 1,
            'unsupported primary inventory schema')
    require(i.get('schema') in {'lysilogy-independent-manual-pilot-v1', 'lysilogy.blind-manual-inventory.v1',
                               'lysilogy-blind-independent-pilot-v1'}, 'unsupported independent inventory schema')
    for original in (p, i):
        paper = original.get('paper', original)
        exact(paper['paper_id'], packet['paper_id'], 'original mapped paper identity differs')
        exact(paper['arxiv_id'], packet['arxiv_id'], 'original arXiv identity differs')
        version = paper['version']
        require((type(version) is int and version == packet['version'])
                or (isinstance(version, str) and version == 'v' + str(packet['version'])),
                'original pinned version differs')
    hash_ref(pr['inventory'], raws['primary'], 'primary receipt changes its original inventory')
    if 'inventory' in ir:
        hash_ref(ir['inventory'], raws['independent'], 'independent receipt changes its original inventory')
    else:
        exact(ir['inventory_sha256'], sha256(raws['independent']), 'independent receipt changes original hash')
        exact(ir['inventory_bytes'], len(raws['independent']), 'independent receipt changes original size')
    require(pr.get('annotator_role', pr.get('annotation_role', 'primary')) == 'primary', 'primary receipt role differs')
    exact(ir.get('annotator', ir.get('annotator_task')), producers['independent'], 'independent receipt producer differs')
    if 'pages_seen_before_source_and_native' in pr:
        exact(pr['pages_seen_before_source_and_native'], pages, 'primary pages-first history differs')
        require(pr['blindness']['all_original_pages_viewed_before_source_and_native'] is True, 'primary pages-first assertion is absent')
    elif 'pages_viewed_first' in pr:
        exact(pr['pages_viewed_first'], pages, 'primary pages-first history differs')
        require(pr['protocol']['all_original_pages_viewed_before_source_and_native'] is True, 'primary pages-first assertion is absent')
    else:
        exact(pr['all_pages_viewed_before_source_native'], pages, 'primary pages-first history differs')
    if 'all_pages_inspected_before_source_native' in ir:
        require(ir['all_pages_inspected_before_source_native'] is True, 'independent pages-first assertion is absent')
        seen = ir['inspected_pages']
    elif 'pages_inspected_before_source_native' in ir:
        seen = ir['pages_inspected_before_source_native']
    elif 'all_original_pages_before_source_native' in ir:
        require(ir['all_original_pages_before_source_native'] is True, 'independent pages-first assertion is absent')
        seen = ir['page_inspection_order']
    else:
        require(isinstance(ir.get('inspection_order'), str) and 'pages inspected first' in ir['inspection_order'],
                'independent original inspection order is absent')
        seen = ir['pages_inspected']
    exact(seen, pages, 'independent pages-first history differs')
    required = {sha256(raws[name]) for name in ('packet', 'source_export', 'native_export', 'prompt')}
    required |= {row['sha256'] for row in packet['images']}
    independent_inputs = ir.get('inputs', ir.get('inputs_rehashed'))
    for ledger in (pr.get('inputs', pr.get('input_files')), independent_inputs):
        require(isinstance(ledger, list) and len(ledger) <= 1000, 'original input ledger is missing or unbounded')
        require(required <= {row['sha256'] for row in ledger}, 'original input ledger omits pages or source/native/prompt')
        require(sha256(raws['construction']) not in {row['sha256'] for row in ledger}, 'later construction became a blind input')
    return producers, identity['reviewer']


def validate_visual_only(candidate_raw, index_raw, source_raw, raws, images, verified_history):
    """Admission requires exact separate construction review after the blind freeze."""
    require(set(raws) == ARTIFACTS and sum(map(len, raws.values())) <= MAX_BUNDLE_BYTES,
            'visual-only evidence inventory or cumulative size differs')
    docs = {key: raw.decode('utf-8') if key == 'prompt' else document(raw) for key, raw in raws.items()}
    candidate = document(candidate_raw); index = document(index_raw)['index']; packet = docs['packet']
    require(candidate['index']['sha256'] == sha256(index_raw) and candidate['source_sha256'] == sha256(source_raw),
            'visual-only source/index identity differs')
    exact(candidate['source_inventory_sha256'], sha256(canonical(candidate['source_inventory'])), 'historical source hash differs')
    for key in ('arxiv_id', 'paper_id', 'index', 'stratum'):
        exact(packet[key], candidate[key], 'blind packet names another original paper')
    require(type(packet['version']) is int and packet['version'] > 0, 'blind packet version is invalid')
    for kind in ('pdf', 'source'):
        exact(packet[kind]['sha256'], candidate[kind + '_sha256'], 'blind packet names another artifact')
    require(not {'candidate', 'source_inventory', 'candidate_sha256', 'inventory_sha256'} & packet.keys(),
            'blind packet contains later parser output')
    pages = [row['number'] for row in index['pages']]
    require(1 <= len(pages) <= 32 and pages == list(range(1, len(pages) + 1)), 'original page inventory is invalid or unbounded')
    producers, reviewer = original_history(docs, raws, pages, verified_history)
    files, archive_members = read_archive(source_raw)
    require(not any('^^' in text for text in files.values()), 'unverified source pre-tokenization substitution')
    exact(docs['source_export']['source_sha256'], sha256(source_raw), 'original source export hash differs')
    exact(docs['source_export']['text_members'], files, 'source export changes deposited source text')
    exact(docs['source_export']['members'], archive_members, 'source export changes archive member inventory')
    construction = docs['construction']
    require(construction['not_an_original_annotator_input'] is True, 'construction timing is not explicit')
    hash_ref(construction['candidate'], candidate_raw, 'construction changes its historical candidate')
    for name in ('primary', 'independent', 'primary_receipt', 'independent_receipt', 'packet', 'source_export', 'native_export'):
        hash_ref(construction['original_artifacts'][name], raws[name], 'construction changes an original artifact')
    inventory = current_source_binding(docs, candidate, files)
    verify_native_export(index_raw, raws['native_export'], candidate['paper_id'],
                         {'format': NATIVE_FORMAT, 'sha256': sha256(raws['native_export']),
                          'bytes': len(raws['native_export']), 'index_sha256': sha256(index_raw)})
    exact({row['path']: row['sha256'] for row in packet['images']}, images, 'original image paths/hashes differ')
    exact([row['page'] for row in packet['images']], pages, 'original image page order differs')
    for image, page in zip(packet['images'], index['pages']):
        require(type(image['dpi']) is int and image['dpi'] == 96, 'original rendering DPI differs')
        for pixel, key, points in [('pixel_width', 'width', 'pdf_width_points'), ('pixel_height', 'height', 'pdf_height_points')]:
            dimension = page[key]
            require(type(dimension) in (int, float) and math.isfinite(dimension) and dimension > 0
                    and type(image[pixel]) is int and image[pixel] == math.ceil(dimension * 96 / 72)
                    and image[points] == dimension, 'original raster/native dimensions differ')
    for side in ('primary', 'independent'):
        audit_excerpts(docs[side], files, index)
    overlay = visual_projection(construction, docs['primary'], docs['independent'], inventory, files, index, docs['geometry'])
    review = docs['review']
    require(review.get('format') == REVIEW_FORMAT and review.get('verdict') == 'clear_complete_visual_only_projection'
            and review.get('findings') == [], 'visual-only construction review is not clear')
    exact(review['artifacts'], {key: sha256(raw) for key, raw in raws.items() if key != 'review'}, 'construction review changes evidence closure')
    exact(review['candidate_sha256'], sha256(candidate_raw), 'construction review changes original candidate')
    exact(review['current_source_inventory_sha256'], sha256(canonical(inventory)), 'construction review changes current source')
    exact(review['annotators'], producers, 'construction review changes grounded original producers')
    exact(review['reviewer'], reviewer, 'construction reviewer is not separately confirmed')
    exact(review['pages_covered_by_original_review'], pages, 'construction review omits original pages')
    require(isinstance(review['original_page_review_basis'], str) and review['original_page_review_basis'].strip()
            and review['later_crosswalk_reviewed_after_annotation_freeze'] is True, 'construction/page review timing is absent')
    exact(review['complete_visual_counts'], construction['complete_visual_counts'], 'review visual counts differ')
    exact(review['source_visual_exclusions'], construction['source_visual_exclusions'], 'review omits syntactic visual source roles')
    exact(review['omitted_metrics'], OMITTED_METRICS, 'visual-only review grants unscored metrics')
    require(not any(key in candidate for key in ('manual_figure_table_overlay', 'manual_object_overlay',
            'manual_bibliography_overlay', 'manual_visual_only_overlay', 'independent_panel')),
            'visual-only projection cannot mix other manual overlays')
    overlay.update(evidence_format=FORMAT, evidence_hashes={key: sha256(raw) for key, raw in raws.items()},
        historical_source_inventory_sha256=candidate['source_inventory_sha256'],
        current_source_inventory_sha256=sha256(canonical(inventory)),
        historical_source_difference_fields=construction['changed_source_inventory_fields'],
        current_source_diagnostics=deepcopy(inventory['coverage']),
        supplemental_unscored_history=deepcopy(docs['supplement_history']), final_k1_publication=False)
    result = deepcopy(candidate); result['manual_visual_only_overlay'] = overlay
    return result


def attach_visual_only(cache, corpus_root, data_root, candidate_raw, paper, mapped, relative):
    """Read only declared immutable cache evidence and original corpus inputs."""
    from builder import fingerprint_file, safe_file
    from manual import bounded
    manifest_raw = bounded(cache, relative + '/manifest.json'); manifest = document(manifest_raw)
    require(manifest.get('format') == FORMAT and type(manifest.get('schema_version')) is int
            and manifest['schema_version'] == 1 and set(manifest['artifacts']) == ARTIFACTS,
            'unsupported visual-only evidence manifest')
    raws = {}; total = 0
    for name, row in manifest['artifacts'].items():
        require(set(row) == {'path', 'sha256', 'bytes'}, 'visual-only artifact declaration differs')
        raw = bounded(cache, row['path']); total += len(raw)
        require(total <= MAX_BUNDLE_BYTES, 'visual-only evidence exceeds cumulative byte limit')
        hash_ref(row, raw, 'visual-only artifact hash/size differs'); raws[name] = raw
    docs = {key: document(raw) for key, raw in raws.items() if key != 'prompt'}
    candidate = document(candidate_raw); packet = docs['packet']
    exact(candidate['index'], mapped['index'], 'visual-only candidate changes mapped native index')
    exact(candidate['paper_id'], mapped['paper_id'], 'visual-only candidate changes mapped paper')
    exact(candidate['arxiv_id'], paper['arxiv_id'], 'visual-only candidate changes original arXiv identity')
    exact(packet['version'], paper['version'], 'visual-only packet changes original pinned version')
    exact(packet['stratum'], paper['stratum'], 'visual-only packet changes original selection stratum')
    index_raw = bounded(data_root, mapped['index']['path'])
    require(sha256(index_raw) == mapped['index']['sha256'] and mapped['pdf_sha256'] == paper['pdf']['sha256'],
            'mapped native index or PDF identity differs')
    source_raw = None
    for kind in ('pdf', 'source'):
        path = safe_file(corpus_root, paper[kind]['path']); digest, size = fingerprint_file(path, cap=64*1024*1024)
        require(digest == paper[kind]['sha256'] == candidate[kind + '_sha256'] and size == paper[kind]['bytes'],
                'frozen original corpus artifact differs')
        exact(packet[kind], paper[kind], 'original packet changes corpus receipt')
        if kind == 'source':
            source_raw = path.read_bytes()
            require(len(source_raw) == size and sha256(source_raw) == digest, 'source archive changed during read')
    exact(packet['selection_sha256'], sha256(raws['selection']), 'visual-only packet changes frozen selection')
    exact(packet['policy_sha256'], sha256(raws['policy']), 'visual-only packet changes frozen policy')
    selected = [row for row in docs['selection']['selected'] if row['arxiv_id'] == paper['arxiv_id']]
    require(len(selected) == 1, 'visual-only paper is absent or repeated in frozen blind selection')
    exact(selected[0]['mapping'], mapped, 'visual-only selection changes native mapping')
    annotation_root = safe_file(cache, manifest['artifacts']['policy']['path']).parent
    pr, ir = docs['primary_receipt'], docs['independent_receipt']
    original_rows = pr.get('inputs', pr.get('input_files')) + ir.get('inputs', ir.get('inputs_rehashed'))
    original_rows = original_rows + [pr['inventory']] + packet['images']
    if 'inventory' in ir: original_rows.append(ir['inventory'])
    supplements = docs['supplement_history']
    require(supplements.get('status') == 'retained_unscored_additive_evidence'
            and isinstance(supplements.get('artifacts'), list) and len(supplements['artifacts']) <= 256,
            'supplement history is missing, unsupported or unbounded')
    original_rows += supplements['artifacts']
    identity = docs['identity_review']; request = docs['assignment_request']
    require(isinstance(identity['papers'], list) and len(identity['papers']) <= 32, 'identity paper map exceeds its bound')
    original_rows += [row for identity_paper in identity['papers'] for row in identity_paper['original_artifacts'].values()]
    for row in original_rows: verify_original_file(cache, annotation_root, row)
    images = {row['path']: verify_original_file(cache, annotation_root, row) for row in packet['images']}
    verified_history = assignment_records(cache, docs['assignment_history'])
    history = request['git_history']; history_path = Path(history['retained_path'])
    # Original root receipts can retain absolute cache paths. Convert only
    # inside this declared cache; safe_file still rejects traversal/symlinks.
    history_relative = str(history_path.relative_to(cache)) if history_path.is_absolute() else str(history_path)
    history_raw = bounded(cache, history_relative)
    hash_ref(history, history_raw, 'retained original assignment history differs')
    require(re.fullmatch(r'[0-9a-f]{40}', history['git_commit']) is not None
            and history['git_path'] == 'docs/knowledge-base-phases.md', 'unsupported assignment history origin')
    verified_history[history['retained_path']] = sha256(history_raw)
    history_text = history_raw.decode('utf-8')
    for row in request['papers']:
        for occurrence in row['phase_exact_hash_occurrences']:
            a, b = occurrence['start'], occurrence['end']
            require(occurrence['unit'] == 'Unicode code points' and type(a) is int and type(b) is int
                    and 0 <= a < b <= len(history_text), 'historical assignment excerpt bounds differ')
            exact(history_text[a:b], occurrence['text'], 'historical assignment excerpt differs')
    result = validate_visual_only(candidate_raw, index_raw, source_raw, raws, images, verified_history)
    for name, row in manifest['artifacts'].items():
        require(bounded(cache, row['path']) == raws[name], 'visual-only artifact changed during validation')
    for row in original_rows: verify_original_file(cache, annotation_root, row)
    require(bounded(cache, history_relative) == history_raw, 'assignment history changed during validation')
    current_history = assignment_records(cache, docs['assignment_history'])
    current_history[history['retained_path']] = sha256(history_raw)
    exact(current_history, verified_history, 'retained running assignment changed during validation')
    require(bounded(cache, relative + '/manifest.json') == manifest_raw, 'visual-only manifest changed during validation')
    result['manual_assembly'] = {'format': FORMAT, 'manifest_sha256': sha256(manifest_raw),
        'candidate_sha256': sha256(candidate_raw), 'final_k1_publication': False,
        'omitted_manual_kinds': ['equation', 'statement', 'proof', 'algorithm', 'bib_entry'],
        'omitted_metrics': OMITTED_METRICS.copy(),
        'source_inventory_policy': 'All original/current roles retained unscored; complete reviewed visual projection only.'}
    return result
