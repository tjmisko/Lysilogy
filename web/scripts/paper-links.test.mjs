import assert from 'node:assert/strict';
import test from 'node:test';
import { discoverPaperLinks as projectPaperLinks, hintCodes } from '../src/lib/paperLinks.ts';
import { backendObjects, fixtureGeneration } from './backend-objects-fixture.mjs';
import { safeLinkUrl, nativePageLinks } from '../src/lib/pdfNativeLinks.ts';
import { placeHintBadges, overlaps } from '../src/lib/linkHintGeometry.ts';

const discoverPaperLinks = index => projectPaperLinks(index, backendObjects(index), fixtureGeneration);

export function fixture(blocks) {
  const index = { schema_version: 4, text: '', tokens: [], pages: [], objects: { word: [], WORD: [], sentence: [], paragraph: [] }, figures: [], gaps: [] };
  for (const [text, kind = 'body', page = 1] of blocks) {
    const start = index.text.length;
    const y = 60 + index.objects.paragraph.filter(block => index.tokens.find(token => token.start === block.start)?.page === page).length * 40;
    for (const match of text.matchAll(/\S+/gu)) index.tokens.push({ start: start + match.index, end: start + match.index + match[0].length, text: match[0], page, provenance: 'native',
      rects: [{ x_min: 48 + match.index * 6, x_max: 48 + (match.index + match[0].length) * 6, y_min: y, y_max: y + 12 }] });
    index.text += text;
    index.objects.paragraph.push({ start, end: index.text.length, kind });
    index.text += '\n\n';
  }
  for (const number of new Set(index.tokens.map(token => token.page))) {
    const tokens = index.tokens.filter(token => token.page === number);
    index.pages.push({ number, start: tokens[0].start, end: tokens.at(-1).end, width: 612, height: 792, provenance: 'native', confidence: null });
  }
  return index;
}

test('numbered bibliography citations support brackets, parentheses, lists and ranges', () => {
  const index = fixture([
    ['Evidence [1, 3–4] confirms (2). Equation (99) is unrelated.'],
    ['References', 'heading', 2],
    ['[1] Adams. First work. 2020.', 'body', 2], ['[2] Baker. Second work. 2021.', 'body', 2],
    ['[3] Chen. Third work. 2022.', 'body', 3], ['[4] Diaz. Fourth work. 2023.', 'body', 3],
  ]);
  const links = discoverPaperLinks(index);
  assert.deepEqual(links.map(link => link.destination.label.match(/^\[\d\]/)[0]).sort(), ['[1]', '[2]', '[3]', '[4]']);
  assert.ok(links.every(link => link.kind === 'reference' && link.page === 1 && link.rects.length));
  assert.equal(links.find(link => link.label === '(2)').destination.page, 2);
});

test('numbered dot entries and alphanumeric citation keys retain the printed convention', () => {
  const index = fixture([['See [12] and [AB20].'], ['BIBLIOGRAPHY', 'heading', 2], ['12. Jones. A work. 2020.', 'body', 2], ['[AB20] Adams and Baker. Title. 2020.', 'body', 2]]);
  assert.deepEqual(discoverPaperLinks(index).map(link => link.label), ['[12]', '[AB20]']);
});

test('author-year citations include narrative, paired authors, et al, year lists and suffixes', () => {
  const index = fixture([
    ['Smith (2020a,b) agrees with (Jones & Brown, 2018) and García et al., 2019; Smith, 2021.'],
    ['References', 'heading', 2],
    ['Smith, A. (2020a). First study.', 'body', 2], ['Smith, A. (2020b). Second study.', 'body', 2],
    ['Smith, A. (2021). Followup.', 'body', 2], ['Jones, C. and Brown, D. (2018). Joint study.', 'body', 2],
    ['García, E., Patel, R. (2019). Team study.', 'body', 3],
  ]);
  assert.equal(discoverPaperLinks(index).length, 5);
});

test('ambiguous citations and labels are withheld rather than choosing a paper arbitrarily', () => {
  const index = fixture([['Smith (2020) and [3] are ambiguous.'], ['References', 'heading', 2],
    ['[3] Smith, A. (2020). First study.', 'body', 2], ['[3] Smith, B. (2020). Different study.', 'body', 2]]);
  assert.deepEqual(discoverPaperLinks(index), []);
});

test('should withhold reference links when backend output is absent or belongs to a different generation', () => {
  const index = fixture([['See [1] and Fig. 1.'], ['Figure 1: Evidence.', 'caption', 2], ['References', 'heading', 2], ['[1] Smith. Study.', 'body', 2]]);
  const artifact = backendObjects(index);
  assert.deepEqual(projectPaperLinks(index).map(link => link.kind), ['figure']);
  assert.deepEqual(projectPaperLinks(index, artifact, '"stale"').map(link => link.kind), ['figure']);
  assert.deepEqual(projectPaperLinks(index, artifact, fixtureGeneration).map(link => link.kind), ['reference', 'figure']);
});

test('should mask bibliography figure mentions when objects are unavailable while retaining appendix links', () => {
  const index = fixture([['See Fig. 1.'], ['Figure 1: Evidence.', 'caption', 2], ['References', 'heading', 3],
    ['[1] Smith, A. Figure 1 is a title fragment. 2020.', 'body', 3], ['Appendix A', 'heading', 4], ['See Fig. 1 and [1].', 'body', 4]]);
  const artifact = backendObjects(index);
  for (const [objects, generation] of [[null,null],[artifact,'"stale"'],[artifact,fixtureGeneration]]) {
    const links = projectPaperLinks(index, objects, generation);
    assert.deepEqual(links.filter(link => link.kind === 'figure').map(link => link.page), [1,4]);
    assert.ok(links.every(link => link.page !== 3));
  }
});

test('should retain exact repeated occurrence offsets when astral text precedes references in one sentence', () => {
  const index = fixture([['😀 Evidence [1] agrees with [1].'], ['References', 'heading', 2], ['[1] Smith. Study.', 'body', 2]]);
  const artifact = backendObjects(index);
  const links = projectPaperLinks(index, artifact, fixtureGeneration);
  assert.equal(links.length, 2);
  assert.deepEqual(links.map(link => index.text.slice(link.span.start, link.span.end)), ['[1]', '[1]']);
  assert.notDeepEqual(links[0].rects, links[1].rects);
  const mentions = artifact.objects.find(object => object.kind === 'bib_entry').mentions;
  assert.equal(mentions[0].sentence_anchor.start, mentions[1].sentence_anchor.start);
  assert.equal(mentions[0].sentence_anchor.end, mentions[1].sentence_anchor.end);
});

test('captions resolve figures, abbreviated tables, Roman and supplementary identifiers', () => {
  const index = fixture([
    ['See Fig. 2(a), Table IV and Supplementary Fig. 1. Tab. A.1 is absent.'],
    ['Fig. 2. Diagram of the mechanism.', 'caption', 2], ['Table IV: Main results.', 'caption', 3],
    ['Figure S1. Additional observations.', 'caption', 4],
  ]);
  assert.deepEqual(discoverPaperLinks(index).map(link => [link.kind, link.destination.page]), [['figure', 2], ['table', 3], ['figure', 4]]);
});

test('figure lists and ranges expose each destination without treating prose as a caption', () => {
  const index = fixture([['Figs. 1–3 and Tables 1, 2 summarize results. Figure 9 shows a conjecture.'],
    ...[1, 2, 3].map(number => [`Figure ${number}. Result.`, 'caption', 2]),
    ...[1, 2].map(number => [`Table ${number}: Result.`, 'caption', 3])]);
  assert.equal(discoverPaperLinks(index).length, 5);
});

test('backend figure destinations augment captions without duplicate hints', () => {
  const index = fixture([['Fig. 1 shows this.'], ['Figure 1: Image.', 'caption', 2]]);
  const caption = index.objects.paragraph[1];
  index.figures.push({ ...caption, id: 'figure-1', label: 'Figure 1', caption: 'Figure 1: Image.', page: 2, rect: null, confidence: 'candidate', references: [] });
  assert.equal(discoverPaperLinks(index).length, 1);
});

test('Roman labels never fall back to a different numeral, and decimals remain distinct', () => {
  const index = fixture([['See Table IV, Fig. 1.1, Figure 11 and Fig. A.1.'], ['Table I: Another table.', 'caption', 2],
    ['Figure 1.1: Hierarchical number.', 'caption', 2], ['Figure 11: Simple number.', 'caption', 3], ['Figure A1: Appendix figure.', 'caption', 3]]);
  const links = discoverPaperLinks(index);
  assert.equal(links.length, 3);
  assert.deepEqual(links.map(link=>link.destination.page), [2,3,3]);
  assert.ok(links.every(link=>link.kind==='figure'));
});

test('supplementary labels accept an explicit S prefix and unclassified caption punctuation', () => {
  const index = fixture([['See Fig. S1 and Supplementary Figure S1.'], ['Supplementary Fig. S1: Extra evidence.', 'body', 2]]);
  assert.equal(discoverPaperLinks(index).length, 2);
});

test('superscript citation detection requires raised geometry, never a plain prose number', () => {
  const index = fixture([['Evidence 1 and 2 are useful.'], ['References', 'heading', 2], ['1. First study.', 'body', 2], ['2. Second study.', 'body', 2]]);
  const before = index.tokens[0].rects[0], raised = index.tokens[1].rects[0];
  raised.x_min = before.x_max + 1; raised.x_max = raised.x_min + 4; raised.y_min = before.y_min - 3; raised.y_max = before.y_max - 5;
  assert.deepEqual(discoverPaperLinks(index).map(link => link.label), ['1']);
});

test('without bibliography or captions, ordinary numbers and years are not links', () => {
  assert.deepEqual(discoverPaperLinks(fixture([['A list [1], equation (2), and Smith (2020).'], ['1. A numbered step.'], ['Figure 3 shows the idea.']])), []);
});

test('appendix citations after the bibliography still resolve without hinting bibliography entries', () => {
  const index = fixture([['Figure 1: Main result.', 'caption', 1], ['References', 'heading', 2], ['[1] Smith, A. (2020). Study.', 'body', 2],
    ['Appendix A', 'heading', 3], ['We reuse [1] and Fig. 1. Smith (2020) discusses this.', 'body', 3]]);
  const links = discoverPaperLinks(index);
  assert.equal(links.length,3);
  assert.ok(links.every(link=>link.page===3));
});

test('hint codes are deterministic, unique and prefix-free across density boundaries', () => {
  for (const count of [0, 1, 9, 10, 81, 82, 1000]) {
    const codes = hintCodes(count);
    assert.equal(codes.length, count);
    assert.equal(new Set(codes).size, count);
    assert.ok(codes.every(code => code.length === codes[0].length && /^[asdfghjkl]+$/.test(code)));
    assert.deepEqual(codes, hintCodes(count));
  }
});

test('dense grouped hints stay separate at every viewport corner', () => {
  const viewport = {left:10,top:20,right:310,bottom:220};
  for (const [left,top] of [[10,20],[290,20],[10,205],[290,205]]) {
    const boxes = placeHintBadges(Array.from({length:20},()=>({box:{left,top,right:left+10,bottom:top+10}})),36,viewport);
    for (const [at,box] of boxes.entries()) {
      assert.ok(box.left>=viewport.left&&box.right<=viewport.right&&box.top>=viewport.top&&box.bottom<=viewport.bottom);
      assert.ok(boxes.slice(0,at).every(other=>!overlaps(box,other)));
    }
  }
});

test('PDF URI actions only expose ordinary web and email links', () => {
  for (const value of ['javascript:alert(1)', 'file:///etc/passwd', 'data:text/html,x', '/relative', null]) assert.equal(safeLinkUrl(value), null);
  assert.equal(safeLinkUrl('https://example.org/paper'), 'https://example.org/paper');
  assert.equal(safeLinkUrl('mailto:author@example.org'), 'mailto:author@example.org');
});

test('native PDF links resolve named, explicit and URI destinations while ignoring actions', async () => {
  const annotations = [
    { subtype: 'Link', rect: [10, 740, 40, 755], dest: 'bib:weird-style' },
    { subtype: 'Link', rect: [50, 740, 80, 755], dest: [1, { name: 'FitH' }, 650] },
    { subtype: 'Link', rect: [90, 740, 120, 755], url: 'https://example.org/paper' },
    { subtype: 'Link', rect: [130, 740, 150, 755], url: 'javascript:alert(1)' },
    { subtype: 'Link', rect: [160, 740, 170, 755], dest: 'missing' },
  ];
  const viewport = { width: 612, height: 792, convertToViewportRectangle: ([x1,y1,x2,y2]) => [x1,792-y1,x2,792-y2], convertToViewportPoint: (x,y) => [x,792-y] };
  const document = { numPages: 2, getPage: async () => ({ getViewport: () => viewport, getAnnotations: async () => annotations, view: [0,0,612,792] }),
    getDestination: async name => name === 'missing' ? null : [{num: 8, gen: 0}, {name: 'XYZ'}, 48, 600, null], getPageIndex: async () => 1 };
  const links = await nativePageLinks(document, 1, null);
  assert.equal(links.length, 3);
  assert.equal(links[0].destination.page, 2);
  assert.equal(links[0].destination.rect.y_min, 192);
  assert.equal(links[1].destination.rect.y_min, 142);
  assert.equal(links[2].destination.url, 'https://example.org/paper');
});
