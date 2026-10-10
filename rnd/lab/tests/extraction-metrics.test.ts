import { test } from 'node:test';
import assert from 'node:assert/strict';
import { wordF1, tokenize, snippetRate } from '../src/extraction-campaign.ts';

test('word F1 counts each reference word once, case-insensitively, like WCXB evaluate.py', () => {
  assert.deepEqual(tokenize("Café, l'été 2026_v2!"), ['café', 'l', 'été', '2026_v2']);
  const score = wordF1('the cat sat on the mat menu', 'The cat sat on the mat');
  assert.equal(score.recall, 1);
  assert.equal(score.precision, 6 / 7);
  assert.deepEqual(wordF1('', ''), { precision: 1, recall: 1, f1: 1 });
  assert.equal(wordF1('anything', '').f1, 0);
});

test('snippet rates are case-insensitive substring shares, and an empty list counts as satisfied', () => {
  assert.equal(snippetRate('Main Content here', ['main content', 'missing part']), 0.5);
  assert.equal(snippetRate('text', []), 1);
});
