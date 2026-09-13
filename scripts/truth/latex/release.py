#!/usr/bin/env python3
"""Publish a versioned, explicitly limited K1 release from reviewed external bundles."""
import argparse
from collections import Counter
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import time

from annotations import canonical, document, require
from archive import sha256
from builder import atomic_json, fingerprint_sources, safe_file
from manual import assemble, bounded

REQUIRED_KINDS = {'figure', 'table', 'equation', 'statement', 'proof', 'algorithm', 'bib_entry'}
METRICS = [f'O{number}' for number in range(1, 12)]


def compact_object(row):
    keys = ('id', 'kind', 'labels', 'printed_label', 'printed_heading', 'spans', 'region', 'source_members',
            'text_sha256', 'source_parent_object', 'source_child_objects', 'proof_targets', 'proof_linkage',
            'reading_index_fidelity', 'semantic_quote_truth')
    return {key: row[key] for key in keys if key in row}


def compact_entry(row):
    fields = {}
    for name, field in row['field_provenance'].items():
        role = field['root_source_role']
        fields[name] = {'native_span': field['native_span'], 'source_member': field['source_member'],
                        'source_role': field['source_role'], 'source_command': {key: role[key] for key in ('command', 'source_command_start', 'source_command_end')}}
    return {key: row[key] for key in ('id', 'printed_key', 'spans', 'field_labels', 'source_members')} | {'field_provenance': fields}


def build_release(assemblies, config, inputs, history):
    """Only caller-verified complete overlays can enter a predeclared cohort."""
    require(re.fullmatch(r'k1-limited-v[1-9][0-9]*', config['version']), 'limited release version is invalid')
    require(config['target_papers'] == 500 and config['coverage_followup'] == 'https://github.com/tjmisko/Lysilogy/issues/97', 'limited release must retain its original coverage target and follow-up')
    require(config['selection_bias'].strip() and config['publication_deviation'].strip(), 'limited release must state selection bias and publication deviation')
    require(len(assemblies) == len(config['papers']) and len({row['paper_id'] for row in assemblies}) == len(assemblies), 'limited release has missing or duplicated paper identities')
    frozen = {row['arxiv_id']: row for row in inputs['papers']}
    require(len(frozen) == len(inputs['papers']), 'frozen eval inputs have duplicate papers')
    papers, totals, strata = [], Counter(), Counter()
    cohort_papers = {key: [] for key in METRICS}
    for candidate, specification in zip(assemblies, config['papers']):
        require(candidate['arxiv_id'] == specification['arxiv_id'] and candidate['paper_id'] == specification['paper_id'], 'release paper order/identity differs from frozen cohort')
        require(candidate['arxiv_id'] in frozen, 'release paper is outside frozen eval inputs')
        source = frozen[candidate['arxiv_id']]
        require(re.fullmatch(r'[0-9a-f]{16}', candidate['paper_id']) and type(source['version']) is int and source['version'] > 0, 'release lacks a valid mapped ID and pinned arXiv version')
        require(candidate['stratum'] == source['stratum'], 'release stratum differs from frozen eval selection')
        require(all(candidate[key] == source['version'] for key in ('version','arxiv_version') if key in candidate), 'release candidate version differs from frozen source version')
        require(candidate['pdf_sha256'] == source['pdf']['sha256'] and candidate['source_sha256'] == source['source']['sha256'], 'release artifact differs from frozen corpus cohort')
        row = {key: candidate[key] for key in ('arxiv_id', 'paper_id', 'pdf_sha256', 'source_sha256', 'index', 'stratum', 'source_inventory_sha256')}
        row.update(arxiv_version=source['version'], arxiv_url='https://arxiv.org/abs/' + candidate['arxiv_id'] + 'v' + str(source['version']),
                   alignment={'quality': 1.0, 'method': 'complete source/PDF inventory independently annotated and reconciled per eligible metric'},
                   objects=[], entries=[], mentions=[], references=[], associated_content=[],
                   metric_eligibility={key: False for key in METRICS}, reviewed_absent_kinds=[],
                   provenance={'automatic_candidate': specification['candidate'], 'automatic_candidate_sha256': candidate['manual_assembly']['candidate_sha256'],
                               'assembly_sha256': sha256(canonical(candidate)), 'automatic_accepted': candidate['accepted'],
                               'automatic_metric_eligibility': candidate['metric_eligibility'], 'overlay_evidence': {}})
        if 'manual_figure_table_overlay' in candidate:
            overlay = candidate['manual_figure_table_overlay']
            require(overlay['metric_eligibility'] == {'O1': True, 'O2': True}, 'figure/table overlay is incomplete')
            row['objects'].extend(compact_object(item) for item in overlay['objects'])
            row['metric_eligibility'].update(overlay['metric_eligibility'])
            row['provenance']['overlay_evidence']['figure_table'] = overlay['evidence_hashes']
        if 'manual_object_overlay' in candidate:
            overlay = candidate['manual_object_overlay']
            require(all(overlay['metric_eligibility'].get(key) is True for key in ('O3','O4','O5','O6','O7')), 'manual object overlay is incomplete')
            row['objects'].extend(compact_object(item) for item in overlay['objects'])
            row['references'] = overlay['references']; row['associated_content'] = overlay['associated_content']
            row['non_object_references'] = overlay['non_object_references']
            row['metric_eligibility'].update(overlay['metric_eligibility'])
            row['reviewed_absent_kinds'] = overlay['reviewed_absent_kinds']
            if set(row['reviewed_absent_kinds']) == {'figure','table'}:
                row['metric_eligibility'].update(O1=True, O2=True)
            row['provenance']['overlay_evidence']['objects'] = overlay['evidence_hashes']
        if 'manual_bibliography_overlay' in candidate:
            overlay = candidate['manual_bibliography_overlay']
            require(overlay['metric_eligibility'] == {'O8': True, 'O9': True, 'O10': True}, 'manual bibliography overlay is incomplete')
            row['entries'] = [compact_entry(item) for item in overlay['entries']]
            row['mentions'] = overlay['mentions']; row['citation_groups'] = overlay['citation_groups']
            row['metric_eligibility'].update(overlay['metric_eligibility'])
            row['provenance']['overlay_evidence']['bibliography'] = overlay['evidence_hashes']
        if 'independent_panel' in candidate:
            panel = candidate['independent_panel']
            require(row['metric_eligibility']['O1'] and len(panel['identities']) == len(panel['panelists']) == 3 and len({item['agent_identity'] for item in panel['identities']}) == 3, 'O11 requires a complete figure/table cohort and three distinct panelists')
            row['panel'] = {key: panel[key] for key in ('identities','valid_ids','panelists','packet_sha256','prompt_sha256','review_sha256','scoring_policy_sha256','selection_policy')}
            row['metric_eligibility']['O11'] = True
        require(any(row['metric_eligibility'].values()), 'release paper has no independently eligible metric')
        object_ids = {item['id'] for item in row['objects']}
        require(len(object_ids) == len(row['objects']), 'release object identities collide')
        counts = Counter(item['kind'] for item in row['objects']); counts['bib_entry'] = len(row['entries'])
        totals.update(counts)
        for metric, eligible in row['metric_eligibility'].items():
            if eligible: cohort_papers[metric].append(row['paper_id'])
        strata[str(candidate['stratum'])] += 1
        row['metric_alignment_quality'] = {key: 1.0 if eligible else None for key,eligible in row['metric_eligibility'].items()}
        row['counts'] = dict(counts)
        row['bibliography_eligible'] = row['metric_eligibility']['O8'] and row['metric_eligibility']['O10']
        papers.append(row)
    require(all(totals[kind] > 0 for kind in REQUIRED_KINDS), 'limited release lacks a positive example of every E1 kind')
    require(all(cohort_papers.values()), 'limited release lacks an independently eligible metric cohort')
    object_refs = sum(1 for paper in papers for ref in paper['references'] if next(item['kind'] for item in paper['objects'] if item['id'] == ref['target']) in ('equation','statement'))
    denominators = {'O1': {'truth_objects': totals['figure'] + totals['table'], 'papers': len(cohort_papers['O1']), 'negative_papers': sum(set(paper['reviewed_absent_kinds']) == {'figure','table'} for paper in papers)},
                    'O2': {'all_annotated_truth_objects': totals['figure'] + totals['table']},
                    'O3': {'equations': totals['equation']}, 'O4': {'object_reference_pairs': object_refs},
                    'O5': {'statements': totals['statement']}, 'O6': {'proofs': totals['proof']}, 'O7': {'algorithms': totals['algorithm']},
                    'O8': {'entries': totals['bib_entry']}, 'O9': {'known_fields': sum(len(item['field_labels']) for paper in papers for item in paper['entries'])},
                    'O10': {'citation_target_pairs': sum(len(paper['mentions']) for paper in papers)}, 'O11': {'papers': len(cohort_papers['O11']), 'panelist_top3_opportunities': 9 * len(cohort_papers['O11'])}}
    coverage = {'release_papers': len(papers), 'target_papers': 500, 'target_met': len(papers) >= 500,
                'frozen_eval_papers': len(frozen), 'cohort_papers': cohort_papers, 'denominators': denominators,
                'excluded_eval_papers_by_metric': {metric: len(frozen) - len(papers) for metric,papers in cohort_papers.items()},
                'frozen_eval_strata': dict(Counter(str(paper['stratum']) for paper in frozen.values())),
                'positive_object_counts': dict(totals), 'strata': dict(strata), 'selection_bias': config['selection_bias'],
                'publication_deviation': config['publication_deviation'], 'followup': config['coverage_followup'],
                'automatic_builds': history, 'system_acceptance_claimed': False}
    release = {'schema_version': 1, 'truth_set': 'K1', 'version': config['version'], 'origin': 'arxiv-latex',
               'alignment_threshold': .95, 'scope': 'limited independently reviewed release; coverage expansion remains open',
               'build_date': config['build_date'], 'coverage': coverage, 'papers': papers}
    bibliography = {key: release[key] for key in ('schema_version','truth_set','origin','alignment_threshold','scope','build_date','coverage')}
    bibliography['version'] = config['version'] + '-bibliography'
    bibliography['papers'] = [{key: paper[key] for key in ('arxiv_id','arxiv_version','arxiv_url','paper_id','pdf_sha256','source_sha256','index','alignment','entries','mentions','provenance')} for paper in papers if paper['bibliography_eligible']]
    return release, bibliography


def evidence_paths(config):
    required = {config['inputs'], config['indexes']}
    required.update(item[key] for item in config['automatic_builds'] for key in ('path','paper_summaries'))
    for paper in config['papers']:
        required.add(paper['candidate'])
        for field, names in [('region_bundle', ('regions-root-v1.json','review-independent-v1.json','source-associations-root-v1.json','source-associations-independent-review-v1.json')),
                             ('object_bundle', ('packet.json','root-v1.json','independent-v1.json','independent-v1-receipt.json','reconciliation-independent-v1.json')),
                             ('bibliography_bundle', ('packet.json','bibliography-packet.json','bibliography-root-v1.json','bibliography-independent-v1.json','bibliography-independent-v1-receipt.json','bibliography-reconciliation-independent-v1.json')),
                             ('panel_bundle', ('packet.json','prompt.txt','vote-evaluator-1.json','vote-evaluator-2.json','vote-evaluator-3.json','panel-root-review-v1.json','scoring-policy-v1.json'))]:
            if paper.get(field): required.update(paper[field] + '/' + name for name in names)
    require(required == set(config['evidence_sha256']), 'release must pin every consumed external evidence document')
    return required


def verify_evidence(cache, config):
    return {path: sha256(bounded(cache, path)) for path in evidence_paths(config)}


def checked_output_ancestors(path):
    """Reject redirected output paths before directory creation or file reads."""
    path = path.absolute()
    require('..' not in path.parts, 'release output cannot traverse parent directories')
    for ancestor in reversed((path, *path.parents)):
        require(not ancestor.is_symlink(), 'release output or staging ancestor is a symlink')
        if ancestor != path:
            require(not ancestor.exists() or ancestor.is_dir(), 'release output ancestor is not a directory')
    return path


def write_immutable(output, payloads):
    output = checked_output_ancestors(output)
    if output.exists():
        require(output.is_dir() and not output.is_symlink() and {path.name for path in output.iterdir()} == set(payloads), 'existing release has unexpected files')
        require(all((output / name).is_file() and not (output / name).is_symlink() and (output / name).read_bytes() == raw for name,raw in payloads.items()), 'published release bytes are immutable; choose a new version')
        return
    stage = output.parent / ('.' + output.name + '-staging-' + sha256(canonical({name:sha256(raw) for name,raw in payloads.items()}))[:16])
    checked_output_ancestors(stage)
    stage.mkdir(parents=True, exist_ok=True)
    require(not stage.is_symlink() and set(path.name for path in stage.iterdir()).issubset(payloads), 'release staging directory has unexpected files')
    for name,raw in payloads.items():
        path=stage/name
        if path.exists(): require(not path.is_symlink() and path.read_bytes()==raw, 'interrupted release staging bytes differ')
        else:
            with path.open('xb') as handle: handle.write(raw);handle.flush();os.fsync(handle.fileno())
    os.rename(stage,output)
    descriptor=os.open(output.parent,os.O_RDONLY)
    try:os.fsync(descriptor)
    finally:os.close(descriptor)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config',type=Path,required=True)
    args=parser.parse_args(); started=time.monotonic()
    repo=Path(__file__).resolve().parents[3]
    requested=args.config.absolute()
    require(requested.is_relative_to(repo/'eval'), 'release configuration must be a repository eval file')
    config_path=safe_file(repo, str(requested.relative_to(repo)))
    config_raw=config_path.read_bytes();config=document(config_raw)
    require(len(config['automatic_builds']) == 2 and {row['label'] for row in config['automatic_builds']} == {'historical', 'current'}, 'release must retain historical and current automatic coverage reports')
    cache=Path.home()/'.cache/lysilogy';corpus=Path.home()/'Corpora/arxiv';data=cache/'arxiv-kb-data'
    implementation=fingerprint_sources()
    before=verify_evidence(cache,config);require(before==config['evidence_sha256'],'release external evidence differs from frozen hashes')
    assemblies=[]
    for paper in config['papers']:
        assemblies.append(assemble(cache,corpus,data,paper['candidate'],paper.get('region_bundle'),paper.get('panel_bundle'),config['inputs'],config['indexes'],paper.get('object_bundle'),paper.get('bibliography_bundle')))
    history=[]
    for row in config['automatic_builds']:
        report=document(bounded(cache,row['path']))
        require(report['input_sha256'] == before[config['inputs']] and report['index_map_sha256'] == before[config['indexes']], 'automatic coverage report used different frozen inputs')
        summaries=bounded(cache,row['paper_summaries']).splitlines()
        summary_rows=[document(line) for line in summaries]
        require(len(summary_rows) == report['papers'] and len({item['arxiv_id'] for item in summary_rows}) == report['papers'], 'automatic per-paper summary inventory is incomplete or duplicated')
        require({item['arxiv_id'] for item in summary_rows} == {item['arxiv_id'] for item in document(bounded(cache,config['inputs']))['papers']}, 'automatic coverage summaries differ from the frozen eval population')
        history.append({'path':row['path'],'sha256':before[row['path']],'paper_summaries':row['paper_summaries'],'paper_summaries_sha256':before[row['paper_summaries']],'label':row['label'],'summary':{key:report[key] for key in ('head','papers','counts','eligible_metrics','wall_seconds','peak_rss_kib','network_calls','model_calls','external_cost_usd')}})
    release,bibliography=build_release(assemblies,config,document(bounded(cache,config['inputs'])),history)
    require(before==verify_evidence(cache,config) and implementation==fingerprint_sources() and config_path.read_bytes()==config_raw,'release inputs or implementation changed during assembly')
    release['provenance']={'build_config_sha256':sha256(config_raw),'implementation':implementation,'evidence_sha256':before}
    bibliography['provenance']=release['provenance']
    payloads={'objects.json':canonical(release)+b'\n','bibliography.json':canonical(bibliography)+b'\n'}
    output=repo/'eval/truth'/config['version'];checked_output_ancestors(output);output.parent.mkdir(parents=True,exist_ok=True)
    write_immutable(output,payloads)
    receipt={'schema_version':1,'release_version':config['version'],'release_paths':{str((output/name).relative_to(repo)):sha256(raw) for name,raw in payloads.items()},'implementation':implementation,'config_sha256':sha256(config_raw),'assembled_at':datetime.now(timezone.utc).isoformat(),'wall_seconds':time.monotonic()-started,'network_calls':0,'model_calls':0,'external_call_cost_usd':0,'prior_agent_judgment_cost_usd':None,'coverage':release['coverage']}
    atomic_json(cache/'k1-release-receipts'/config['version']/(sha256(canonical(receipt))+'.json'),receipt)
    print(json.dumps(receipt,sort_keys=True))


if __name__=='__main__':main()
