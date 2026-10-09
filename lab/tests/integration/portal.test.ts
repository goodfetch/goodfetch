import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import { runPortalCell, type PortalCell } from '../../src/portal-campaign.ts';

const config = { group: 'routing', variants: [], policies: [], repeats: 2, blocks: 1, seed: 31, timeoutMs: 20_000 } as never;
const cell = (scenario: string, variant: string, policy: string, tier?: string) => runPortalCell(config, { scenario, variant, policy, ...(tier ? { tier } : {}) } as PortalCell, 0, tmpdir()) as Promise<{ status: string; falseSuccess: number; logins: number; browserPagesPerRun: number; reasons: string[]; notes: string[] }>;

test('stale HTML corrected by script: the empty-result heuristic keeps the stale prices, the sampled contract does not', async () => {
  const heuristic = await cell('routing', 'stale-html', 'heuristic'); assert.equal(heuristic.falseSuccess, 2);
  const sampled = await cell('routing', 'stale-html', 'sampled-contract'); assert.equal(sampled.status, 'verified', sampled.reasons.join('\n'));
});

test('API capture learns a cursor chain behind GraphQL and a page-mapped API behind partial hydration, then runs without the browser', async () => {
  for (const variant of ['graphql', 'partial-hydration']) {
    const result = await cell('routing', variant, 'api-capture');
    assert.equal(result.status, 'verified', result.reasons.join('\n'));
    assert.ok(result.notes.some(n => n.includes('learned api')), result.notes.join('\n'));
    assert.equal(result.browserPagesPerRun, 0.5, 'only the first run samples a browser page');
  }
});

test('a session logs in through the browser and resumes from its journal with the exported cookies', async () => {
  const result = await cell('session', 'session-resume', 'contract', 'browser');
  assert.equal(result.status, 'verified', result.reasons.join('\n')); assert.equal(result.logins, 2, 'one login per resumed task, over two runs');
});
