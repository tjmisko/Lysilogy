"""Offline complete bibliography membership and provenance boundaries."""
from copy import deepcopy
import unittest
import tempfile
from pathlib import Path
from manual import attach_bibliography

from annotations import canonical
from archive import read_archive, sha256
from bibliography_annotations import apply_bibliography_overlay, verified_field_argument, printed_field_agrees
from parser import parse_project


def fixture():
    source = (r'\documentclass{article}\begin{document}See \cite{one,two}.\begin{thebibliography}{9}'
              r'\bibitem{one}\bibinfo{person}{Alice Smith}. \bibinfo{year}{2020}. \showarticletitle{A complete title}.'
              r'\bibitem{two}\bibinfo{person}{Bob Jones}. \bibinfo{year}{2021}. \showarticletitle{Another complete title}.'
              r'\end{thebibliography}\end{document}').encode()
    files, members = read_archive(source); parsed = parse_project(files)
    text = '😀 See [1, 2].\n[1] Alice Smith. 2020. A complete title.\n[2] Bob Jones. 2021. Another complete title.'
    def span(value):
        start = text.index(value); return {'start': len(text[:start].encode('utf-16-le')) // 2, 'end': len(text[:start + len(value)].encode('utf-16-le')) // 2, 'text': value, 'text_utf8_sha256': sha256(value.encode())}
    def native(value):
        row = span(value); return {'start': row['start'], 'end': row['end'], 'native_text_sha256': row['text_utf8_sha256']}
    index = {'index': {'text': text, 'pages': [{'number': 1, 'start': 0, 'end': len(text.encode('utf-16-le'))//2}]}}
    candidate = {'arxiv_id': '2001.00001', 'paper_id': '0123456789abcdef', 'pdf_sha256': '1'*64, 'source_sha256': sha256(source), 'index': {'path': 'papers/actual/index.json', 'sha256': sha256(canonical(index))}, 'source_inventory': parsed, 'source_inventory_sha256': sha256(canonical(parsed)), 'automatic_exclusions': ['retained']}
    image = {'path': 'page-1.png', 'sha256': '2'*64}
    original = {'arxiv_id': candidate['arxiv_id'], 'paper_id': candidate['paper_id'], 'candidate_sha256': sha256(canonical(candidate)), 'inventory_sha256': candidate['source_inventory_sha256'], 'index': candidate['index'], 'images': [image]}
    number = next(i for i, row in enumerate(parsed['links']) if row['kind'] == 'citation'); cite = parsed['links'][number]
    packet = {**original, 'original_packet_sha256': sha256(canonical(original)), 'pdf': {'sha256': candidate['pdf_sha256']}, 'source': {'sha256': candidate['source_sha256']}, 'entries': [{'id': row['id'], 'source_members': row['source_members']} for row in parsed['entries']], 'citations': [{'source_link': number, 'command': cite['command'], 'source_members': cite['source_members'], 'targets': cite['targets']}]}
    roots, rows, reviews = [], [], []
    for i, values in enumerate((('Alice Smith', 'A complete title', '2020'), ('Bob Jones', 'Another complete title', '2021'))):
        item = parsed['entries'][i]; start = text.index(f'[{i+1}] '); end = text.find('\n', start); end = len(text) if end == -1 else end
        entry_text = text[start:end]; fields, prior, reviewed = {}, {}, {}
        for name, value in zip(('first_author','title','year'), values):
            role = {'first_author': 'bibinfo{person}', 'title': 'showarticletitle', 'year': 'bibinfo{year}'}[name]
            source_text = source.decode(); member_start = source_text.index(value)
            member = {'path': next(iter(files)), 'start': member_start, 'end': member_start + len(value)}
            command_start = source_text.rfind('\\'+role, 0, member_start)
            field = {'value': value, 'status': 'known', 'source_role': '\\'+role, 'source_member': member, 'source_payload': value, 'index_span': span(value)}
            fields[name] = field
            prior[name] = {'value': value, 'printed_span': native(value), 'source_role': {'command': role, 'value': value, 'source_command_start': command_start, 'source_command_end': member['end']+1}}
            reviewed[name] = {'value': value, 'span': {k: span(value)[k] for k in ('start','end')}, 'native_text_sha256': sha256(value.encode()), 'source_payload_sha256': sha256(value.encode()), 'disposition': 'accepted; exact'}
        roots.append({'id': item['id'], 'printed_number': str(i+1), 'source_members': item['source_members'], 'spans': [native(entry_text)], 'fields': prior})
        rows.append({'id': item['id'], 'printed_key': str(i+1), 'source_members': item['source_members'], 'members': [span(entry_text)], 'fields': fields, 'field_labels': {name: field['value'] for name,field in fields.items()}, 'page': 1})
        reviews.append({'id': item['id'], 'fields': reviewed})
    root = {**{key: candidate[key] for key in ('arxiv_id','paper_id','pdf_sha256','source_sha256','index','source_inventory_sha256')}, 'annotator': 'root', 'attestation': {'other_annotator_labels_read': False, 'detector_outputs_read': False}, 'candidate_sha256': sha256(canonical(candidate)), 'original_packet_sha256': sha256(canonical(original)), 'bibliography_packet_sha256': sha256(canonical(packet)), 'page_images': [image], 'bibliography_detail_image': image, 'entries': roots, 'citations': [{'source_link': number, 'source_members': cite['source_members'], 'spans': [native('[1, 2]')], 'printed_marker': '[1, 2]', 'targets': ['one','two']}], 'complete_inventory': {'bibliography_entries': 2, 'citation_commands': 1, 'citation_target_pairs': 2, 'first_author_labels': 2, 'title_labels': 2, 'year_labels': 2}}
    independent = {'annotator': 'independent', 'arxiv_id': candidate['arxiv_id'], 'paper_id': candidate['paper_id'], 'independence': {key: False for key in ('bibliography_root_labels_read','root_bibliography_code_read','detector_outputs_read','provider_data_read')}, 'inputs': {'packet': {'sha256': sha256(canonical(packet))}, 'candidate_sha256_from_packet': sha256(canonical(candidate)), 'inventory_sha256_from_packet': candidate['source_inventory_sha256'], 'pdf': {'sha256': candidate['pdf_sha256']}, 'source': {'sha256': candidate['source_sha256']}, 'reading_index': {'sha256': candidate['index']['sha256']}, 'source_members': members, 'images': [image]}, 'entries': rows, 'citations': [{'source_link': number, 'command': cite['command'], 'source_members': cite['source_members'], 'source_target_order': ['one','two'], 'targets': ['one','two'], 'span': span('[1, 2]'), 'location_role': 'body', 'printed_numbers': [1,2]}], 'complete_inventory': {'coverage_complete_for_this_paper': True, 'bibliography_entries': 2, 'citation_groups': 1, 'citation_target_mentions': 2, 'known_fields': {name:2 for name in ('first_author','title','year')}, 'unknown_fields': {name:0 for name in ('first_author','title','year')}}}
    receipt = {'annotation': {}, 'packet_sha256': sha256(canonical(packet)), 'detail_images': [image], 'detail_render': {'viewed_both': True, 'source_pdf_sha256': candidate['pdf_sha256']}}
    comparison = {'schema_version': 1, 'verdict': 'clear_complete_bibliography_overlay', 'findings': [], 'arxiv_id': candidate['arxiv_id'], 'paper_id': candidate['paper_id'], 'entries_verified': ['one','two'], 'field_values_and_anchors_verified': reviews, 'citation_source_links_verified': [number], 'citation_occurrences_verified': [{'source_link': number, 'span': {key: span('[1, 2]')[key] for key in ('start','end')}, 'printed': '[1, 2]', 'targets': ['one','two'], 'location_role': 'body', 'disposition': 'accepted'}], 'citation_target_pairs_verified': [{'source_link': number,'target': target} for target in ('one','two')], 'counts': {'entries': 2, 'known_field_values': 6, 'exact_native_field_anchors': 6, 'citation_groups': 1, 'citation_target_pairs': 2, 'unknown_requested_fields': 0, 'disagreements': 0}}
    result = [candidate,index,source,original,packet,root,independent,receipt,comparison,{'page-1.png':'2'*64}]; reseal(result); return result


def reseal(rows):
    rows[7]['annotation'] = {'sha256': sha256(canonical(rows[6])), 'bytes': len(canonical(rows[6]))}
    rows[8].update(root_annotation_sha256=sha256(canonical(rows[5])), independent_annotation_sha256=sha256(canonical(rows[6])), independent_receipt_sha256=sha256(canonical(rows[7])), bibliography_packet_sha256=sha256(canonical(rows[4])))


def apply(rows):
    return apply_bibliography_overlay(*(row if i in (2,9) else canonical(row) for i,row in enumerate(rows)))


def materialize(cache):
    rows=fixture(); candidate,index,source,original,packet,root,independent,receipt,review,_=rows
    corpus,data,bundle=cache/'corpus',cache/'data',cache/'bundle'
    corpus.mkdir(); (data/'papers/actual').mkdir(parents=True);bundle.mkdir()
    pdf=b'PDF fixture';image=b'render fixture';script=b'inert code receipt'
    candidate['pdf_sha256']=sha256(pdf);root['pdf_sha256']=sha256(pdf)
    paper={'pdf':{'path':'paper.pdf','sha256':sha256(pdf),'bytes':len(pdf)},'source':{'path':'paper.src','sha256':sha256(source),'bytes':len(source)}}
    packet['pdf']=paper['pdf'];packet['source']=paper['source']
    for row in packet['images']+original['images']+root['page_images']+independent['inputs']['images']+receipt['detail_images']+[root['bibliography_detail_image']]:row['sha256']=sha256(image)
    candidate_hash=sha256(canonical(candidate));original['candidate_sha256']=candidate_hash;packet['candidate_sha256']=candidate_hash;root['candidate_sha256']=candidate_hash
    independent['inputs']['candidate_sha256_from_packet']=candidate_hash
    packet['original_packet_sha256']=sha256(canonical(original));root['original_packet_sha256']=sha256(canonical(original))
    packet_hash=sha256(canonical(packet));root['bibliography_packet_sha256']=packet_hash;receipt['packet_sha256']=packet_hash
    independent['inputs']['packet']={'path':str(bundle/'bibliography-packet.json'),'sha256':packet_hash}
    for kind in ('pdf','source'):independent['inputs'][kind]={'path':str(corpus/paper[kind]['path']),'sha256':paper[kind]['sha256']}
    independent['inputs']['reading_index']['path']=str(data/candidate['index']['path'])
    code={'path':str(cache/'serializer.py'),'sha256':sha256(script),'bytes':len(script)}
    receipt['serializer']=code;review['review_script']=code;independent['inputs']['bounded_archive_module']=code;review['input_revalidation']=[]
    receipt['detail_render']['source_pdf_sha256']=sha256(pdf)
    reseal(rows);receipt['annotation']['path']=str(bundle/'bibliography-independent-v1.json');review['independent_receipt_sha256']=sha256(canonical(receipt))
    for name,value in [('packet.json',original),('bibliography-packet.json',packet),('bibliography-root-v1.json',root),('bibliography-independent-v1.json',independent),('bibliography-independent-v1-receipt.json',receipt),('bibliography-reconciliation-independent-v1.json',review)]: (bundle/name).write_bytes(canonical(value))
    (corpus/'paper.pdf').write_bytes(pdf);(corpus/'paper.src').write_bytes(source);(bundle/'page-1.png').write_bytes(image);(cache/'serializer.py').write_bytes(script)
    (data/candidate['index']['path']).write_bytes(canonical(index))
    return (cache,corpus,data,canonical(candidate),paper,{'paper_id':candidate['paper_id'],'index':candidate['index']},'bundle')


class BibliographyAnnotationTests(unittest.TestCase):
    def test_should_preserve_automatic_results_when_complete_independent_labels_are_attached(self):
        rows=fixture(); before=deepcopy(rows[0]); result=apply(rows)
        self.assertEqual({key:result[key] for key in before},before)
        overlay=result['manual_bibliography_overlay']; self.assertEqual(overlay['counts']['citation_target_pairs'],2)
        self.assertEqual(len(overlay['entries']),2); self.assertFalse(overlay['final_k1_publication'])
        self.assertEqual(overlay['mentions'][0]['start'],7)  # UTF-16 emoji prefix

    def test_should_reject_changed_actual_evidence_when_the_safe_file_boundary_assembles_bibliography(self):
        for relative in ('corpus/paper.pdf','corpus/paper.src','bundle/page-1.png','serializer.py'):
            with self.subTest(relative=relative),tempfile.TemporaryDirectory() as directory:
                arguments=materialize(Path(directory))
                self.assertEqual(attach_bibliography(*arguments)['manual_bibliography_overlay']['counts']['entries'],2)
                (Path(directory)/relative).write_bytes(b'changed')
                with self.assertRaises(ValueError):attach_bibliography(*arguments)

    def test_should_reject_omitted_entries_or_citations_when_review_claims_complete_coverage(self):
        for field in ('entries','citations'):
            rows=fixture(); rows[6][field].pop(); reseal(rows)
            with self.subTest(field=field), self.assertRaises(ValueError): apply(rows)

    def test_should_reject_wrong_fields_when_value_role_or_native_membership_disagrees(self):
        mutations=[lambda f:f.update(value='Invented'),lambda f:f['index_span'].update(start=0),lambda f:f['source_member'].update(start=0),lambda f:f.update(source_payload='Invented')]
        for mutate in mutations:
            rows=fixture(); mutate(rows[6]['entries'][0]['fields']['title']);reseal(rows)
            with self.subTest(mutate=mutate),self.assertRaises(ValueError):apply(rows)

    def test_should_reject_dropped_or_reordered_targets_when_grouped_source_citations_are_complete(self):
        for targets in (['one'],['two','one'],['one','one']):
            rows=fixture();rows[6]['citations'][0]['targets']=targets;reseal(rows)
            with self.subTest(targets=targets),self.assertRaises(ValueError):apply(rows)

    def test_should_reject_contradictory_review_when_counts_anchors_or_disposition_change(self):
        mutations=[lambda r:r['counts'].update(entries=1),lambda r:r['citation_occurrences_verified'][0]['span'].update(end=99999),lambda r:r['field_values_and_anchors_verified'][0]['fields']['year'].update(value='1999'),lambda r:r.update(verdict='pending')]
        for mutate in mutations:
            rows=fixture();mutate(rows[8]);reseal(rows)
            with self.subTest(mutate=mutate),self.assertRaises(ValueError):apply(rows)

    def test_should_reject_agreed_fabricated_values_when_source_and_printed_fields_disagree(self):
        for name,value in (('year','1999'),('title','a complete title'),('first_author','Different Author')):
            rows=fixture()
            rows[6]['entries'][0]['fields'][name]['value']=value
            rows[6]['entries'][0]['field_labels'][name]=value
            rows[5]['entries'][0]['fields'][name]['value']=value
            rows[8]['field_values_and_anchors_verified'][0]['fields'][name]['value']=value
            reseal(rows)
            with self.subTest(name=name),self.assertRaisesRegex(ValueError,'complete source field'):apply(rows)

    def test_should_reject_cross_command_payloads_when_reviewed_offsets_extend_past_the_real_argument(self):
        rows=fixture();item=rows[6]['entries'][0]
        item['fields']['year']['source_member']=deepcopy(item['fields']['title']['source_member'])
        rows[5]['entries'][0]['fields']['year']['source_role']['source_command_end']=rows[5]['entries'][0]['fields']['title']['source_role']['source_command_end']
        reseal(rows)
        with self.assertRaisesRegex(ValueError,'exact balanced source argument'):apply(rows)

    def test_should_reject_a_later_person_when_the_first_author_slot_is_already_defined(self):
        text=r'\bibfield{author}{\bibinfo{person}{One} and \bibinfo{person}{Two}}'
        member={'start':text.index('Two'),'end':text.index('Two')+3}
        role={'command':'bibfield{author}/bibinfo{person}','source_command_start':0,'source_command_end':len(text)}
        with self.assertRaisesRegex(ValueError,'exact balanced source argument'):
            verified_field_argument(text,role,member,[{'start':0,'end':len(text)}],'first_author')

    def test_should_only_fold_presentation_when_printed_fields_are_line_wrapped(self):
        self.assertTrue(printed_field_agrees('Complete Feedback','Complete Feed-\n\nback'))
        for printed in ('Incomplete Feedback','complete Feedback','Complete Feed-back','CompleteFeedback'):
            with self.subTest(printed=printed):self.assertFalse(printed_field_agrees('Complete Feedback',printed))

    def test_should_reject_image_substitution_when_annotations_only_claim_old_render_hashes(self):
        rows=fixture();rows[9]['page-1.png']='f'*64
        with self.assertRaisesRegex(ValueError,'actual file paths'):apply(rows)


if __name__=='__main__':unittest.main()
