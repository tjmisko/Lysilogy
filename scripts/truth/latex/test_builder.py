"""Offline builder provenance fixtures; no corpus or model calls."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from archive import UnsupportedSource, read_archive, sha256
from builder import choose_main, derive_paper, ordered_papers, publication_problems, safe_file
from test_latex import document, tar
import tarfile


class BuilderTests(unittest.TestCase):
    def test_should_order_all_strata_reproducibly_when_input_order_changes(self):
        rows = [{'arxiv_id': str(number), 'stratum': [number % 3, 2020]} for number in range(12)]
        first = ordered_papers(rows, 'fixed-seed')
        self.assertEqual(first, ordered_papers(list(reversed(rows)), 'fixed-seed'))
        self.assertEqual({row['stratum'][0] for row in first[:3]}, {0, 1, 2})
        self.assertEqual(len(first), 12)

    def test_should_select_unique_title_evidence_when_two_roots_have_different_titles(self):
        files = {'a.tex': document('Body.', r'\title{The sufficiently distinctive first paper title}'),
                 'b.tex': document('Other.', r'\title{A completely different second paper title}')}
        text = 'The sufficiently distinctive first paper title'
        selected, evidence = choose_main(files, {'text': text, 'pages': [{'number': 1, 'start': 0, 'end': len(text)}]})
        self.assertEqual(selected, 'a.tex')
        self.assertIn('PDF first page', evidence['method'])

    def test_should_reject_title_ties_when_different_documents_share_a_title(self):
        title = r'\title{The sufficiently distinctive shared paper title}'
        files = {'a.tex': document('First body.', title), 'b.tex': document('Second body.', title)}
        text = 'The sufficiently distinctive shared paper title'
        with self.assertRaisesRegex(UnsupportedSource, 'non-equivalent'):
            choose_main(files, {'text': text, 'pages': [{'number': 1, 'start': 0, 'end': len(text)}]})

    def test_should_compare_graphics_hashes_when_identical_roots_use_relative_resources(self):
        source = document(r'\includegraphics{image.pdf}')
        files = {'a/main.tex': source, 'b/main.tex': source}
        members = [{'path': 'a/image.pdf', 'sha256': 'a' * 64}, {'path': 'b/image.pdf', 'sha256': 'b' * 64}]
        with self.assertRaisesRegex(UnsupportedSource, 'non-equivalent'):
            choose_main(files, {'text': '', 'pages': []}, members)
        members[1]['sha256'] = members[0]['sha256']
        selected, evidence = choose_main(files, {'text': '', 'pages': []}, members)
        self.assertEqual(selected, 'a/main.tex')
        self.assertEqual(evidence['equivalent_roots'], sorted(files))

    def test_should_hash_binary_members_when_source_equivalence_needs_resource_provenance(self):
        raw = tar([('main.tex', document('A body.').encode(), tarfile.REGTYPE), ('image.pdf', b'\0binary', tarfile.REGTYPE)])
        files, members = read_archive(raw)
        self.assertEqual(list(files), ['main.tex'])
        binary = next(row for row in members if row['path'] == 'image.pdf')
        self.assertEqual(binary['sha256'], sha256(b'\0binary'))
        self.assertIsNone(binary['encoding'])

    def test_should_reject_unsafe_evidence_paths_when_inputs_escape_or_follow_symlinks(self):
        with tempfile.TemporaryDirectory(dir=Path.home() / '.cache/lysilogy') as directory:
            root = Path(directory)
            (root / 'safe.json').write_text('{}')
            (root / 'alias.json').symlink_to(root / 'safe.json')
            for path in ('../safe.json', '/safe.json', 'a/./b', 'a\\b', '.env', '.env.local', '.secrets/data', 'alias.json'):
                with self.subTest(path=path), self.assertRaises(ValueError):
                    safe_file(root, path)
            self.assertEqual(safe_file(root, 'safe.json'), root / 'safe.json')

    def test_should_rehash_parsed_source_when_bytes_change_between_fingerprint_and_read(self):
        with tempfile.TemporaryDirectory(dir=Path.home() / '.cache/lysilogy') as directory:
            root = Path(directory)
            original, replacement, pdf = document('Original.').encode(), document('Replaced.').encode(), b'PDF'
            (root / 'source.src').write_bytes(replacement)
            (root / 'paper.pdf').write_bytes(pdf)
            raw_index = json.dumps({'index': {'text': 'Original.'}}).encode()
            (root / 'index.json').write_bytes(raw_index)
            paper = {'pdf': {'path': 'paper.pdf', 'sha256': sha256(pdf), 'bytes': len(pdf)},
                     'source': {'path': 'source.src', 'sha256': sha256(original), 'bytes': len(original)}}
            mapped = {'index': {'path': 'index.json', 'sha256': sha256(raw_index)}, 'pdf_sha256': sha256(pdf)}
            with patch('builder.fingerprint_file', side_effect=[(sha256(pdf), len(pdf)), (sha256(original), len(original))]):
                with self.assertRaisesRegex(ValueError, 'source bytes read'):
                    derive_paper(paper, mapped, root, root)

    def test_should_keep_partial_builds_unpublished_when_true_coverage_and_panel_are_missing(self):
        snapshot = {'missing_pairs': ['missing'], 'papers': [{'stratum': ['math', 2020]}]}
        problems = publication_problems(snapshot, [])
        self.assertTrue(any('500' in row for row in problems))
        self.assertTrue(any('full eval' in row for row in problems))
        self.assertTrue(any('panel' in row for row in problems))


if __name__ == '__main__':
    unittest.main()
