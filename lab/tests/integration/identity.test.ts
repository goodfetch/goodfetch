import { test } from 'node:test';
import assert from 'node:assert/strict';
import { runIdentityCell, type IdentityCell } from '../../src/identity-campaign.ts';
import { startIdentitySite } from '../../fixtures/identity-site.ts';
import { launchBrowser } from '../../src/core/browser-host.ts';
import { browserPath } from '../../src/environment.ts';
import { CdpConnection } from '../../adapters/cdp-node/cdp.ts';
import { KeyStore, serveDirectory } from '../../src/identity/signature.ts';
import { IdentitySession, productToken } from '../../src/identity/session.ts';

const config = { httpTiers: [], profiles: [], purposes: [], variants: [], repeats: 1, blocks: 1, seed: 11, timeoutMs: 20_000 } as never;
const cell = (httpTier: string, profile: string, purpose: string, variant: string) => runIdentityCell(config, { httpTier, profile, purpose, variant } as IdentityCell, 0) as Promise<{ status: string; violations: Record<string, number>; reasons: string[]; falseSuccess: number }>;

test('a session whose HTTP tier runs in its browser keeps one signed identity across the escalation', async () => {
  const result = await cell('browser-fetch', 'identified', 'search', 'key-rotation');
  assert.equal(result.status, 'verified', result.reasons.join('\n'));
  assert.deepEqual(Object.values(result.violations).filter(Boolean), []);
});

test('the site oracle sees the fingerprint change when an HTTP client hands over to the browser', async () => {
  const result = await cell('impit-chrome', 'standard', 'search', 'coherent');
  assert.equal(result.status, 'failed');
  assert.ok(result.violations.coherence! > 0 && !result.violations.data, result.reasons.join('\n'));
  assert.ok(result.reasons.some(reason => reason.includes('TLS fingerprint changed')));
});

test('preferences are applied per profile and purpose: the identified agent obeys its own robots group, ai-train stops on Content-Signal', async () => {
  assert.equal((await cell('browser-fetch', 'standard', 'search', 'robots-agent')).status, 'verified');
  const refused = await cell('browser-fetch', 'identified', 'ai-train', 'content-signal');
  assert.equal(refused.status, 'verified', refused.reasons.join('\n')); assert.equal(refused.falseSuccess, 0);
});

test('a session keeps its profile, and with every key revoked it sends nothing rather than unsigned requests', async () => {
  const browser = await launchBrowser(browserPath()); const connection = await CdpConnection.connect(browser.wsEndpoint);
  await connection.send('Security.setIgnoreCertificateErrors', { ignore: true });
  const site = await startIdentitySite({ variant: 'coherent', seed: 5 }); const keys = new KeyStore(); const directory = await serveDirectory(keys);
  try {
    const session = await IdentitySession.open({ profile: 'identified', httpTier: 'browser-fetch', purpose: 'search', connection, keys, agentOrigin: directory.origin, timeoutMs: 10_000 });
    assert.throws(() => session.switchProfile('standard'), (error: Error & { code?: string }) => error.code === 'E_PROFILE_LOCKED');
    keys.revoke(keys.active()!.kid);
    await assert.rejects(session.get(site.startUrl()));
    await session.close();
    const verdict = await site.verify([{ label: null, error: 'E_NO_SIGNING_KEY' }], { profile: 'identified', purpose: 'search', productToken });
    assert.equal(site.requests(), 0, 'no request reached the site');
    assert.equal(verdict.violations.signature, 0);
  } finally { await directory.close(); await site.close(); connection.close(); await browser.close(); }
});
