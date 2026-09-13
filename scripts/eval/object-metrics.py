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
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
TRUTH = 'eval/truth/k1-limited-v1/objects.json'
CONFIG = 'eval/truth/k1-limited-v1-build.json'
TRACE = 'eval/inputs/evidence/object-metrics.json'
INPUT = 'eval/inputs/objects/figure-table.json'
KINDS = ('figure', 'table')
VERSION = 'figure-table-metrics-v1'
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


def evaluate_paper(paper, artifact, index):
    require(paper['metric_eligibility'].get('O1') is True, 'paper is outside complete O1 cohort')
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


def validate_registry(registry, papers, corpus_pdf_root):
    require(registry['schema_version']==1 and registry['library_root']==str(corpus_pdf_root), 'registry root or schema differs')
    records=registry['records'];active=Counter(r['relative_path'] for r in records.values() if r['active'])
    for paper in papers:
        row=records.get(paper['paper_id']);name=paper['arxiv_id']+'v'+str(paper['arxiv_version'])+'.pdf'
        require(row is not None and row['active'] is True and row['relative_path']==name and row['content_hash']==paper['pdf_sha256'], 'canonical registry identity differs')
        require(active[name]==1 and paper['paper_id'] not in {ident for ids in registry.get('unresolved',{}).values() for ident in ids}, 'canonical registry identity is ambiguous')


def validate_truth(repo, cache, corpus, data, truth_raw):
    """Replay only independent truth construction; never expose predictions to labels."""
    sys.path.insert(0,str(repo/'scripts/truth/latex'))
    import release
    import manual
    truth=document(truth_raw);config_raw=read(repo/CONFIG);config=document(config_raw)
    require(truth['schema_version']==1 and truth['truth_set']=='K1' and truth['origin']=='arxiv-latex' and truth['version']==config['version'], 'unsupported frozen K1 identity')
    original=truth['provenance']
    require(digest(config_raw)==original['build_config_sha256'], 'K1 config hash differs')
    # The historical detector is embedded in the frozen native caches. Current
    # wrappers are measured separately, while changed detectors require new caches.
    require(digest(read(repo/'src/source_index/figures.rs'))==original['implementation']['src/source_index/figures.rs'], 'native cache detector source differs; new-generation measurement required')
    for path,expected in original['implementation'].items():
        if path.startswith('scripts/truth/latex/'):
            require(digest(read(repo/path))==expected,'truth implementation changed')
    evidence=release.verify_evidence(cache,config)
    require(evidence==config['evidence_sha256']==original['evidence_sha256'], 'frozen K1 evidence drift')
    inputs=document(read(cache/config['inputs']))
    assemblies=[manual.assemble(cache,corpus,data,p['candidate'],p.get('region_bundle'),p.get('panel_bundle'),config['inputs'],config['indexes'],p.get('object_bundle'),p.get('bibliography_bundle')) for p in config['papers']]
    current={name:read(repo/'scripts/truth/latex'/name) for name in ('archive.py','tex.py','parser.py','align.py','builder.py')}
    history=release.validate_automatic_reports(cache,config,inputs,evidence,current)
    rebuilt,_=release.build_release(assemblies,config,inputs,history);rebuilt['provenance']=original
    require(canonical(rebuilt)+b'\n'==truth_raw, 'K1 labels or complete cohort differ from independent evidence')
    return truth,config


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


def implementation_files(repo):
    files=['Cargo.toml','Cargo.lock','examples/object_metrics.rs','scripts/eval/object-metrics.py','eval/object-metrics-contract.md','src/domain.rs','src/layout.rs','src/objects/mod.rs','src/source_index.rs','src/library.rs']
    for root in ('src','scripts/truth/latex'):
        if (repo/root).is_dir():
            files.extend(str(p.relative_to(repo)) for p in (repo/root).rglob('*') if p.is_file() and p.suffix in ('.rs','.py') and not p.name.startswith('test'))
    return sorted(set(files))


def build_bridge(repo):
    sources={p:digest(read(repo/p)) for p in implementation_files(repo)}
    environment=dict(os.environ,CARGO_BUILD_JOBS='1',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0',CARGO_INCREMENTAL='0',CARGO_NET_OFFLINE='true',CARGO_TARGET_DIR=str(repo/'target'))
    environment.pop('CARGO_BUILD_TARGET',None)
    command=['cargo','build','--offline','--example','object_metrics','--message-format=json']
    started=time.monotonic()
    result=subprocess.run(command,cwd=repo,env=environment,capture_output=True,check=True,timeout=600)
    artifacts=[document(line) for line in result.stdout.splitlines() if line.startswith(b'{')]
    artifacts=[a for a in artifacts if a.get('reason')=='compiler-artifact' and a['target']['name']=='object_metrics' and a.get('executable')]
    expected=repo/'target/debug/examples/object_metrics'
    require(len(artifacts)==1 and artifacts[0]['executable']==str(expected),'Cargo selected another executable')
    require(sources=={p:digest(read(repo/p)) for p in sources},'source changed during bridge build')
    receipt={'schema_version':1,'command':command,'implementation':sources,'executable_sha256':digest(read(expected,128*1024*1024)),'cargo_stdout_sha256':digest(result.stdout),'cargo_stderr_sha256':digest(result.stderr),'selected_artifact':artifacts[0],'wall_seconds':time.monotonic()-started}
    cache=Path.home()/'.cache/lysilogy/object-metrics/builds'/digest(canonical(receipt))
    cache.mkdir(parents=True,exist_ok=True)
    for name,raw in [('cargo.jsonl',result.stdout),('stderr.log',result.stderr)]:
        path=cache/name
        if path.exists():require(read(path)==raw,'immutable build log differs')
        else:path.write_bytes(raw)
    receipt['raw_logs']=str(cache)
    atomic_json(repo/'target/object-metrics-build.json',receipt)
    print(json.dumps({'executable':str(expected),'sha256':receipt['executable_sha256'],'wall_seconds':receipt['wall_seconds']}))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build',action='store_true',help='Build the exact worktree bridge and freeze its Cargo receipt')
    parser.add_argument('--executable',type=Path,help='Cargo-built object_metrics executable in this worktree target/debug/examples')
    args=parser.parse_args();started=time.monotonic()
    if args.build:
        build_bridge(ROOT);return
    require(args.executable is not None, '--executable is required for measurement')
    cache=Path.home()/'.cache/lysilogy';corpus=Path.home()/'Corpora/arxiv';data=cache/'arxiv-kb-data'
    expected=ROOT/'target/debug/examples/object_metrics'
    # A mutable receipt alone never establishes a current executable. Invoke
    # Cargo on every measurement and verify its exact selected artifact.
    build_bridge(ROOT)
    require(args.executable.absolute()==expected, 'executable must be the current worktree example')
    executable_raw=read(expected,128*1024*1024);executable_hash=digest(executable_raw)
    # Cargo JSON output binds the executable to its build, target, source and
    # dependency graph; inherited CARGO_TARGET_DIR/CARGO_BUILD_TARGET cannot select it.
    build= document(read(ROOT/'target/object-metrics-build.json'))
    sources={path:digest(read(ROOT/path)) for path in implementation_files(ROOT)}
    require(build['executable_sha256']==executable_hash and build['implementation']==sources, 'executable build receipt is stale')
    truth_raw=read(ROOT/TRUTH);truth,config=validate_truth(ROOT,cache,corpus,data,truth_raw)
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
    result=subprocess.run([str(expected)],input=canonical({'corpus_root':str(corpus/'pdf'),'data_root':str(data),'papers':request}),cwd=ROOT,capture_output=True,check=True,timeout=60)
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
        object_hashes[paper['paper_id']]={'rust_serialized_sha256':row['object_sha256'],'canonical_sha256':digest(canonical(artifact))}
        metrics.append(evaluate_paper(paper,artifact,indexes[paper['paper_id']]))
    require(sources=={p:digest(read(ROOT/p)) for p in sources} and read(ROOT/TRUTH)==truth_raw and digest(read(expected,128*1024*1024))==executable_hash,'measurement source or truth changed')
    for path,expected_hash in tracked.items():require(digest(read(Path(path),512*1024*1024))==expected_hash,'canonical input changed during measurement')
    sys.path.insert(0,str(ROOT/'scripts/truth/latex'));import release
    require(release.verify_evidence(cache,config)==config['evidence_sha256'],'external truth evidence changed')
    totals={k:sum(p[k] for p in metrics) for k in ('tp','fp','fn','truth_objects','predictions','unknown_truth_regions')}
    values=[v for p in metrics for v in p['region_values']];matched=[v for p in metrics for v in p['matched_region_values']]
    require(totals['truth_objects']>0 and values,'empty objective denominator')
    observation={'schema_version':1,'collector':VERSION,'truth_sha256':digest(truth_raw),'truth_version':truth['version'],'coverage':truth['coverage'],'summary':totals,'O1':2*totals['tp']/(2*totals['tp']+totals['fp']+totals['fn']),'O2':statistics.median(values),'matched_only_median':statistics.median(matched) if matched else None,'papers':metrics,'object_hashes':object_hashes,'executable_sha256':executable_hash,'build_receipt_sha256':digest(read(ROOT/'target/object-metrics-build.json')),'external_input_hashes':tracked,'network_calls':0,'model_calls':0,'cost_usd':0,'wall_seconds':time.monotonic()-started}
    # Freeze full predictions externally; commit only derived diagnostic records.
    run_root=cache/'object-metrics'/digest(result.stdout)
    atomic_json(run_root/'predictions.json',response)
    observation['predictions_sha256']=digest(canonical(response)+b'\n')
    atomic_json(ROOT/TRACE,observation)
    evidence=lambda p,v:{'path':p,'version':v,'sha256':digest(read(ROOT/p))}
    payload={'schema_version':1,'suite':'objects','collector':VERSION,'implementation':[evidence(p,VERSION) for p in sources],'truth_sets':{'K1':evidence(TRUTH,truth['version'])},'metrics':{'O1':{'sample':{'method':'f1','true_positive':totals['tp'],'false_positive':totals['fp'],'false_negative':totals['fn']},'cases':totals['truth_objects'],'evidence':[evidence(TRACE,VERSION)]},'O2':{'sample':{'method':'median','values':values},'cases':len(values),'evidence':[evidence(TRACE,VERSION)]}},'cost_usd':0,'wall_seconds':observation['wall_seconds']}
    atomic_json(ROOT/INPUT,payload)
    print(json.dumps({k:observation[k] for k in ('summary','O1','O2','matched_only_median','wall_seconds')},sort_keys=True))


if __name__=='__main__':main()
