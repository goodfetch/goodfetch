import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import { runPortalCell, type PortalCell } from '../src/portal-campaign.ts';
import { startPortalSite } from '../fixtures/portal-site.ts';

const config = { group: 'session', variants: [], policies: [], repeats: 1, blocks: 1, seed: 21, timeoutMs: 10_000, maxRequests: 300 } as never;
const cell = (scenario: string, variant: string, policy: string, tier?: string) => runPortalCell(config, { scenario, variant, policy, ...(tier ? { tier } : {}) } as PortalCell, 0, tmpdir()) as Promise<{ status: string; falseSuccess: number; logins: number; refreshes: number; reasons: string[]; notes: string[] }>;

test('a portal that answers an expired session with a 200 login page fools a status-only client into a false success', async () => {
  const fooled = await cell('session', 'session-expiry', 'status-only', 'http');
  assert.equal(fooled.status, 'failed'); assert.equal(fooled.falseSuccess, 1);
  const judged = await cell('session', 'session-expiry', 'contract', 'http');
  assert.equal(judged.status, 'verified', judged.reasons.join('\n')); assert.equal(judged.logins, 2);
});

test('a resumed run reuses the journaled session, a revoked one logs in again, and the refresh token avoids a second login', async () => {
  assert.equal((await cell('session', 'session-resume', 'contract', 'http')).logins, 1);
  assert.equal((await cell('session', 'session-resume', 'relogin-each-run', 'http')).logins, 2);
  const revoked = await cell('session', 'resume-revoked', 'contract', 'http'); assert.equal(revoked.status, 'verified'); assert.equal(revoked.logins, 2);
  const refreshed = await cell('session', 'session-refresh', 'contract', 'http'); assert.equal(refreshed.logins, 1); assert.ok(refreshed.refreshes >= 1);
});

test('offset pagination over a table that changes while it is paged loses rows and still reports a complete crawl', async () => {
  const offset = await cell('frontier', 'large-table', 'naive'); assert.equal(offset.falseSuccess, 1, offset.reasons.join('\n'));
  const cursor = await cell('frontier', 'large-table', 'canonical'); assert.equal(cursor.status, 'verified', cursor.reasons.join('\n'));
});

test('the canonical frontier ends a crawl through session ids, sort permutations and an endless calendar; the naive one runs out of budget', async () => {
  const canonical = await cell('frontier', 'crawler-trap', 'canonical');
  assert.equal(canonical.status, 'verified', canonical.reasons.join('\n')); assert.ok(canonical.notes.some(n => n.includes('/events: three empty pages')));
  const naive = await cell('frontier', 'crawler-trap', 'naive'); assert.equal(naive.falseSuccess, 0); assert.ok(naive.reasons.some(r => r.includes('E_BUDGET')));
});

test('the portal oracle requires the rows that lived through the whole crawl, once each', async () => {
  const site = await startPortalSite({ variant: 'large-table', seed: 3 });
  try {
    const page = await (await fetch(site.startUrl)).text(); const first = [...page.matchAll(/data-id="([^"]+)" data-price="(\d+)"/g)].map(m => ({ id: m[1]!, price: Number(m[2]) }));
    assert.equal(site.verify([...first, first[0]!]).duplicates, 1);
    assert.ok(site.verify(first).missing > 1000);
  } finally { await site.close(); }
});
