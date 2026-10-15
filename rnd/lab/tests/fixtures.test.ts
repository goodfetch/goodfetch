import { test } from 'node:test';
import assert from 'node:assert/strict';
import { startFixtures } from '../fixtures/server.ts';

test('catalogue exposes a paginated fresh dataset and its oracle refuses omissions', async () => {
  const fixtures = await startFixtures();
  try {
    const run = await fixtures.createRun({ scenario: 'http-catalog', seed: 42, pages: 3 });
    const response = await fetch(run.url);
    assert.equal(response.status, 200);
    const html = await response.text();
    assert.match(html, /P-001/);
    const partial = await fixtures.verify(run.id, { records: [{ id: 'P-001', price: 1201 }] });
    assert.equal(partial.status, 'failed');
    assert.ok(partial.reasons.some(reason => reason.includes('missing')));
  } finally { await fixtures.close(); }
});

test('oracle rejects duplicates, changed values and fabricated form effects', async () => {
  const fixtures = await startFixtures();
  try {
    const run = await fixtures.createRun({ scenario: 'http-catalog', seed: 42, pages: 1 });
    assert.equal((await fixtures.verify(run.id, { records: [{ id: 'P-001', price: 1201 }, { id: 'P-002', price: 1238 }] })).status, 'verified');
    assert.equal((await fixtures.verify(run.id, { records: [{ id: 'P-001', price: 1201 }, { id: 'P-001', price: 1201 }, { id: 'P-002', price: 1238 }] })).status, 'failed');
    const other = await fixtures.createRun({ scenario: 'http-catalog', seed: 43, pages: 1 });
    assert.equal((await fixtures.verify(other.id, { records: [{ id: 'P-001', price: 1201 }, { id: 'P-002', price: 1238 }] })).status, 'failed');
    const form = await fixtures.createRun({ scenario: 'form', seed: 42 });
    await fetch(form.url.replace('/ui','/effect'), { method: 'POST', body: JSON.stringify({ name: 'Goodfetch', region: 'eu', consent: true, trusted: false }) });
    assert.equal((await fixtures.verify(form.id, {})).status, 'failed');
  } finally { await fixtures.close(); }
});
