#!/usr/bin/env python3
"""Measure production cached figure/table objects against frozen independent K1."""
import argparse
from collections import Counter
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import statistics
import struct
import subprocess
import sys
import tempfile
import time
import types

ROOT = Path(__file__).resolve().parents[2]
TRUTH = 'eval/truth/k1-limited-v1/objects.json'
CONFIG = 'eval/truth/k1-limited-v1-build.json'
TRACE = 'eval/inputs/evidence/object-metrics.json'
INPUT = 'eval/inputs/objects/figure-table.json'
KINDS = ('figure', 'table')
VERSION = 'figure-table-metrics-v6'
TRUTH_VERSIONS = ('k1-limited-v1', 'k1-limited-v2', 'k1-limited-v3', 'k1-limited-v4', 'k1-limited-v5')
BOUNDED_TRUTH_VERSION = 'k1-limited-v5'
NATIVE_BASIS_FORMAT = 'native-json-f32-v1'
DETECTOR_VERSION = 6
GRAPHICS_VERSION = 5
MAX_JSON = 32 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False, allow_nan=False).encode()


def read(path, cap=MAX_JSON):
    path = Path(path).absolute()
    require('..' not in path.parts and not any(part == '.secrets' or part == '.env' or part.startswith('.env.') for part in path.parts), 'unsafe evidence path')
    require(not any(part.is_symlink() for part in (path, *path.parents)), 'symlinked evidence path')
    require(path.is_file() and path.stat().st_size <= cap, 'evidence is missing or oversized')
    with path.open('rb') as stream:
        raw = stream.read(cap + 1)
    require(len(raw) <= cap, 'evidence grew beyond byte bound')
    return raw


def document(raw):
    def pairs(rows):
        result = {}
        for key, value in rows:
            require(key not in result, 'duplicate JSON key')
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=lambda _: require(False, 'nonfinite JSON number'))


def label(value, kind):
    if not isinstance(value, str):
        return None
    prefix = r'(?:fig(?:ure)?\.?)' if kind == 'figure' else r'table'
    value = re.sub(r'^' + prefix + r'\s*', '', value.strip(), flags=re.I)
    value = value.strip(' .:()').lower()
    return value if re.fullmatch(r'(?:[a-z]?[0-9]+(?:\.[0-9]+)*[a-z]?|[ivxlcdm]+)', value, flags=re.ASCII) else None


def utf16_members(index, spans, page=None, encoded=None):
    """Exact non-whitespace scalar membership; reject split surrogates and cross-page anchors."""
    encoded = index['text'].encode('utf-16-le') if encoded is None else encoded
    require(len(encoded) <= 8 * 1024 * 1024, 'native text exceeds index bound')
    require(isinstance(spans, list) and len(spans) <= 100, 'invalid caption spans')
    selected = set(); pages = set(); consumed = 0
    for span in spans:
        start, end = span.get('start'), span.get('end')
        require(type(start) is int and type(end) is int and 0 <= start < end <= len(encoded)//2, 'invalid UTF-16 caption boundary')
        consumed += end-start
        require(consumed <= 20000, 'caption membership exceeds bound')
        try: text = encoded[start*2:end*2].decode('utf-16-le')
        except UnicodeDecodeError as error: raise ValueError('split UTF-16 scalar') from error
        owners = [p for p in index['pages'] if p['start'] <= start < end <= p['end']]
        require(len(owners) == 1 and owners[0]['provenance'] == 'native', 'caption lacks a unique native page')
        require(page is None or owners[0]['number'] == page, 'caption page differs')
        require('page' not in span or span['page'] == owners[0]['number'], 'caption anchor page differs')
        offset=start
        for char in text:
            width=2 if ord(char)>0xffff else 1
            if not char.isspace(): selected.update(range(offset,offset+width))
            offset+=width
        pages.add(owners[0]['number'])
    require(selected, 'caption has no authored membership')
    return selected, pages


def rectangle(value, page):
    if not isinstance(value, dict):
        return None
    keys = ('x_min','y_min','x_max','y_max')
    coords = [value.get(k) for k in keys]
    if not all(type(v) in (int, float) and math.isfinite(v) for v in coords):
        return None
    x0,y0,x1,y1 = coords
    if not (0 <= x0 < x1 <= page['width'] and 0 <= y0 < y1 <= page['height']):
        return None
    return x0,y0,x1,y1


def union_area(rectangles):
    """Exact sweep-line area for bounded axis-aligned unions."""
    require(len(rectangles) <= 1000, 'too many region rectangles')
    xs = sorted({v for rect in rectangles for v in (rect[0],rect[2])})
    area = 0.0
    for a,b in zip(xs,xs[1:]):
        intervals = sorted((r[1],r[3]) for r in rectangles if r[0] < b and r[2] > a)
        height = 0.0; end = -math.inf
        for start,stop in intervals:
            height += max(0, stop-max(start,end)); end = max(end,stop)
        area += (b-a)*height
    return area


def iou(truth, predicted):
    if predicted is None:
        return 0.0
    intersections = []
    for r in truth:
        q = (max(r[0],predicted[0]), max(r[1],predicted[1]), min(r[2],predicted[2]), min(r[3],predicted[3]))
        if q[0] < q[2] and q[1] < q[3]: intersections.append(q)
    intersection = union_area(intersections)
    return intersection/(union_area(truth)+union_area([predicted])-intersection)


def evaluate_paper(paper, artifact, index, truth_version=TRUTH_VERSIONS[0]):
    require(truth_version in TRUTH_VERSIONS, 'unsupported collector truth version')
    require(paper['metric_eligibility'].get('O1') is True
            or truth_version == BOUNDED_TRUTH_VERSION and paper['metric_eligibility'].get('O2') is True,
            'paper is outside complete visual cohorts')
    require(artifact['paper_id'] == paper['paper_id'] and artifact['reading_index_generation'] == '"'+paper['index']['sha256']+'"', 'wrong object paper or index generation')
    require(len(artifact['objects']) <= 2000 and len(paper['objects']) <= 2000, 'object population exceeds bound')
    pages = {p['number']:p for p in index['pages']}
    require(len(pages)==len(index['pages']), 'duplicate native pages')
    for p in pages.values():
        require(all(type(p[k]) in (int,float) and math.isfinite(p[k]) and p[k]>0 for k in ('width','height')), 'invalid page dimensions')
    truths = [o for o in paper['objects'] if o['kind'] in KINDS]
    require(len({o['id'] for o in truths}) == len(truths), 'duplicate truth identity')
    for kind in KINDS:
        actual = sum(o['kind']==kind for o in truths)
        require(actual==paper['counts'].get(kind,0), 'truth kind inventory differs')
        require(actual>0 or kind in paper['reviewed_absent_kinds'], 'missing complete negative-kind review')
    encoded = index['text'].encode('utf-16-le')
    require(sum(span['end']-span['start'] for o in truths for span in o['spans'])<=200000, 'truth caption membership exceeds paper bound')
    truth_info = []
    for o in truths:
        members, owned = utf16_members(index,o['spans'],encoded=encoded)
        require(len(owned)==1 and label(o['printed_label'],o['kind']) is not None, 'ambiguous truth caption identity')
        truth_info.append((members,next(iter(owned))))
    predictions = [o for o in artifact['objects'] if o['kind'] in KINDS]
    choices = {}; errors = {}; ambiguous = []
    for n,o in enumerate(predictions):
        try: members,_ = utf16_members(index,[o['anchor']],o['page'],encoded)
        except (ValueError,KeyError,TypeError): errors[n]='invalid_caption_anchor';continue
        edges = []
        for j,t in enumerate(truths):
            known,page = truth_info[j]
            if o['kind']!=t['kind'] or o['page']!=page or label(o.get('label'),o['kind'])!=label(t['printed_label'],t['kind']): continue
            overlap = len(members & known)
            if 2*overlap >= len(members) and 2*overlap >= len(known):
                edges.append((j,2*overlap/(len(members)+len(known))))
        if len(edges)>1: ambiguous.append(n)
        elif edges: choices[n]=edges[0]
    winners = {}
    for n,(j,score) in choices.items():
        if j not in winners or score>winners[j][1]: winners[j]=(n,score)
    matched = {n for n,_ in winners.values()}; values=[]; matched_values=[]; outcomes=[]; unknown=0
    for j,t in enumerate(truths):
        n,score = winners.get(j,(None,None)); regions=t.get('region')
        if truth_version in ('k1-limited-v3', 'k1-limited-v4', BOUNDED_TRUTH_VERSION) and isinstance(regions,dict):
            require(set(regions)=={'page','rect'} and type(regions['page']) is int
                    and isinstance(regions['rect'],dict)
                    and set(regions['rect'])=={'x_min','y_min','x_max','y_max'},
                    'invalid singleton truth region')
            regions=[regions]
        annotated = isinstance(regions,list) and bool(regions)
        if not annotated: unknown+=1
        validated=[]
        if annotated:
            require(len(regions)<=1000,'truth region bound exceeded')
            for region in regions:
                page=pages.get(region['page']);rect=rectangle(region.get('rect'),page) if page else None
                require(rect is not None, 'invalid independently annotated truth geometry')
                validated.append((region['page'],rect))
        outcome={'truth_id':t['id'],'kind':t['kind'],'prediction_position':n,'caption_dice':score,'iou':None}
        if paper['metric_eligibility'].get('O2') and annotated:
            pred=predictions[n] if n is not None else None
            rect=rectangle(pred.get('region'),pages[pred['page']]) if pred is not None else None
            # Prediction is one page; regions on all other pages remain in the union denominator.
            same=[r for page,r in validated if pred is not None and page==pred['page']]
            area=sum(union_area([r for p,r in validated if p==page]) for page in {p for p,_ in validated})
            if rect is None: value=0.0
            else:
                intersection=union_area([(max(r[0],rect[0]),max(r[1],rect[1]),min(r[2],rect[2]),min(r[3],rect[3])) for r in same if max(r[0],rect[0])<min(r[2],rect[2]) and max(r[1],rect[1])<min(r[3],rect[3])])
                value=intersection/(area+union_area([rect])-intersection)
            values.append(value); outcome['iou']=value
            if pred is not None: matched_values.append(value)
        outcomes.append(outcome)
    tp=len(winners);fp=len(predictions)-tp;fn=len(truths)-tp
    by_kind={kind:{'tp':sum(truths[j]['kind']==kind for j in winners),'fp':sum(o['kind']==kind and n not in matched for n,o in enumerate(predictions)),'fn':sum(t['kind']==kind and j not in winners for j,t in enumerate(truths))} for kind in KINDS}
    return {'paper_id':paper['paper_id'],'tp':tp,'fp':fp,'fn':fn,'by_kind':by_kind,'truth_objects':len(truths),'predictions':len(predictions),'region_values':values,'matched_region_values':matched_values,'unknown_truth_regions':unknown,'excluded_O2':not paper['metric_eligibility'].get('O2'),'ambiguous_prediction_positions':ambiguous,'invalid_caption_anchors':errors,'unmatched_prediction_positions':[n for n in range(len(predictions)) if n not in matched],'outcomes':outcomes}


def aggregate_metrics(truth, metrics):
    totals={k:sum(p[k] for p in metrics) for k in ('tp','fp','fn','truth_objects','predictions','unknown_truth_regions')}
    values=[v for p in metrics for v in p['region_values']]
    matched=[v for p in metrics for v in p['matched_region_values']]
    expected=truth['coverage']['denominators']['O2']['all_annotated_truth_objects']
    require(type(expected) is int and len(values)==expected, 'incomplete frozen O2 region denominator')
    require(totals['truth_objects']>0 and values,'empty objective denominator')
    return totals, values, matched


def aggregate_layout_metrics(truth, decisions):
    """Stream complete ordered decisions; count only each metric's declared cohort."""
    totals = dict.fromkeys(('tp', 'fp', 'fn', 'truth_objects', 'predictions', 'unknown_truth_regions'), 0)
    values = []; matched = []; cohorts = {'O1': [], 'O2': []}
    for descriptor, decision in zip(truth['papers'], decisions, strict=True):
        require(decision['paper_id'] == descriptor['paper_id']
                and decision['metric_eligibility'] == descriptor['metric_eligibility'], 'decision population differs')
        eligible = descriptor['metric_eligibility']
        if not (eligible['O1'] or eligible['O2']):
            require(decision['status'] == 'not_in_visual_cohort' and decision['outcomes'] == [],
                    'ineligible paper claims measured outcomes')
            continue
        require(decision['status'] == 'measured', 'visual cohort paper was not measured')
        row = decision['metrics']
        require(row['paper_id'] == descriptor['paper_id'], 'measured decision paper differs')
        require(type(row['truth_objects']) is int and row['truth_objects'] >= 0
                and row['truth_objects'] == descriptor['counts'].get('figure', 0) + descriptor['counts'].get('table', 0),
                'per-paper truth denominator differs')
        if eligible['O1']:
            cohorts['O1'].append(descriptor['paper_id'])
            for key in totals:
                require(type(row[key]) is int and row[key] >= 0, 'invalid metric count')
                totals[key] += row[key]
        if eligible['O2']:
            cohorts['O2'].append(descriptor['paper_id'])
            require(len(values) + len(row['region_values']) <= 100000, 'global metric value bound exceeded')
            require(all(type(v) in (int, float) and math.isfinite(v) and 0 <= v <= 1
                        for v in row['region_values']), 'invalid region metric')
            require(len(row['region_values']) == row['truth_objects']
                    and len(row['matched_region_values']) <= row['truth_objects']
                    and all(type(v) in (int, float) and math.isfinite(v) and 0 <= v <= 1
                            for v in row['matched_region_values']), 'incomplete per-paper region denominator')
            values.extend(row['region_values']); matched.extend(row['matched_region_values'])
    expected = truth['coverage']['denominators']
    require(all(cohorts[m] == truth['coverage']['cohort_papers'][m] for m in cohorts), 'metric cohort is incomplete')
    require(totals['truth_objects'] == expected['O1']['truth_objects'] > 0,
            'incomplete frozen O1 denominator')
    require(len(values) == expected['O2']['all_annotated_truth_objects'] > 0, 'incomplete frozen O2 denominator')
    return totals, values, matched


def validate_registry(registry, papers, corpus_pdf_root):
    require(registry['schema_version']==1 and registry['library_root']==str(corpus_pdf_root), 'registry root or schema differs')
    records=registry['records'];active=Counter(r['relative_path'] for r in records.values() if r['active'])
    for paper in papers:
        row=records.get(paper['paper_id']);name=paper['arxiv_id']+'v'+str(paper['arxiv_version'])+'.pdf'
        require(row is not None and row['active'] is True and row['relative_path']==name and row['content_hash']==paper['pdf_sha256'], 'canonical registry identity differs')
        require(active[name]==1 and paper['paper_id'] not in {ident for ids in registry.get('unresolved',{}).values() for ident in ids}, 'canonical registry identity is ambiguous')


def truth_verifier(repo):
    # Load the application adapter by its exact path; historical parser modules
    # are loaded only inside its isolated, manifest-verified worker.
    path=repo/'scripts/truth/latex/versioned.py'
    verifier=types.ModuleType('k1_versioned_verifier');verifier.__file__=str(path)
    exec(compile(read(path),str(path),'exec'),verifier.__dict__)
    return verifier


def validate_truth(repo, cache, corpus, data, truth_raw, with_receipt=False):
    """Replay the selected immutable release without using current parser modules."""
    truth=document(truth_raw)
    require(truth['schema_version'] == (2 if truth['version'] == BOUNDED_TRUTH_VERSION else 1)
            and truth['truth_set']=='K1' and truth['origin']=='arxiv-latex', 'unsupported frozen K1 identity')
    # Current detector output is rederived from immutable native coordinates.
    # validate_derivation binds that separately versioned generation below.
    rebuilt,config,receipt=truth_verifier(repo).replay(repo,cache,corpus,data,truth['version'])
    require(canonical(rebuilt)+b'\n'==truth_raw, 'K1 labels or complete cohort differ from independent evidence')
    return (truth,config,receipt) if with_receipt else (truth,config)


def canonical_native_float(value):
    require(math.isfinite(value),'nonfinite native float')
    try:raw=struct.pack('>f',value)
    except (OverflowError,struct.error) as error:raise ValueError('native float exceeds f32') from error
    typed=struct.unpack('>f',raw)[0]
    require(math.isfinite(typed),'native float exceeds f32')
    if value!=typed:
        # Native schema6 serializes f32 with the shortest round-tripping decimal.
        # Accept that representation, but reject arbitrary extra f64 precision.
        shortest=None
        for precision in range(1,10):
            candidate=float(format(typed,'.'+str(precision)+'g'))
            if struct.pack('>f',candidate)==raw:
                shortest=candidate;break
        require(value==shortest,'unsupported noncanonical native float')
    return typed


def native_value_digest(value, normalize=False):
    """Tagged/length-framed canonical native values; all floats are exact f32."""
    result=hashlib.sha256(b'lysilogy-native-basis-v1\0')
    def framed(tag,raw):result.update(tag+struct.pack('>Q',len(raw))+raw)
    def visit(item,depth):
        require(depth<=64,'native basis nesting exceeds64')
        if item is None:result.update(b'n')
        elif type(item) is bool:result.update(b't' if item else b'f')
        elif type(item) is int:
            require(-(1<<63)<=item<(1<<64),'native integer exceeds supported range')
            framed(b'i',str(item).encode())
        elif type(item) is float:
            value=canonical_native_float(item) if normalize else item
            require(math.isfinite(value),'nonfinite native float')
            try:raw=struct.pack('>f',value)
            except (OverflowError,struct.error) as error:raise ValueError('native float exceeds f32') from error
            require(struct.unpack('>f',raw)[0]==value,'native value is not exact f32')
            result.update(b'r'+raw)
        elif type(item) is str:framed(b's',item.encode('utf-8'))
        elif type(item) is list:
            result.update(b'a'+struct.pack('>Q',len(item)))
            for child in item:visit(child,depth+1)
        elif type(item) is dict:
            require(all(type(key) is str for key in item),'native object keys must be strings')
            result.update(b'o'+struct.pack('>Q',len(item)))
            for key in sorted(item,key=lambda k:k.encode('utf-8')):visit(key,depth+1);visit(item[key],depth+1)
        else:raise ValueError('unsupported native value')
    visit(value,0)
    return result.hexdigest()


def native_basis_digest(index):
    require(type(index.get('schema_version')) is int and index['schema_version']==6,'unsupported native basis schema')
    require(set(index)=={'schema_version','text','pages','tokens','objects','gaps','figures'},'unsupported native basis fields')
    return native_value_digest({key:value for key,value in index.items() if key!='figures'},normalize=True)


def validate_derivation(row, artifact, index):
    """Bind current production predictions to the exact frozen native basis."""
    require(row.get('native_basis_format')==NATIVE_BASIS_FORMAT and type(row.get('native_schema_version')) is int and row['native_schema_version']==6,'unsupported native commitment format/schema')
    require(row['native_basis_sha256']==native_basis_digest(index),'detector native text, tokens, geometry or provenance differs')
    require(type(artifact.get('figure_detector_version')) is int and artifact['figure_detector_version']==DETECTOR_VERSION,'unsupported current detector version')
    generation_input='figures:'+str(DETECTOR_VERSION)+':'+artifact['reading_index_generation']
    if artifact.get('graphics') is not None: generation_input+=':graphics:'+artifact['graphics']['generation']
    generation=digest(generation_input.encode())
    require(artifact.get('figure_detector_generation')==generation,'derived detector generation differs')
    return {'version':DETECTOR_VERSION,'generation':generation,'native_basis_format':NATIVE_BASIS_FORMAT,'native_schema_version':6,'native_basis_sha256':row['native_basis_sha256'],'index_sha256':row['index_sha256']}


def mask_runtime():
    wrapper=Path('/usr/bin/prlimit')
    if sys.platform!='linux' or not wrapper.is_file(): return None
    wrapper=wrapper.resolve()
    return {'wrapper_path':str(wrapper),'wrapper_sha256':digest(read(wrapper,8*1024*1024)),
            'script_sha256':digest(read(ROOT/'src/source_index/graphics/masks.js',64*1024))}


def mask_runtime_key(runtime):
    return [runtime[key] for key in ('wrapper_path','wrapper_sha256','script_sha256')] if runtime else None


def validate_vector_raster(raw, evidence, native):
    """Validate the retained renderer format and bounded component records.

    The Cargo-bound producer owns flood-fill derivation; this check binds its
    complete component output to the exact full-page RGB artifact, never a crop.
    """
    width,height=evidence['width'],evidence['height']
    require(all(type(n) is int and 0<n<=8192 for n in (width,height)) and width*height<=4194304,'vector pixel bound differs')
    require(width==math.ceil(native['width']*2) and height==math.ceil(native['height']*2),'vector native dimensions differ')
    end=raw.find(b'ENDHDR\n',0,1031)
    require(0<=end<=1024,'vector PAM header missing')
    lines=raw[:end].splitlines();require(lines and lines[0]==b'P7','vector PAM magic differs')
    pairs=[line.split() for line in lines[1:]]
    require(all(len(pair)==2 for pair in pairs) and len(pairs)==5,'vector PAM fields differ')
    header=dict(pairs)
    require(len(header)==5 and header=={b'WIDTH':str(width).encode(),b'HEIGHT':str(height).encode(),b'DEPTH':b'3',b'MAXVAL':b'255',b'TUPLTYPE':b'RGB'},'vector PAM identity differs')
    require(len(raw)-end-7==width*height*3,'vector PAM sample length differs')
    components=evidence['components'];require(isinstance(components,list) and len(components)<=4096,'vector component bound differs')
    total=0
    for component in components:
        require(isinstance(component,dict) and set(component)=={'bounds','pixels'},'vector component fields differ')
        bounds=component['bounds'];count=component['pixels']
        require(isinstance(bounds,list) and len(bounds)==4 and all(type(n) is int for n in bounds),'vector component coordinates differ')
        x0,y0,x1,y1=bounds
        require(0<=x0<x1<=width and 0<=y0<y1<=height,'vector component outside page')
        require(type(count) is int and 0<count<=(x1-x0)*(y1-y0),'vector component pixel count differs')
        total+=count
    require(total<=width*height,'vector component inventory exceeds page')


def validate_vectors(row, pages, index, cache, runtime, total):
    rasters=row['graphics_vectors']
    require(isinstance(rasters,list) and len(rasters)<=64,'invalid vector raster inventory')
    require(all(isinstance(r,dict) and set(r)=={'page','sha256','path'} and type(r['page']) is int for r in rasters),'invalid vector raster binding')
    by_page={r['page']:r for r in rasters};retained={};statuses=Counter()
    require(len(by_page)==len(rasters),'duplicate vector raster')
    expected={page['page'] for page in pages if page.get('vectors') is not None and page['vectors']['raster_sha256'] is not None}
    require(set(by_page)==expected,'vector raster coverage differs')
    allowed={'complete','unsupported_renderer_trace','native_gap_or_unavailable','pixel_limit','component_limit','unsupported_raster','resource_wrapper_unavailable','resource_limit','tool_failed'}
    for page in pages:
        evidence=page.get('vectors')
        if evidence is None: continue
        require(isinstance(evidence,dict) and set(evidence)=={'status','raster_sha256','dpi','contrast','width','height','components','diagnostic'},'vector evidence fields differ')
        status=evidence['status'];statuses[status]+=1
        require(status in allowed and type(evidence['dpi']) is int and evidence['dpi']==144 and type(evidence['contrast']) is int and evidence['contrast']==5,'vector policy differs')
        require(evidence['diagnostic'] is None or isinstance(evidence['diagnostic'],str) and len(evidence['diagnostic'])<=1024,'vector diagnostic exceeds bound')
        require(page['trace_sha256'] is not None,'vector evidence lacks trace')
        if status!='complete':
            require(evidence['components']==[] and evidence['width'] is None and evidence['height'] is None,'unavailable vector page admitted support')
        sha=evidence['raster_sha256']
        if sha is None:
            require(status not in {'complete','component_limit','unsupported_raster'},'vector raster missing');continue
        require(runtime is not None and status in {'complete','component_limit','unsupported_raster','resource_limit'},'vector raster status/runtime differs')
        require(isinstance(sha,str) and re.fullmatch('[0-9a-f]{64}',sha),'invalid vector raster hash')
        saved=by_page[page['page']];path=cache/'object-graphics-vectors'/(sha+'.pam')
        require(saved['sha256']==sha and saved['path']==str(path),'vector raster path/hash differs')
        raw=read(path,16*1024*1024);total+=len(raw)
        require(digest(raw)==sha and total<=64*1024*1024,'vector raster bytes differ or exceed bound')
        if status=='complete':
            native=next(native for native in index['pages'] if native['number']==page['page'])
            require(native['provenance']=='native' and not any(gap['page']==page['page'] for gap in index.get('gaps',[])),'vector page lacks complete native basis')
            require(page['images']==[] and page['unsupported_images']==0 and page.get('mask') is None,'vector page broadened an image path')
            validate_vector_raster(raw,evidence,native)
        retained[str(path)]=sha
    return retained,dict(statuses)


def validate_graphics(row, artifact, paper, index, cache):
    evidence=artifact.get('graphics')
    require(isinstance(evidence,dict) and type(evidence.get('version')) is int and evidence['version']==GRAPHICS_VERSION,'current source factory omitted graphics evidence')
    basis_raw=row['graphics_basis_json'].encode();basis=document(basis_raw)
    require(basis==dict(evidence,generation='') and digest(basis_raw)==evidence['generation'],'graphics generation differs')
    require(evidence['native_generation']==artifact['reading_index_generation'] and evidence['pdf_sha256']==paper['pdf_sha256'],'graphics native/PDF identity differs')
    expected_tool=shutil.which('mutool')
    expected_tool=str(Path(expected_tool).resolve()) if expected_tool else None
    require(evidence.get('tool_path')==expected_tool,'graphics tool path differs')
    require(evidence.get('tool_sha256')==(digest(read(Path(expected_tool),128*1024*1024)) if expected_tool else None),'graphics executable changed')
    runtime=mask_runtime();require(evidence.get('mask_runtime')==runtime,'mask runtime provenance differs')
    require(evidence['cache_key']==digest(canonical([GRAPHICS_VERSION,evidence['native_generation'],evidence['pdf_sha256'],evidence['tool_sha256'],mask_runtime_key(runtime)])),'graphics cache identity differs')
    pages=evidence['pages'];require(isinstance(pages,list) and len(pages)<=400,'invalid graphics page inventory')
    page_ids=[page['page'] for page in pages]
    require(all(type(number) is int and 1<=number<=400 for number in page_ids),'invalid graphics page number')
    require(page_ids==sorted(set(page_ids)) and set(page_ids)=={obj['page'] for obj in artifact['objects'] if obj['kind'] in KINDS},'graphics page denominator differs')
    traces=row['graphics_traces'];require(isinstance(traces,list) and len(traces)<=64,'invalid graphics trace inventory')
    require(len({trace['page'] for trace in traces})==len(traces),'duplicate graphics trace')
    by_page={trace['page']:trace for trace in traces};total=0;retained={}
    require(set(by_page)=={page['page'] for page in pages if page['trace_sha256'] is not None},'graphics trace coverage differs')
    allowed={'complete','partial','unsupported_trace','resource_limit','tool_failed','tool_unavailable'}
    for page in pages:
        require(expected_tool is not None or page['status'] in {'tool_unavailable','resource_limit'},'trace recorded without a graphics tool')
        require(page['status'] in allowed and type(page['unsupported_images']) is int and 0<=page['unsupported_images']<=256,'invalid graphics status')
        require(page['status']!='complete' or page['unsupported_images']==0,'complete graphics page has unsupported images')
        require(page['status']!='partial' or page['unsupported_images']>0,'partial graphics page lacks exclusions')
        native=[native for native in index['pages'] if native['number']==page['page']]
        require(len(native)==1,'graphics page lacks unique native owner')
        require(isinstance(page['images'],list) and len(page['images'])+page['unsupported_images']<=256,'graphics image inventory exceeds bound')
        require(all(rectangle(rect,native[0]) is not None for rect in page['images']),'graphics placement is invalid')
        require(page['status'] in {'complete','partial'} or not page['images'],'unsupported graphics state admitted placements')
        if page['trace_sha256'] is None:
            require(page['status'] not in {'complete','partial','unsupported_trace'},'graphics trace receipt missing')
            continue
        trace=by_page[page['page']];sha=page['trace_sha256']
        require(isinstance(sha,str) and re.fullmatch('[0-9a-f]{64}',sha) and trace['sha256']==sha,'graphics trace hash differs')
        expected=cache/'object-graphics-traces'/(sha+'.xml')
        require(trace['path']==str(expected),'graphics trace path differs')
        raw=read(expected,16*1024*1024);total+=len(raw)
        require(digest(raw)==sha and total<=64*1024*1024,'graphics trace bytes differ or exceed bound')
        retained[str(expected)]=sha
    masks=row['graphics_masks'];require(isinstance(masks,list) and len(masks)<=64,'invalid mask receipt inventory')
    require(len({mask['page'] for mask in masks})==len(masks),'duplicate mask receipt')
    by_page={mask['page']:mask for mask in masks};mask_hashes={}
    require(set(by_page)=={page['page'] for page in pages if page.get('mask') is not None and page['mask']['receipt_sha256'] is not None},'mask receipt coverage differs')
    for page in pages:
        mask=page.get('mask')
        if mask is None: continue
        require(mask['status'] in {'evaluated','unsupported_receipt','tool_failed','tool_unavailable','resource_limit'},'invalid mask status')
        require(all(type(mask[key]) is int and 0<=mask[key]<=256 for key in ('supported_images','empty_images')),'invalid mask counts')
        require(mask['supported_images']<=len(page['images']) and len(page['images'])+page['unsupported_images']+mask['empty_images']<=256,'mask image accounting differs')
        require(mask['status']=='evaluated' or mask['supported_images']==mask['empty_images']==0,'unevaluated mask admitted images')
        require(runtime is not None or mask['status'] in {'tool_unavailable','resource_limit'},'mask data lacks runtime')
        sha=mask['receipt_sha256']
        if sha is None:
            require(mask['status'] not in {'evaluated','unsupported_receipt'},'mask receipt missing');continue
        require(mask['status'] in {'evaluated','unsupported_receipt'} and isinstance(sha,str) and re.fullmatch('[0-9a-f]{64}',sha),'invalid mask receipt state/hash')
        saved=by_page[page['page']];expected=cache/'object-graphics-masks'/(sha+'.json')
        require(saved['sha256']==sha and saved['path']==str(expected),'mask receipt path/hash differs')
        raw=read(expected,16*1024*1024);total+=len(raw)
        require(digest(raw)==sha and total<=64*1024*1024,'mask receipt bytes differ or exceed bound')
        if mask['status']=='evaluated':
            decoded=document(raw);require(decoded['schema_version']==1 and decoded['page']==page['page'],'mask decoded identity differs')
        mask_hashes[str(expected)]=sha
    vector_hashes,vector_statuses=validate_vectors(row,pages,index,cache,runtime,total)
    return {'vector_hashes':vector_hashes,'vector_statuses':vector_statuses,'generation':evidence['generation'],'cache_key':evidence['cache_key'],'tool_sha256':evidence['tool_sha256'],'mask_runtime':runtime,'page_statuses':dict(Counter(page['status'] for page in pages)),'trace_hashes':retained,'mask_hashes':mask_hashes}


def atomic_json(path, value):
    raw=canonical(value)+b'\n'
    require(len(raw)<=8*1024*1024,'collector output exceeds harness bound')
    require(not any(p.is_symlink() for p in (path,*path.parents)), 'symlinked collector output')
    path.parent.mkdir(parents=True,exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent,prefix=path.name+'.',delete=False) as stream:
        pending=Path(stream.name)
        try:
            stream.write(raw);stream.flush();os.fsync(stream.fileno());os.replace(pending,path)
        finally:pending.unlink(missing_ok=True)


def release_paths(version):
    require(version in TRUTH_VERSIONS, 'unsupported collector truth version')
    return {'truth': 'eval/truth/' + version + '/objects.json',
            'config': 'eval/truth/' + version + '-build.json',
            'trace': TRACE if version == TRUTH_VERSIONS[0] else 'eval/inputs/evidence/object-metrics-' + version + '.json',
            'input': INPUT}


def freeze_measurement(repo, input_raw, observation_raw, paths):
    """Preserve exact original bytes outside the harness's active input directory."""
    manifest={'schema_version':1,'original_paths':{'input':paths['input'],'observation':paths['trace']},
              'sha256':{'input.json':digest(input_raw),'observation.json':digest(observation_raw)}}
    payloads={'input.json':input_raw,'observation.json':observation_raw,'manifest.json':canonical(manifest)+b'\n'}
    folder=repo/'eval/evidence/object-metrics-history'/digest(canonical(manifest))
    require(not any(p.is_symlink() for p in (folder,*folder.parents)), 'symlinked measurement history')
    folder.mkdir(parents=True,exist_ok=True)
    for name,raw in payloads.items():
        require(len(raw)<=8*1024*1024,'measurement history exceeds bound')
        path=folder/name
        if path.exists():require(read(path)==raw,'immutable measurement history differs')
        else:
            with path.open('xb') as stream:stream.write(raw);stream.flush();os.fsync(stream.fileno())
    return str(folder.relative_to(repo))


def publish_measurement(repo, paths, observation, payload):
    # The harness scans every immediate JSON input and rejects duplicate metric
    # owners. Keep one active input; preserve overlapping prior cohorts as history.
    active=repo/INPUT
    if active.exists() or active.is_symlink():
        old_raw=read(active);old=document(old_raw)
        old_version=old['truth_sets']['K1']['version'];old_paths=release_paths(old_version)
        require(set(old['metrics'])=={'O1','O2'} and old['suite']=='objects', 'existing input has another metric owner')
        old_observation=read(repo/old_paths['trace'])
        for metric in old['metrics'].values():
            require(any(row['path']==old_paths['trace'] and row['sha256']==digest(old_observation)
                        for row in metric['evidence']), 'prior observation differs from its input')
        freeze_measurement(repo,old_raw,old_observation,old_paths)
    input_raw=canonical(payload)+b'\n';observation_raw=canonical(observation)+b'\n'
    archive=freeze_measurement(repo,input_raw,observation_raw,paths)
    atomic_json(repo/paths['trace'],observation)
    atomic_json(repo/INPUT,payload)
    return archive


def implementation_files(repo, truth_version=TRUTH_VERSIONS[0]):
    files=['src/source_index/graphics/masks.js',release_paths(truth_version)['config'],'eval/native-basis-vectors.json','Cargo.toml','Cargo.lock','examples/object_metrics.rs','scripts/eval/object-metrics.py','eval/object-metrics-contract.md','src/domain.rs','src/layout.rs','src/objects/mod.rs','src/source_index.rs','src/library.rs']
    for root in ('src','scripts/truth/latex'):
        if (repo/root).is_dir():
            files.extend(str(p.relative_to(repo)) for p in (repo/root).rglob('*') if p.is_file() and p.suffix in ('.rs','.py') and not p.name.startswith('test'))
    retained=repo/'eval/implementations'/truth_version
    if retained.is_dir():
        files.extend(str(p.relative_to(repo)) for p in retained.iterdir() if p.is_file() and (p.suffix=='.py' or p.name=='manifest.json'))
    return sorted(set(files))


def build_bridge(repo, truth_version=TRUTH_VERSIONS[0]):
    sources={p:digest(read(repo/p)) for p in implementation_files(repo,truth_version)}
    environment=dict(os.environ,CARGO_BUILD_JOBS='1',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0',CARGO_INCREMENTAL='0',CARGO_NET_OFFLINE='true',CARGO_TARGET_DIR=str(repo/'target'))
    environment.pop('CARGO_BUILD_TARGET',None)
    command=['cargo','build','--offline','--example','object_metrics','--message-format=json']
    started=time.monotonic()
    result=subprocess.run(command,cwd=repo,env=environment,capture_output=True,check=True,timeout=600)
    artifacts=[document(line) for line in result.stdout.splitlines() if line.startswith(b'{')]
    artifacts=[a for a in artifacts if a.get('reason')=='compiler-artifact' and a['target']['name']=='object_metrics' and a.get('executable')]
    expected=repo/'target/debug/examples/object_metrics'
    require(len(artifacts)==1 and artifacts[0]['executable']==str(expected) and artifacts[0]['target'].get('src_path')==str(repo/'examples/object_metrics.rs') and artifacts[0]['target'].get('kind')==['example'],'Cargo selected another executable')
    require(sources=={p:digest(read(repo/p)) for p in sources},'source changed during bridge build')
    receipt={'schema_version':1,'truth_version':truth_version,'command':command,'implementation':sources,'executable_sha256':digest(read(expected,128*1024*1024)),'cargo_stdout_sha256':digest(result.stdout),'cargo_stderr_sha256':digest(result.stderr),'selected_artifact':artifacts[0],'wall_seconds':time.monotonic()-started}
    cache=Path.home()/'.cache/lysilogy/object-metrics/builds'/digest(canonical(receipt))
    cache.mkdir(parents=True,exist_ok=True)
    for name,raw in [('cargo.jsonl',result.stdout),('stderr.log',result.stderr)]:
        path=cache/name
        if path.exists():require(read(path)==raw,'immutable build log differs')
        else:path.write_bytes(raw)
    receipt['raw_logs']=str(cache)
    atomic_json(repo/'target/object-metrics-build.json',receipt)
    print(json.dumps({'executable':str(expected),'sha256':receipt['executable_sha256'],'wall_seconds':receipt['wall_seconds']}))


def collect_layout(truth, config, before, truth_raw, paths, sources, executable_hash, expected, started):
    """New-format only: complete ordered records, one native/bridge paper at a time."""
    cache = Path.home() / '.cache/lysilogy'; corpus = Path.home() / 'Corpora/arxiv'; data = cache / 'arxiv-kb-data'
    verifier = truth_verifier(ROOT)
    bundle, fixed = verifier.verified_bundle(ROOT, BOUNDED_TRUTH_VERSION)
    api = verifier.bounded_api(ROOT, bundle, fixed)
    with api['release_process'].address_limit():
        return _collect_layout(truth, config, before, truth_raw, paths, sources, executable_hash,
                               expected, started, api, cache, corpus, data)


def _collect_layout(truth, config, before, truth_raw, paths, sources, executable_hash,
                    expected, started, api, cache, corpus, data):
    layout, transport = api['release_layout'], api['release_process']
    descriptors = layout.check_manifest(truth)
    root = ROOT / 'eval/truth' / BOUNDED_TRUTH_VERSION
    run_id = digest(truth_raw)[:16] + '-' + str(time.time_ns())
    external = cache / 'object-metrics-layout' / run_id
    decisions_root = ROOT / 'eval/evidence/object-metrics-layout' / run_id
    layout.direct(external).mkdir(parents=True)
    layout.direct(decisions_root / 'papers').mkdir(parents=True)
    deadline = time.monotonic() + 220 * len(descriptors) + 540
    registry_path = data / 'paper-identities.json'; registry_raw = read(registry_path)
    registry = document(registry_raw); decision_rows = []; prediction_rows = []
    decision_bytes = prediction_bytes = 0

    def timely():
        require(time.monotonic() < deadline, 'serial collector aggregate deadline exceeded')

    def check_source():
        timely()
        require(sources == {path: digest(read(ROOT / path)) for path in sources}
                and read(ROOT / paths['truth']) == truth_raw
                and layout.fingerprint(expected, 128 * 1024 * 1024)['sha256'] == executable_hash,
                'serial measurement source changed')

    def check_external(bindings):
        for name, expected_hash in bindings.items():
            timely()
            require(layout.fingerprint(Path(name), 512 * 1024 * 1024)['sha256'] == expected_hash,
                    'canonical input changed during serial measurement')

    with (external / 'ledger.jsonl').open('x') as ledger:
        def event(row):
            ledger.write((canonical(row) + b'\n').decode()); ledger.flush(); os.fsync(ledger.fileno())
        try:
            for descriptor in descriptors:
                timely(); paper = layout.paper(root, descriptor)
                ident = paper['paper_id']; ordinal = descriptor['ordinal']; eligibility = paper['metric_eligibility']
                decision = {'paper_id': ident, 'metric_eligibility': eligibility,
                            'truth': descriptor, 'status': 'not_in_visual_cohort', 'outcomes': []}
                if eligibility['O1'] or eligibility['O2']:
                    validate_registry(registry, [paper], corpus / 'pdf')
                    require(paper['index']['path'] == 'papers/' + ident + '/reading-index.json', 'wrong canonical index path')
                    tracked = {str(registry_path): digest(registry_raw)}
                    for kind, extension in (('pdf', 'pdf'), ('source', 'src')):
                        path = corpus / kind / (paper['arxiv_id'] + 'v' + str(paper['arxiv_version']) + '.' + extension)
                        h = layout.fingerprint(path, 512 * 1024 * 1024)['sha256']
                        require(h == paper[kind + '_sha256'], 'frozen corpus identity differs')
                        tracked[str(path)] = h
                    native_path = data / paper['index']['path']; raw = read(native_path)
                    require(digest(raw) == paper['index']['sha256'], 'native index changed')
                    tracked[str(native_path)] = digest(raw); index = document(raw)['index']; del raw
                    request = {'corpus_root': str(corpus / 'pdf'), 'data_root': str(data),
                               'papers': [{'paper_id': ident, 'relative_path': paper['arxiv_id'] + 'v' + str(paper['arxiv_version']) + '.pdf',
                                           'index_sha256': paper['index']['sha256']}]}
                    check_source()
                    raw, process_receipt = transport.run([str(expected)], canonical(request), external / ('bridge-' + str(ordinal)),
                        cwd=ROOT, seconds=min(40, deadline - time.monotonic()), stdout_cap=layout.PAPER_BYTES)
                    check_source()
                    response = layout.document(raw, layout.PAPER_BYTES)
                    require(response['schema_version'] == 1 and response['network_calls'] == response['model_calls'] == 0
                            and [row['paper_id'] for row in response['papers']] == [ident], 'serial bridge population differs')
                    row = response['papers'][0]
                    require(row['index_sha256'] == paper['index']['sha256'], 'bridge index differs')
                    artifact_raw = row['artifact_json'].encode()
                    require(digest(artifact_raw) == row['object_sha256'], 'object artifact hash differs')
                    artifact = layout.document(artifact_raw, layout.PAPER_BYTES)
                    derivation = validate_derivation(row, artifact, index)
                    derivation['graphics'] = validate_graphics(row, artifact, paper, index, cache)
                    for field in ('trace_hashes', 'mask_hashes', 'vector_hashes'):
                        tracked.update(derivation['graphics'][field])
                    metric = evaluate_paper(paper, artifact, index, BOUNDED_TRUTH_VERSION)
                    prediction_raw = canonical(response) + b'\n'
                    prediction_bytes += len(prediction_raw)
                    require(prediction_bytes <= layout.TOTAL_BYTES, 'cumulative prediction bytes exceeded')
                    prediction_path = external / (ident + '.json')
                    layout.immutable(prediction_path, prediction_raw, layout.PAPER_BYTES)
                    prediction = {'ordinal': ordinal, 'paper_id': ident, 'path': str(prediction_path),
                                  'bytes': len(prediction_raw), 'sha256': digest(prediction_raw)}
                    prediction_rows.append(prediction)
                    decision.update(status='measured', metrics=metric, outcomes=metric['outcomes'],
                        prediction=prediction, external_input_hashes=tracked,
                        object_hashes={'rust_serialized_sha256': row['object_sha256'],
                                       'canonical_sha256': digest(canonical(artifact)), 'derivation': derivation},
                        process_receipt=process_receipt)
                    check_external(tracked)
                    del index, response, row, artifact, artifact_raw, raw, prediction_raw, metric
                raw = layout.canonical(decision); decision_bytes += len(raw)
                require(decision_bytes <= layout.TOTAL_BYTES, 'cumulative decision bytes exceeded')
                saved = layout.decision_descriptor(ident, ordinal, raw)
                layout.immutable(decisions_root / saved['path'], raw, layout.PAPER_BYTES)
                decision_rows.append(saved); event({'status': decision['status'], 'decision': saved})
                del decision, paper, raw

            def read_decisions():
                for descriptor in decision_rows:
                    raw = layout.read(decisions_root / descriptor['path'], layout.PAPER_BYTES)
                    require(len(raw) == descriptor['bytes'] and digest(raw) == descriptor['sha256'], 'decision bytes changed')
                    row = layout.document(raw, layout.PAPER_BYTES)
                    if row['status'] == 'measured':
                        check_external(row['external_input_hashes'])
                        p = row['prediction']
                        require(layout.fingerprint(Path(p['path']), layout.PAPER_BYTES) == {k: p[k] for k in ('bytes', 'sha256')},
                                'prediction changed after serial measurement')
                    yield row

            totals, values, matched = aggregate_layout_metrics(truth, read_decisions())
            check_source()
            _, _, after = validate_truth(ROOT, cache, corpus, data, truth_raw, True)
            timely()
            check_source()
            for row in descriptors:
                layout.paper(root, row)
            for _ in read_decisions():
                pass
            prediction_manifest = {'schema_version': 1, 'truth_sha256': digest(truth_raw),
                                   'papers': prediction_rows, 'total_bytes': prediction_bytes}
            layout.immutable(external / 'manifest.json', layout.canonical(prediction_manifest), layout.MANIFEST_BYTES)
            observation = {'schema_version': 2, 'collector': VERSION, 'truth_version': BOUNDED_TRUTH_VERSION,
                'truth_sha256': digest(truth_raw), 'coverage': truth['coverage'],
                'truth_verification': {'before': before, 'after': after}, 'summary': totals,
                'O1': 2 * totals['tp'] / (2 * totals['tp'] + totals['fp'] + totals['fn']),
                'O2': statistics.median(values), 'matched_only_median': statistics.median(matched) if matched else None,
                'papers': decision_rows, 'decision_root': str(decisions_root.relative_to(ROOT)),
                'predictions': {'path': str(external / 'manifest.json'),
                                **layout.fingerprint(external / 'manifest.json', layout.MANIFEST_BYTES)},
                'executable_sha256': executable_hash, 'build_receipt_sha256': digest(read(ROOT / 'target/object-metrics-build.json')),
                'network_calls': 0, 'model_calls': 0, 'cost_usd': 0, 'wall_seconds': time.monotonic() - started}
            child_evidence = []
            for row in descriptors:
                child_evidence.append({'path': str((root / row['path']).relative_to(ROOT)),
                                       'version': BOUNDED_TRUTH_VERSION, 'sha256': row['sha256']})
            for row in decision_rows:
                child_evidence.append({'path': str((decisions_root / row['path']).relative_to(ROOT)),
                                       'version': VERSION, 'sha256': row['sha256']})
            observation_ref = {'path': paths['trace'], 'version': VERSION, 'sha256': digest(canonical(observation) + b'\n')}
            def reference(path, version):
                return {'path': path, 'version': version, 'sha256': digest(read(ROOT / path))}
            payload = {'schema_version': 1, 'suite': 'objects', 'collector': VERSION,
                'implementation': [reference(path, VERSION) for path in sources],
                'truth_sets': {'K1': reference(paths['truth'], BOUNDED_TRUTH_VERSION)},
                'metrics': {'O1': {'sample': {'method': 'f1', 'true_positive': totals['tp'], 'false_positive': totals['fp'],
                                            'false_negative': totals['fn']}, 'cases': totals['truth_objects'],
                                   'evidence': [observation_ref, *child_evidence]},
                            'O2': {'sample': {'method': 'median', 'values': values}, 'cases': len(values),
                                   'evidence': [observation_ref, *child_evidence]}},
                'cost_usd': 0, 'wall_seconds': observation['wall_seconds']}
            timely()
            # Preflight both active files before the historical publication routine mutates either.
            layout.canonical(observation, layout.PAPER_BYTES)
            layout.canonical(payload, layout.PAPER_BYTES)
            publish_measurement(ROOT, paths, observation, payload)
            event({'status': 'complete', 'summary': totals})
            print(json.dumps({k: observation[k] for k in ('summary', 'O1', 'O2', 'matched_only_median', 'wall_seconds')}, sort_keys=True))
        except BaseException as error:
            event({'status': 'failed', 'error': str(error)[:4096], 'retained_decisions': len(decision_rows)})
            raise


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--truth-version', choices=TRUTH_VERSIONS, default=TRUTH_VERSIONS[0], help='Explicit immutable cohort; preserve prior observations/input before selecting the active cohort')
    parser.add_argument('--build',action='store_true',help='Build the exact worktree bridge and freeze its Cargo receipt')
    parser.add_argument('--executable',type=Path,help='Cargo-built object_metrics executable in this worktree target/debug/examples')
    args=parser.parse_args();started=time.monotonic()
    paths=release_paths(args.truth_version)
    if args.build:
        build_bridge(ROOT,args.truth_version);return
    require(args.executable is not None, '--executable is required for measurement')
    cache=Path.home()/'.cache/lysilogy';corpus=Path.home()/'Corpora/arxiv';data=cache/'arxiv-kb-data'
    expected=ROOT/'target/debug/examples/object_metrics'
    # A mutable receipt alone never establishes a current executable. Invoke
    # Cargo on every measurement and verify its exact selected artifact.
    build_bridge(ROOT,args.truth_version)
    require(args.executable.absolute()==expected, 'executable must be the current worktree example')
    executable_raw=read(expected,128*1024*1024);executable_hash=digest(executable_raw)
    # Cargo JSON output binds the executable to its build, target, source and
    # dependency graph; inherited CARGO_TARGET_DIR/CARGO_BUILD_TARGET cannot select it.
    build= document(read(ROOT/'target/object-metrics-build.json'))
    sources={path:digest(read(ROOT/path)) for path in implementation_files(ROOT,args.truth_version)}
    require(build['truth_version']==args.truth_version and build['executable_sha256']==executable_hash and build['implementation']==sources, 'executable build receipt is stale')
    truth_raw=read(ROOT/paths['truth']);truth,config,truth_verification_before=validate_truth(ROOT,cache,corpus,data,truth_raw,True)
    require(truth['version']==args.truth_version and config['version']==args.truth_version, 'selected collector truth version differs')
    if args.truth_version == BOUNDED_TRUTH_VERSION:
        collect_layout(truth, config, truth_verification_before, truth_raw, paths, sources,
                       executable_hash, expected, started)
        return
    papers=[p for p in truth['papers'] if p['metric_eligibility']['O1']]
    require([p['paper_id'] for p in papers]==truth['coverage']['cohort_papers']['O1'] and papers,'incomplete frozen detection cohort')
    registry_path=data/'paper-identities.json';registry_raw=read(registry_path)
    validate_registry(document(registry_raw),papers,corpus/'pdf')
    tracked={str(registry_path):digest(registry_raw)};request=[];indexes={}
    for paper in papers:
        ident=paper['paper_id'];require(re.fullmatch('[0-9a-f]{16}',ident),'invalid mapped paper ID')
        require(paper['index']['path']=='papers/'+ident+'/reading-index.json','wrong canonical index path')
        for kind,ext in [('pdf','pdf'),('source','src')]:
            path=corpus/kind/(paper['arxiv_id']+'v'+str(paper['arxiv_version'])+'.'+ext)
            actual=digest(read(path,512*1024*1024));require(actual==paper[kind+'_sha256'],'frozen artifact changed');tracked[str(path)]=actual
        path=data/paper['index']['path'];raw=read(path);actual=digest(raw)
        require(actual==paper['index']['sha256'],'index generation changed');tracked[str(path)]=actual
        indexes[ident]=document(raw)['index']
        request.append({'paper_id':ident,'relative_path':paper['arxiv_id']+'v'+str(paper['arxiv_version'])+'.pdf','index_sha256':actual})
    result=subprocess.run([str(expected)],input=canonical({'corpus_root':str(corpus/'pdf'),'data_root':str(data),'papers':request}),cwd=ROOT,capture_output=True,check=True,timeout=max(60,40*len(papers)))
    require(len(result.stdout)<=MAX_JSON,'bridge response too large')
    response=document(result.stdout);rows=response['papers']
    require(response['schema_version']==1 and response['model_calls']==response['network_calls']==0,'invalid bridge receipt')
    require([r['paper_id'] for r in rows]==[p['paper_id'] for p in papers],'bridge omitted or duplicated a paper')
    metrics=[];object_hashes={}
    for paper,row in zip(papers,rows):
        require(row['index_sha256']==paper['index']['sha256'],'bridge index differs')
        # Rust serde serialization order is retained externally for the actual
        # object digest; a separately canonicalized digest binds the JSON projection.
        artifact_raw=row['artifact_json'].encode();require(digest(artifact_raw)==row['object_sha256'],'object artifact hash differs')
        artifact=document(artifact_raw)
        derivation=validate_derivation(row,artifact,indexes[paper['paper_id']])
        derivation['graphics']=validate_graphics(row,artifact,paper,indexes[paper['paper_id']],cache)
        tracked.update(derivation['graphics']['trace_hashes'])
        tracked.update(derivation['graphics']['mask_hashes'])
        tracked.update(derivation['graphics']['vector_hashes'])
        object_hashes[paper['paper_id']]={'rust_serialized_sha256':row['object_sha256'],'canonical_sha256':digest(canonical(artifact)),'derivation':derivation}
        metrics.append(evaluate_paper(paper,artifact,indexes[paper['paper_id']],args.truth_version))
    require(sources=={p:digest(read(ROOT/p)) for p in sources} and read(ROOT/paths['truth'])==truth_raw and digest(read(expected,128*1024*1024))==executable_hash,'measurement source or truth changed')
    for path,expected_hash in tracked.items():require(digest(read(Path(path),512*1024*1024))==expected_hash,'canonical input changed during measurement')
    _,_,truth_verification_after=validate_truth(ROOT,cache,corpus,data,truth_raw,True)
    require(sources=={p:digest(read(ROOT/p)) for p in sources},'measurement source changed during final truth replay')
    totals,values,matched=aggregate_metrics(truth,metrics)
    observation={'schema_version':1,'collector':VERSION,'truth_sha256':digest(truth_raw),'truth_version':truth['version'],'coverage':truth['coverage'],'truth_verification':{'before':truth_verification_before,'after':truth_verification_after},'summary':totals,'O1':2*totals['tp']/(2*totals['tp']+totals['fp']+totals['fn']),'O2':statistics.median(values),'matched_only_median':statistics.median(matched) if matched else None,'papers':metrics,'object_hashes':object_hashes,'executable_sha256':executable_hash,'build_receipt_sha256':digest(read(ROOT/'target/object-metrics-build.json')),'external_input_hashes':tracked,'network_calls':0,'model_calls':0,'cost_usd':0,'wall_seconds':time.monotonic()-started}
    # Freeze full predictions externally; commit only derived diagnostic records.
    run_root=cache/'object-metrics'/digest(result.stdout)
    atomic_json(run_root/'predictions.json',response)
    observation['predictions_sha256']=digest(canonical(response)+b'\n')
    evidence=lambda p,v:{'path':p,'version':v,'sha256':digest(read(ROOT/p))}
    observation_evidence={'path':paths['trace'],'version':VERSION,'sha256':digest(canonical(observation)+b'\n')}
    payload={'schema_version':1,'suite':'objects','collector':VERSION,'implementation':[evidence(p,VERSION) for p in sources],'truth_sets':{'K1':evidence(paths['truth'],truth['version'])},'metrics':{'O1':{'sample':{'method':'f1','true_positive':totals['tp'],'false_positive':totals['fp'],'false_negative':totals['fn']},'cases':totals['truth_objects'],'evidence':[observation_evidence]},'O2':{'sample':{'method':'median','values':values},'cases':len(values),'evidence':[observation_evidence]}},'cost_usd':0,'wall_seconds':observation['wall_seconds']}
    publish_measurement(ROOT,paths,observation,payload)
    print(json.dumps({k:observation[k] for k in ('summary','O1','O2','matched_only_median','wall_seconds')},sort_keys=True))


if __name__=='__main__':main()
