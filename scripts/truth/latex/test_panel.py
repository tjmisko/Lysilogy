"""Offline arithmetic fixtures; no synthetic vote becomes panel truth."""
import unittest

from panel import score_frozen_panels, score_panel


class PanelTests(unittest.TestCase):
    def test_should_average_all_panelists_when_their_independent_rankings_differ(self):
        result = score_panel(['a', 'b', 'c'], [['a', 'b', 'c'], ['a', 'b', 'd'], ['a', 'd', 'e']], {'a', 'b', 'c', 'd', 'e'})
        self.assertEqual(result['panel_overlaps'], [3, 2, 1])
        self.assertEqual(result['numerator'], 6)
        self.assertEqual(result['denominator'], 9)
        self.assertEqual(result['agreement'], 2 / 3)

    def test_should_keep_missing_slots_when_predictions_repeat_or_name_invalid_ids(self):
        votes = [['a', 'b', 'c']] * 3
        result = score_panel(['a', 'a', 'unknown', 'b', 'c'], votes, {'a', 'b', 'c'})
        self.assertEqual(result['numerator'], 3)
        self.assertEqual(result['denominator'], 9)
        self.assertEqual(result['missing_top_three_slots'], 2)
        self.assertEqual(result['ignored_later_positions'], 2)

    def test_should_count_unproduced_papers_when_a_frozen_model_result_is_missing(self):
        papers = [{'arxiv_id': key, 'valid_ids': ['a', 'b', 'c'], 'panelists': [['a', 'b', 'c']] * 3} for key in ('one', 'two')]
        result = score_frozen_panels(papers, {'one': ['a', 'b', 'c']})
        self.assertEqual(result['numerator'], 9)
        self.assertEqual(result['denominator'], 18)
        self.assertEqual(result['agreement'], .5)

    def test_should_reject_ambiguous_ranks_when_inputs_are_not_ordered_id_lists(self):
        for value in ({1: 'a'}, ['a', {'rank': 2}], ['a'] * 6):
            with self.assertRaises(ValueError):
                score_panel(value, [['a', 'b', 'c']] * 3, {'a', 'b', 'c'})

    def test_should_reject_invalid_votes_when_panelists_do_not_supply_exactly_three_choices(self):
        for votes in ([['a', 'b', 'c']] * 2, [['a', 'a', 'b']] * 3, [['a', 'b', 'invalid']] * 3, [['a', 'b']] * 3):
            with self.assertRaises(ValueError):
                score_panel(['a'], votes, {'a', 'b', 'c'})


if __name__ == '__main__':
    unittest.main()
