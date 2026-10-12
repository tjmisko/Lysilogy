"""Synthetic composition and blind-export provenance; no real annotation labels."""
from copy import deepcopy
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch

from annotations import canonical
from archive import sha256
from manual import assemble, compose_overlays, verify_annotator_native_input
from native_exports import FORMAT, native_projection, verify_native_export


def native_fixture():
    index = {'index': {'text': 'A😀 B', 'pages': [{'number': 1, 'start': 0, 'end': 5, 'width': 100, 'height': 100}],
                       'tokens': [{'start': 0, 'end': 3, 'page': 1, 'text': 'A😀', 'rects': [{'x_min': 0, 'y_min': 0, 'x_max': 10, 'y_max': 10}], 'provenance': 'native'}],
                       'figures': [{'prediction': 'must remain hidden'}], 'objects': [{'prediction': 'also hidden'}]}}
    raw = canonical(index); exported = canonical(native_projection(raw, 'a' * 16))
    declaration = {'format': FORMAT, 'index_sha256': sha256(raw), 'sha256': sha256(exported), 'bytes': len(exported)}
    return raw, exported, declaration


class ManualCompositionTests(unittest.TestCase):
    def test_should_combine_separately_validated_overlays_when_both_use_the_same_candidate(self):
        base = {'automatic_accepted': False, 'objects': [], 'excluded': ['retained']}
        visual = {**deepcopy(base), 'manual_figure_table_overlay': {'objects': [{'id': 'visual', 'kind': 'figure'}]}}
        math = {**deepcopy(base), 'manual_object_overlay': {'objects': [{'id': 'math', 'kind': 'equation'}], 'reviewed_absent_kinds': ['table']}}
        result = compose_overlays(canonical(base), [visual, math])
        self.assertEqual({key: result[key] for key in base}, base)
        self.assertIn('manual_figure_table_overlay', result); self.assertIn('manual_object_overlay', result)

    def test_should_reject_contradictions_when_overlay_candidates_or_kind_claims_disagree(self):
        base = {'accepted': False, 'nullable': None}
        visual = {**base, 'manual_figure_table_overlay': {'objects': [{'id': 'visual', 'kind': 'figure'}]}}
        math = {**base, 'manual_object_overlay': {'objects': [{'id': 'math', 'kind': 'equation'}], 'reviewed_absent_kinds': []}}
        mutations = [lambda v, m: m.update(accepted=True), lambda v, m: m.pop('nullable'),
                     lambda v, m: m['manual_object_overlay']['objects'][0].update(id='visual'),
                     lambda v, m: m['manual_object_overlay']['reviewed_absent_kinds'].append('figure')]
        for mutation in mutations:
            v, m = deepcopy(visual), deepcopy(math); mutation(v, m)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): compose_overlays(canonical(base), [v, m])

    def test_should_validate_both_bundles_against_original_bytes_when_assembly_combines_them(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory); (cache/'regions').mkdir()
            (cache/'regions/regions-root-v1.json').write_text('{}')
            base = {'arxiv_id': 'synthetic', 'paper_id': 'a' * 16}
            raw = canonical(base)
            (cache/'candidate.json').write_bytes(raw)
            (cache/'inputs.json').write_bytes(canonical({'papers': [{'arxiv_id': 'synthetic'}]}))
            (cache/'indexes.json').write_bytes(canonical({'papers': [{'paper_id': 'a' * 16}]}))
            visual = {**base, 'manual_figure_table_overlay': {'objects': [{'id': 'figure', 'kind': 'figure'}]}}
            math = {**base, 'manual_object_overlay': {'objects': [{'id': 'equation', 'kind': 'equation'}], 'reviewed_absent_kinds': []}}
            with patch('manual.attach_objects', return_value=math) as objects, patch('manual.attach_manual_regions', return_value=visual) as regions:
                result = assemble(cache, cache, cache, 'candidate.json', 'regions', inputs_relative='inputs.json', indexes_relative='indexes.json', object_relative='math')
            self.assertEqual(objects.call_args.args[3], raw); self.assertEqual(regions.call_args.args[0], raw)
            self.assertIn('manual_figure_table_overlay', result); self.assertIn('manual_object_overlay', result)
            self.assertEqual(result['manual_assembly']['omitted_manual_kinds'], ['bib_entry'])

    def test_should_reject_candidate_type_changes_when_python_values_compare_equal(self):
        base = {'accepted': False, 'counts': {'objects': 0}, 'quality': 1.0}
        for key, changed in [('accepted', 0), ('counts', {'objects': False}), ('quality', 1)]:
            supplied = {**deepcopy(base), key: changed, 'manual_object_overlay': {'objects': []}}
            self.assertEqual({name: supplied[name] for name in base}, base)
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'changed the original candidate'):
                compose_overlays(canonical(base), [supplied])

    def test_should_dispatch_only_an_explicit_tranche_when_no_legacy_bundle_is_supplied(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            (cache/'candidate.json').write_bytes(canonical({'arxiv_id': 'synthetic', 'paper_id': 'paper'}))
            (cache/'inputs.json').write_bytes(canonical({'papers': [{'arxiv_id': 'synthetic'}]}))
            (cache/'indexes.json').write_bytes(canonical({'papers': [{'paper_id': 'paper'}]}))
            with patch('tranche.attach_tranche', return_value={'retained': True}) as tranche, patch('manual.attach_objects') as legacy:
                (cache / 'new-format').mkdir()
                (cache / 'new-format/manifest.json').write_bytes(canonical({'format': 'k1-manual-tranche-v1'}))
                output = assemble(cache, cache, cache, 'candidate.json', None, inputs_relative='inputs.json', indexes_relative='indexes.json', tranche_relative='new-format')
                self.assertEqual(output, {'retained': True}); tranche.assert_called_once(); legacy.assert_not_called()
                with self.assertRaisesRegex(ValueError, 'legacy bundle arguments'):
                    assemble(cache, cache, cache, 'candidate.json', 'legacy', inputs_relative='inputs.json', indexes_relative='indexes.json', tranche_relative='new-format')

    def test_should_exclude_predictions_and_preserve_utf16_text_when_native_export_is_verified(self):
        raw, exported, declaration = native_fixture()
        self.assertNotIn(b'prediction', exported)
        self.assertEqual(native_projection(raw, 'a' * 16)['page_text'], [{'page': 1, 'text': 'A😀 B'}])
        self.assertEqual(verify_native_export(raw, exported, 'a' * 16, declaration)['index_sha256'], sha256(raw))

    def test_should_reject_resealed_export_changes_when_native_members_or_allowed_fields_disagree(self):
        raw, _, declaration = native_fixture()
        mutations = [lambda e: e.update(text='different'), lambda e: e.update(figures=[]),
                     lambda e: e['tokens'][0].update(start=1), lambda e: e['tokens'][0]['rects'][0].update(x_max=11),
                     lambda e: e['page_text'][0].update(text='different'), lambda e: e.update(paper_id='b' * 16)]
        for mutation in mutations:
            value = native_projection(raw, 'a' * 16); mutation(value); changed = canonical(value)
            claim = {**declaration, 'sha256': sha256(changed), 'bytes': len(changed)}
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'fixed native allowlist'):
                verify_native_export(raw, changed, 'a' * 16, claim)

    def test_should_reject_foreign_format_or_index_when_a_blind_export_claims_another_origin(self):
        raw, exported, declaration = native_fixture()
        for key, value in [('format', 'another-format'), ('index_sha256', 'f' * 64), ('sha256', 'f' * 64), ('bytes', len(exported) + 1)]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                verify_native_export(raw, exported, 'a' * 16, {**declaration, key: value})

    def test_should_reject_resealed_native_type_changes_when_python_values_compare_equal(self):
        raw, _, declaration = native_fixture()
        mutations = [lambda e: e['tokens'][0].update(start=False),
                     lambda e: e['tokens'][0].update(page=True),
                     lambda e: e['pages'][0].update(width=100.0),
                     lambda e: e['tokens'][0]['rects'][0].update(x_min=False)]
        for mutation in mutations:
            value = native_projection(raw, 'a' * 16); mutation(value)
            self.assertEqual(value, native_projection(raw, 'a' * 16))
            changed = canonical(value)
            claim = {**declaration, 'sha256': sha256(changed), 'bytes': len(changed)}
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'fixed native allowlist'):
                verify_native_export(raw, changed, 'a' * 16, claim)

    def test_should_rehash_actual_blind_export_when_annotators_used_its_safe_cache_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); data = root/'data'; data.mkdir()
            raw, exported, declaration = native_fixture()
            (data/'index.json').write_bytes(raw); (root/'native.json').write_bytes(exported)
            mapped = {'paper_id': 'a' * 16, 'index': {'path': 'index.json', 'sha256': sha256(raw)}}
            inputs = {'reading_index': {'sha256': sha256(raw)}, 'native_export': {**declaration, 'path': str(root/'native.json')}}
            self.assertEqual(verify_annotator_native_input(root, data, mapped, inputs)['sha256'], sha256(exported))
            (root/'native.json').write_bytes(b'changed')
            with self.assertRaises(ValueError): verify_annotator_native_input(root, data, mapped, inputs)

    def test_should_reject_unverified_export_paths_when_claims_escape_or_follow_links(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); data = root/'data'; data.mkdir()
            raw, exported, declaration = native_fixture()
            (data/'index.json').write_bytes(raw); (root/'native.json').write_bytes(exported)
            (root/'alias.json').symlink_to(root/'native.json')
            mapped = {'paper_id': 'a' * 16, 'index': {'path': 'index.json', 'sha256': sha256(raw)}}
            for path in (str(root/'alias.json'), str(root/'../native.json'), 'native.json'):
                inputs = {'reading_index': {'sha256': sha256(raw)}, 'native_export': {**declaration, 'path': path}}
                with self.subTest(path=path), self.assertRaises(ValueError): verify_annotator_native_input(root, data, mapped, inputs)


if __name__ == '__main__': unittest.main()
