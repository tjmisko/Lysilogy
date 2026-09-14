"""Inert BibTeX lexical fixtures; no system BibTeX/TeX dependency or truth labels."""
import unittest

from archive import Limits, UnsupportedSource, sha256
from parser import bibtex_fields, parse_project
from tex import Renderer


class BibtexTests(unittest.TestCase):
    def scan(self, source, limits=Limits()):
        evidence = []
        result, issues = bibtex_fields({'references.bib': source}, Renderer(limits=limits), evidence)
        return result, issues, evidence[0]

    def test_should_preserve_literal_percent_when_quoted_or_braced_metadata_is_complete(self):
        for note in ('"%%CITATION = TEST;%%"', '{100% retained}'):
            source = '@misc{case,title={Ordinary title},note=' + note + '}\n'
            labels, issues, evidence = self.scan(source)
            self.assertEqual(labels['case']['labels'], {'title': 'Ordinary title'})
            self.assertFalse(issues)
            field = evidence['entries'][0]['fields'][1]
            part = field['parts'][0]
            self.assertEqual(source[part['start']:part['end']], note)
            self.assertEqual(source[part['content_start']:part['content_end']], note[1:-1])
            self.assertEqual(field['sha256'], sha256(('note=' + note).encode()))

    def test_should_withhold_a_whole_label_when_literal_percent_is_not_faithfully_rendered(self):
        for value in ('"A 100% complete title"', '{A 100% complete title}'):
            source = '@misc{case,title=' + value + ',year=2020}\n'
            labels, issues, evidence = self.scan(source)
            self.assertEqual(labels['case']['labels'], {'year': '2020'})
            self.assertEqual(issues['bibtex_literal_percent_rendering'], 1)
            self.assertEqual(evidence['entries'][0]['fields'][0]['unsupported'], ['literal_percent_rendering'])
        labels, issues, _ = self.scan(r'@misc{case,title={A 100\% complete title}}')
        self.assertEqual(labels['case']['labels']['title'], 'A 100% complete title')
        self.assertFalse(issues)

    def test_should_preserve_internal_quotes_when_braces_protect_the_quoted_field(self):
        source = '@misc{case,title="{A stochastic "noise vector" approach}",year=2020}'
        labels, issues, evidence = self.scan(source)
        self.assertEqual(labels['case']['labels']['title'], 'A stochastic "noise vector" approach')
        self.assertFalse(issues)
        field = evidence['entries'][0]['fields'][0]
        self.assertEqual(source[field['start']:field['end']], 'title="{A stochastic "noise vector" approach}"')
        provenance = labels['case']['provenance']['title']
        self.assertEqual(provenance['field_sha256'], field['sha256'])
        self.assertEqual(provenance['value_part'], field['parts'][0])

    def test_should_count_bibtex_braces_when_backslashes_precede_delimiters(self):
        for value in (r'{A \"o accent}', r'"A {\"o} accent"', r'"A \{x\} token"'):
            source = '@misc{case,title=' + value + '}'
            _, _, evidence = self.scan(source)
            part = evidence['entries'][0]['fields'][0]['parts'][0]
            self.assertEqual(source[part['content_start']:part['content_end']], value[1:-1])
        for value in (r'"A \"quoted\" title"', r'{A \{ token}'):
            with self.subTest(value=value), self.assertRaises(UnsupportedSource):
                self.scan('@misc{case,title=' + value + ',year=2020}')

    def test_should_keep_fake_entry_syntax_inside_values_when_entry_delimiters_differ(self):
        for opening, closing in (('{', '}'), ('(', ')')):
            source = '@misc' + opening + 'case,title="Hello @misc{fake,title={x}} World",note={literal ) token}' + closing
            labels, issues, evidence = self.scan(source)
            self.assertEqual(list(labels), ['case'])
            self.assertEqual(labels['case']['labels']['title'], 'Hello @miscfake,title=x World')
            self.assertFalse(issues)
            self.assertEqual(len(evidence['entries']), 1)
            self.assertEqual(source[evidence['entries'][0]['end'] - 1], closing)

    def test_should_ignore_outer_ordinary_text_when_a_real_command_begins_later(self):
        source = 'outside "unbalanced } text % still outside\n@misc{case,title={Live}}\n'
        labels, issues, evidence = self.scan(source)
        self.assertEqual(labels['case']['labels']['title'], 'Live')
        self.assertFalse(issues)
        self.assertEqual(evidence['entries'][0]['start'], source.index('@misc'))
        # At the outer database level a percent does not hide an @ command.
        labels, _, _ = self.scan('outside % @misc{case,title={Live}}')
        self.assertEqual(labels['case']['labels']['title'], 'Live')

    def test_should_resume_at_the_next_line_when_a_comment_has_multiline_brace_text(self):
        source = '@comment{ignored\n@misc{embedded,title={Embedded}}\n}\n@misc{case,title={Live}}'
        labels, issues, evidence = self.scan(source)
        self.assertEqual(list(labels), ['embedded', 'case'])
        self.assertFalse(issues)
        self.assertEqual(evidence['entries'][0]['type'], 'comment')
        self.assertEqual(source[:evidence['entries'][0]['end']], '@comment{ignored\n')
        for spelling in ('@COMMENT arbitrary ) } [', '@comment(ignored)', '@comment{ignored}'):
            labels, _, _ = self.scan(spelling + '\n@misc{case,title={Live}}')
            self.assertEqual(list(labels), ['case'])

    def test_should_reject_inline_command_search_when_outer_line_semantics_are_unsupported(self):
        for source in ('@misc{a,title={First}} @misc{b,title={Second}}',
                       '@comment{outside @misc{hidden,title={Hidden}}}',
                       '@comment(ignored @misc{same,title={Same}}',
                       '@string{foo="String"} @misc{case,title=foo}'):
            with self.subTest(source=source), self.assertRaisesRegex(UnsupportedSource, 'inline|one physical line'):
                self.scan(source)
        labels, _, _ = self.scan('@misc{a,title={First}}\n@misc{b,title={Second}}')
        self.assertEqual(list(labels), ['a', 'b'])

    def test_should_reject_complete_prefixes_when_their_remaining_field_syntax_is_malformed(self):
        for tail in ('title="unterminated', 'title={unterminated', 'title="unbalanced } title"}',
                     'title={Known},year=', 'title={Known},year=2020',
                     'title={Known} junk}', 'title={Known},% note={Not a comment}\n}',
                     'title={Known},,year=2020}', 'title={Known},year=2020oops}',
                     'title={Known},note=foo # {unterminated}', 'title={Known},note=foo # }',
                     'title={Known},note=foo {unjoined}}', 'title={Known},note=foo, bad token}',
                     'title={Known})'):
            with self.subTest(tail=tail), self.assertRaises(UnsupportedSource):
                self.scan('@misc{case,' + tail)

    def test_should_keep_macro_and_concatenation_unknown_when_later_fields_are_valid(self):
        source = '@misc{case,title=prefix # {A, B @misc{not_an_entry}} # " tail",author=people,year=2020}'
        labels, issues, evidence = self.scan(source)
        self.assertEqual(labels['case']['labels'], {'year': '2020'})
        self.assertEqual(issues['bibtex_concatenated_field'], 1)
        self.assertEqual(issues['bibtex_unresolved_field_macro'], 2)
        fields = evidence['entries'][0]['fields']
        self.assertEqual(len(fields), 3)
        self.assertEqual(len(fields[0]['parts']), 3)
        self.assertEqual(source[fields[0]['start']:fields[0]['end']],
                         'title=prefix # {A, B @misc{not_an_entry}} # " tail"')

    def test_should_retain_directives_without_evaluating_them_when_the_database_uses_strings_or_preambles(self):
        source = '@string{known="A string"}\n@preamble{"\\newcommand" # { something}}\n@misc{case,title=known,year=2020}'
        labels, issues, evidence = self.scan(source)
        self.assertEqual(labels['case']['labels'], {'year': '2020'})
        self.assertEqual(issues['bibtex_string_macro'], 1)
        self.assertEqual(issues['bibtex_preamble_not_evaluated'], 1)
        self.assertEqual([row['type'] for row in evidence['entries']], ['string', 'preamble', 'misc'])
        for directive in ('@string{known="unterminated}', '@preamble{{unterminated}',
                          '@string{known="A",other="B"}', '@preamble{"A" # }'):
            with self.subTest(directive=directive), self.assertRaises(UnsupportedSource):
                self.scan(directive + '\n@misc{case,title={Known}}')

    def test_should_withhold_every_duplicate_field_when_field_names_differ_only_in_case(self):
        labels, issues, evidence = self.scan('@misc{case,title={First},TITLE={Second},year=2020}')
        self.assertEqual(labels['case']['labels'], {'year': '2020'})
        self.assertEqual(issues['duplicate_bibtex_field'], 2)
        self.assertEqual([field['unsupported'] for field in evidence['entries'][0]['fields'][:2]],
                         [['duplicate_field'], ['duplicate_field']])

    def test_should_withhold_all_key_spellings_when_duplicate_keys_cross_entries_or_files(self):
        evidence = []
        files = {'a.bib': '@misc{Case,title={First}}\n@misc{case,title={Second}}',
                 'b.bib': '@misc{CASE,title={Third}}'}
        labels, issues = bibtex_fields(files, Renderer(), evidence)
        self.assertEqual(labels, {'Case': None, 'case': None, 'CASE': None})
        self.assertEqual(issues['duplicate_bibtex_key'], 2)
        self.assertEqual(sum(len(member['entries']) for member in evidence), 3)

    def test_should_preserve_literal_apostrophes_when_keys_start_with_or_contain_them(self):
        for key in ("O'Key", "'initial", "a'b'c"):
            for opening, closing in (('{', '}'), ('(', ')')):
                source = '@misc' + opening + key + ',title={Ordinary title}' + closing
                labels, issues, evidence = self.scan(source)
                self.assertEqual(list(labels), [key])
                self.assertEqual(labels[key]['labels']['title'], 'Ordinary title')
                self.assertFalse(issues)
                row = evidence['entries'][0]
                self.assertEqual(row['key'], key)
                self.assertEqual(source[row['key_span']['start']:row['key_span']['end']], key)
                self.assertEqual(labels[key]['provenance']['title']['key'], key)

    def test_should_withhold_all_case_variants_when_apostrophe_keys_are_duplicated(self):
        source = "@misc{O'Key,title={First}}\n@misc{o'key,title={Second}}"
        labels, issues, evidence = self.scan(source)
        self.assertEqual(labels, {"O'Key": None, "o'key": None})
        self.assertEqual(issues['duplicate_bibtex_key'], 1)
        self.assertEqual([entry['key'] for entry in evidence['entries']], ["O'Key", "o'key"])
        for row in evidence['entries']:
            self.assertEqual(source[row['key_span']['start']:row['key_span']['end']], row['key'])

    def test_should_bind_decoded_offsets_and_complete_field_hashes_when_unicode_precedes_fields(self):
        source = 'Résumé outside\n@misc{case, title = "A {nested \"quote\"} title",\n year = 2020 }\n'
        labels, _, evidence = self.scan(source)
        self.assertEqual(evidence['decoded_sha256'], sha256(source.encode()))
        row = evidence['entries'][0]
        self.assertEqual(source[row['key_span']['start']:row['key_span']['end']], 'case')
        for field in row['fields']:
            self.assertEqual(field['sha256'], sha256(source[field['start']:field['end']].encode()))
            provenance = labels['case']['provenance'][field['name']]
            self.assertEqual(provenance['decoded_sha256'], evidence['decoded_sha256'])
            self.assertEqual(provenance['field_span'], {'start': field['start'], 'end': field['end']})
            self.assertEqual(provenance['entry_span'], {'start': row['start'], 'end': row['end']})

    def test_should_bound_database_work_when_bytes_records_or_brace_nesting_exceed_limits(self):
        for source, limits in (('@misc{case,title={abc}}', Limits(member_bytes=8)),
                               ('@misc{case,title={abc}}', Limits(text_bytes=8)),
                               ('@misc{case,title={{{deep}}}}', Limits(group_depth=2)),
                               ('@misc{case,title={abc},year=2020}', Limits(expansion_steps=3)),
                               ('@misc{case,title={a} # {b} # {c}}', Limits(expansion_steps=4))):
            with self.subTest(limits=limits), self.assertRaisesRegex(UnsupportedSource, 'bound'):
                self.scan(source, limits)
        with self.assertRaisesRegex(UnsupportedSource, 'cumulative'):
            bibtex_fields({'a.bib': '@misc{a,title={a}}', 'b.bib': '@misc{b,title={b}}'},
                          Renderer(limits=Limits(text_bytes=25)))

    def test_should_keep_printed_roles_authoritative_when_database_fields_have_no_printed_boundaries(self):
        body = r'\begin{thebibliography}{9}\bibitem{k}\bibinfo{author}{Actual Author}. \bibinfo{title}{Printed title}. \bibinfo{year}{2020}.\end{thebibliography}'
        source = r'\documentclass{article}\begin{document}\bibliography{refs}\end{document}'
        parsed = parse_project({'main.tex': source, 'main.bbl': body, 'refs.bib': '@misc{k,title={Database title},note="%%CITATION = TEST;%%",year=2020}'})
        entry = parsed['entries'][0]
        self.assertEqual(entry['field_labels'], {'first_author': 'Actual Author', 'title': 'Printed title', 'year': '2020'})
        self.assertEqual([row['field'] for row in entry['field_conflicts']], ['title'])
        self.assertEqual(len(parsed['coverage']['bibtex_source'][0]['entries'][0]['fields']), 3)
        self.assertIn('field_span', entry['field_conflicts'][0]['provenance'])


if __name__ == '__main__':
    unittest.main()
