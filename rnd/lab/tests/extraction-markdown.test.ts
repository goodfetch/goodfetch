import { test } from 'node:test';
import assert from 'node:assert/strict';
import { markdownElements, scoreMarkdown, matchCounts, f1Of, type ElementType } from '../src/extraction-markdown.ts';
import { loadTruth } from '../src/extraction-campaign.ts';

test('the oracle reads the same structure whatever Markdown syntax a converter chooses', () => {
  const atx = markdownElements('# Hello **big** world\n\n* one\n* two\n\n| A | B |\n| - | - |\n| 1 | 2 |\n\n```rust\nfn x() {}\n```\n');
  const setext = markdownElements('Hello big world\n==========\n\n- one\n- two\n\n<table><thead><tr><th>A</th><th>B</th></tr></thead><tr><td>1</td><td>2</td></tr></table>\n\n    fn x() {}\n');
  assert.deepEqual(atx.heading, ['hello big world']);
  assert.deepEqual(atx.list, ['one', 'two']);
  assert.deepEqual(atx.table, ['a', 'b', '1', '2']);
  assert.deepEqual(atx.code, ['fn x() {}']);
  assert.deepEqual({ ...setext, link: [] }, { ...atx, link: [] });
});

test('nested list items count once each, links keep text and target, escapes and entities are normalized', () => {
  const elements = markdownElements('1. first\n   - inner [doc](https://x.test/a?b=1&amp;c=2)\n2. second \\_x\\_ &amp; y\n');
  assert.deepEqual(elements.list.sort(), ['first', 'inner doc', 'second _x_ & y']);
  assert.deepEqual(elements.link, ['doc -> https://x.test/a?b=1&c=2']);
});

test('scores are multiset micro F1 over headings, lists, tables and code; a page without structure is not scored', () => {
  assert.deepEqual(matchCounts(['a', 'a', 'b'], ['a', 'c']), { matched: 1, predicted: 3, expected: 2 });
  assert.equal(f1Of({ matched: 1, predicted: 2, expected: 2 }), 0.5);
  const score = scoreMarkdown('# Title\n\n- a\n- b\n\nfooter text', '## Title\n\n- a\n- c\n');
  assert.deepEqual(score.structure, { matched: 2, predicted: 3, expected: 3 });
  assert.equal(score.structureF1, 2 / 3);
  assert.equal(scoreMarkdown('# x', 'plain text only').structureF1, null);
  assert.equal(scoreMarkdown('', '# x').structureF1, 0);
});

test('every curated fixture states the structure of its category in its expected Markdown', async () => {
  const truth = await loadTruth({ split: 'test', corpus: 'fixtures/markdown-structure', extractors: ['htmd'], pageTypes: [], blocks: 1, seed: 1 });
  const typeOf: Record<string, ElementType> = { headings: 'heading', lists: 'list', tables: 'table', links: 'link', code: 'code' };
  assert.equal(truth.length, 30);
  for (const page of truth) {
    const elements = markdownElements(page.mainMarkdown!);
    const types: ElementType[] = page.pageType === 'mixed' ? ['heading', 'list', 'table', 'code'] : [typeOf[page.pageType]!];
    assert.ok(types.some(type => elements[type].length > 0), `${page.fileId} ${page.pageType}`);
  }
});
