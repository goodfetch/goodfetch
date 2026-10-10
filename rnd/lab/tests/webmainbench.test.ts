import { test } from 'node:test';
import assert from 'node:assert/strict';
import { annotationMarkers, stripAnnotationMarkers } from '../src/webmainbench.ts';

test('WebMainBench annotation markers are stripped, the rest of the page is kept', () => {
  const html = '<head><style data-anno-uid="u0" id="cc-extraStyle" name="cc">.mark-selected { outline: 0 }</style></head>'
    + '<body><h1 cc-select="true" class="mark-selected" data-anno-uid="u1">Title</h1>'
    + '<p cc-select="true" class="lead mark-selected intro" data-anno-uid="u2">Text</p><div class="nav" data-anno-uid="u3">Menu</div></body>';
  const stripped = stripAnnotationMarkers(html);
  assert.equal(stripped, '<head></head><body><h1 data-anno-uid="u1">Title</h1>'
    + '<p class="lead intro" data-anno-uid="u2">Text</p><div class="nav" data-anno-uid="u3">Menu</div></body>');
  assert.deepEqual(annotationMarkers(stripped), []);
  assert.deepEqual(annotationMarkers(html), ['cc-select', 'mark-selected', 'cc-extraStyle']);
});
