"""Visual-only admission boundaries with positive, ambiguous and omitted roles."""
from copy import deepcopy
import unittest

from archive import read_archive
from test_tranche_visual import fixture as original_fixture
from tranche_visual_only import (OMITTED_METRICS, pointer, retained_original, retained_source,
                                 visual_projection, visual_source)


def fixture():
    candidate, wrapped, raw, docs, _ = original_fixture(with_links=True)
    p, i = docs['primary'], docs['independent']
    files, _ = read_archive(raw)
    source_id = candidate['source_inventory']['objects'][0]['id']
    construction = {'visuals': [{'source_id': source_id, 'primary': '/objects/0', 'independent': '/objects/0',
        'region': {'page': 1, 'rect': {'x_min': 9, 'y_min': 12, 'x_max': 60, 'y_max': 42}},
        'body_basis': {'artifact': 'geometry', 'pointer': '/regions/0'}}],
        'source_visual_exclusions': [], 'complete_visual_counts': {'figure': 0, 'table': 1},
        'reviewed_absent_visual_kinds': ['figure']}
    data = [construction, p, i, candidate['source_inventory'], files, wrapped['index']]
    seal(data)
    return data


def seal(data):
    c, p, i, inventory, _, _ = data
    c['retained_inventory'] = {
        'primary': retained_original(p, {r['primary'] for r in c['visuals']}),
        'independent': retained_original(i, {r['independent'] for r in c['visuals']}),
        'current_source': retained_source(inventory, {r['source_id'] for r in c['visuals']},
                                         {r['source_id']: r for r in c['source_visual_exclusions']})}


class VisualOnlyProjectionTests(unittest.TestCase):
    def test_should_retain_reference_and_bibliography_roles_when_only_visuals_are_projected(self):
        data = fixture(); before = deepcopy(data)
        result = visual_projection(*data)
        self.assertEqual(data, before)
        self.assertEqual(result['metric_eligibility'], {'O1': True, 'O2': True})
        self.assertEqual(result['omitted_metrics'], [f'O{i}' for i in range(3, 12)])
        self.assertEqual(len(result['retained_inventory']['current_source']['links']), 3)
        self.assertTrue(all(r['disposition'] == 'retained_unscored_source'
                            for r in result['retained_inventory']['current_source']['links']))
        self.assertEqual(len(result['retained_inventory']['current_source']['entries']), 1)
        self.assertNotIn('references', result)

    def test_should_retain_formal_positive_and_unknown_target_when_visual_inventory_is_empty(self):
        data = fixture(); c, p, i, parsed, _, _ = data
        for original in (p, i):
            original['objects'] = [{'id': 'theorem', 'kind': 'statement', 'child_objects': ['proof'],
                                    'fidelity_gaps': ['original source rendering is unverified']},
                                   {'id': 'proof', 'kind': 'proof', 'parent_object': 'theorem'}]
            original['references'].append({'id': 'missing', 'kind': 'statement_reference', 'target': None,
                                            'ambiguity': 'The printed lemma does not exist.'})
        parsed['objects'] = [{**parsed['objects'][0], 'id': 'theorem', 'kind': 'statement'}]
        c.update(visuals=[], complete_visual_counts={'figure': 0, 'table': 0},
                 reviewed_absent_visual_kinds=['figure', 'table'])
        seal(data); result = visual_projection(*data)
        self.assertEqual(result['objects'], [])
        self.assertEqual(result['reviewed_absent_kinds'], ['figure', 'table'])
        self.assertEqual(result['omitted_metrics'], OMITTED_METRICS)
        roles = next(r for r in result['retained_inventory']['primary'] if r['pointer'] == '/objects')
        self.assertEqual(roles['collection_count'], 2)
        self.assertEqual(roles['records'][1]['original_roles']['parent_object'], 'theorem')

    def test_should_reject_omitted_source_visual_when_original_annotations_do_not_include_it(self):
        data = fixture(); extra = deepcopy(data[3]['objects'][0]); extra['id'] = 'unaccounted'
        data[3]['objects'].append(extra); seal(data)
        with self.assertRaisesRegex(ValueError, 'syntactic visual source role is unaccounted'):
            visual_projection(*data)

    def test_should_reject_omitted_original_visual_when_source_crosswalk_is_short(self):
        data = fixture(); data[0]['visuals'] = []; seal(data)
        with self.assertRaisesRegex(ValueError, 'original visual inventory is incomplete'):
            visual_projection(*data)

    def test_should_reject_reclassified_visual_when_only_aggregate_counts_still_match(self):
        data = fixture()
        for original in data[1:3]: original['objects'][0]['kind'] = 'figure'
        data[0]['complete_visual_counts'] = {'figure': 1, 'table': 0}; seal(data)
        with self.assertRaisesRegex(ValueError, 'per-occurrence visual source kind differs'):
            visual_projection(*data)

    def test_should_reject_reused_original_when_source_ids_are_distinct(self):
        data = fixture(); c = data[0]
        other = deepcopy(data[3]['objects'][0]); other['id'] = 'second'
        data[3]['objects'].append(other)
        c['visuals'].append({**deepcopy(c['visuals'][0]), 'source_id': 'second'}); seal(data)
        with self.assertRaisesRegex(ValueError, 'original visual occurrence is reused'):
            visual_projection(*data)

    def test_should_reject_forged_original_caption_when_fresh_record_hashes_are_supplied(self):
        data = fixture()
        data[2]['objects'][0]['caption_native']['text'] = 'altered original'; seal(data)
        with self.assertRaises(ValueError): visual_projection(*data)

    def test_should_reject_empty_or_cross_page_caption_when_numbers_look_valid(self):
        for case in ('empty', 'other_page'):
            with self.subTest(case=case):
                data = fixture()
                if case == 'empty':
                    data[1]['objects'][0]['native_caption_members'] = []
                    data[2]['objects'][0]['caption_native_members'] = []
                else:
                    data[0]['visuals'][0]['region']['page'] = 2
                    data[5]['pages'].append({'number': 2, 'start': len(data[5]['text']),
                                             'end': len(data[5]['text']), 'width': 150, 'height': 150})
                seal(data)
                with self.assertRaises(ValueError): visual_projection(*data)

    def test_should_reject_missing_formal_ledger_row_when_visual_membership_is_unchanged(self):
        data = fixture(); data[0]['retained_inventory']['current_source']['links'].pop()
        with self.assertRaisesRegex(ValueError, 'retained unscored roles'):
            visual_projection(*data)

    def test_should_reject_unverified_source_label_when_original_caption_agrees(self):
        data = fixture(); data[3]['unverified_label_names'] = ['tab:a']
        seal(data)
        with self.assertRaisesRegex(ValueError, 'ambiguous or unverified ownership'):
            visual_projection(*data)

    def test_should_reject_nonfinite_bool_and_extra_region_fields_when_body_is_reviewed(self):
        for case in ('bool_page', 'bool_coordinate', 'nan', 'infinity', 'extra', 'backwards'):
            with self.subTest(case=case):
                data = fixture(); region = data[0]['visuals'][0]['region']
                if case == 'bool_page': region['page'] = True
                elif case == 'bool_coordinate': region['rect']['x_min'] = True
                elif case == 'nan': region['rect']['x_min'] = float('nan')
                elif case == 'infinity': region['rect']['x_max'] = float('inf')
                elif case == 'extra': region['prediction'] = 'not original evidence'
                else: region['rect']['x_max'] = 1
                with self.assertRaises(ValueError): visual_projection(*data)

    def test_should_retain_all_empty_collections_when_original_role_lists_are_empty(self):
        value = {'objects': [], 'definitions': [], 'nonvisual': {'unknown': None}}
        output = retained_original(value, set())
        self.assertEqual([row['pointer'] for row in output], ['/objects', '/definitions', '/nonvisual'])
        self.assertEqual(output[1]['collection_count'], 0)

    def test_should_resolve_only_canonical_pointers_when_original_keys_contain_slashes(self):
        value = {'a/b': [{'~': 'value'}]}
        self.assertEqual(pointer(value, '/a~1b/0/~0'), 'value')
        for path in ('a/b', '/a~1b/00', '/a~1b/-1', '/a~2b/0', '/a~1b/1'):
            with self.subTest(path=path), self.assertRaises(ValueError): pointer(value, path)

    def test_should_preserve_exact_source_expansion_when_a_visual_includes_one_text_member(self):
        before = r'\begin{figure}\include{plot}'
        after = r'\caption{Plot}\label{fig:a}\end{figure}'
        main = before + after; dependency = r'\begin{tikzpicture}drawing\end{tikzpicture}'
        files = {'main.tex': main, 'plot.tex': dependency}
        def src(name, start, end):
            return {'member': name, 'start': start, 'end': end, 'text': files[name][start:end]}
        env = src('main.tex', 0, len(main)); included = src('plot.tex', 0, len(dependency))
        p = {'source_members': [env, included]}
        i = {'source': env, 'source_inclusion': src('main.tex', len(r'\begin{figure}'), len(before)),
             'source_dependency_members': ['plot.tex']}
        parts = [{'path': 'main.tex', 'start': 0, 'end': len(r'\begin{figure}')},
                 {'path': 'plot.tex', 'start': 0, 'end': len(dependency)},
                 {'path': 'main.tex', 'start': len(before), 'end': len(main)}]
        self.assertEqual(visual_source({'source_members': parts}, p, i, files)[0], parts)
        for case in ('missing_dependency', 'reordered', 'shortened', 'extra_primary'):
            with self.subTest(case=case):
                a, b, current = deepcopy(p), deepcopy(i), deepcopy(parts)
                if case == 'missing_dependency': b['source_dependency_members'] = []
                elif case == 'reordered': current.reverse()
                elif case == 'shortened': current[1]['end'] -= 1
                else: a['source_members'].append(deepcopy(env))
                with self.assertRaises(ValueError): visual_source({'source_members': current}, a, b, files)


if __name__ == '__main__':
    unittest.main()
