// Identity lane (D-identity): a crawl through one identity session, over HTTP until a page needs script, then in the session's
// browser. Cells cross the HTTP tier, the access profile, the declared purpose and the site variant; the site's oracle judges
// the outcome, the identity presented by every request and the Web Bot Auth signatures.
import { startIdentitySite, identityVariants, type IdentityVariant, type IdentityVerdict } from '../fixtures/identity-site.ts';
import { launchBrowser } from './core/browser-host.ts';
import { browserPath } from './environment.ts';
import { CdpConnection } from '../adapters/cdp-node/cdp.ts';
import { KeyStore, serveDirectory } from './identity/signature.ts';
import { IdentitySession, productToken, type HttpTier, type Profile } from './identity/session.ts';
import { StopError, type Purpose } from './identity/policy.ts';
import { quantiles } from './statistics.ts';
import { startProxyPool } from '../fixtures/proxy-pool.ts';
import { ProxyPool } from './identity/egress.ts';

export type IdentityConfig = { httpTiers: HttpTier[]; profiles: Profile[]; purposes: Purpose[]; variants: IdentityVariant[]; repeats: number; blocks: number; seed: number; timeoutMs: number };
export type IdentityCell = { httpTier: HttpTier; profile: Profile; purpose: Purpose; variant: IdentityVariant };
export const httpTiers: HttpTier[] = ['node-https', 'impit-chrome', 'browser-fetch'];
export { identityVariants };

// Key changes only exist for a signing profile.
// Each variant judges the contract it tests; every violation is still recorded in the raw cell.
const concernOf = (variant: IdentityVariant): (keyof IdentityVerdict['violations'] | 'proxy')[] =>
  variant === 'coherent' || variant.startsWith('key-') ? ['data', 'policy', 'coherence', 'signature', 'cookie', 'isolation'] : variant === 'isolation' ? ['data', 'cookie', 'isolation'] : variant.startsWith('proxy-') ? ['data', 'policy', 'cookie', 'signature', 'proxy'] : ['data', 'policy'];

export const identityCombinations = (config: IdentityConfig): IdentityCell[] => config.httpTiers.flatMap(httpTier => config.profiles.flatMap(profile => config.purposes.flatMap(purpose =>
  config.variants.filter(variant => (profile === 'identified' || !variant.startsWith('key-')) && (httpTier === 'browser-fetch' || !variant.startsWith('proxy-'))).map(variant => ({ httpTier, profile, purpose, variant })))));

const parseRecords = (html: string) => [...html.matchAll(/<article data-id="([^"]+)" data-price="(\d+)"/g)].map(m => ({ id: m[1]!, price: Number(m[2]) }));

// The task: follow the catalog; a page marked as needing script is opened in the session's browser and read once rendered.
async function crawl(session: IdentitySession, start: string, afterPage: (page: number) => void) {
  const records: { id: string; price: number }[] = []; let url: string | null = start;
  for (let page = 0; url && page < 5; page++) {
    let html: string = (await session.get(url)).body;
    if (/data-needs-js/.test(html)) {
      const tab = await session.browse(url);
      try { html = await tab.evaluate<string>(`new Promise((resolve,reject)=>{const t0=performance.now();const tick=()=>{const main=document.querySelector('#catalog');if(main?.dataset.ready==='true')resolve(document.documentElement.outerHTML);else if(performance.now()-t0>5000)reject(new Error('page did not render'));else setTimeout(tick,10)};tick()})`); }
      finally { await tab.close(); }
    }
    records.push(...parseRecords(html)); afterPage(page);
    const next: RegExpMatchArray | null = html.match(/class="next" href="([^"]+)"/); url = next ? new URL(next[1]!, url).href : null;
  }
  return records;
}

export async function runIdentityCell(config: IdentityConfig, cell: IdentityCell, block: number): Promise<Record<string, unknown>> {
  const browser = await launchBrowser(browserPath()); const connection = await CdpConnection.connect(browser.wsEndpoint);
  await connection.send('Security.setIgnoreCertificateErrors', { ignore: true });
  const taskMs: number[] = []; const reasons: string[] = []; let verified = 0, falseSuccess = 0; const violations: Record<string, number> = {}; let signatures = 0, retries = 0, journaled = 0, proxyBytes = 0;
  try {
    for (let repeat = 0; repeat < config.repeats; repeat++) {
      const site = await startIdentitySite({ variant: cell.variant, seed: config.seed + block * 100 + repeat });
      const keys = new KeyStore(); const directory = await serveDirectory(keys);
      const proxies = cell.variant.startsWith('proxy-') ? await startProxyPool(3) : null; const downAt = new Map<number, number>();
      const pool = proxies ? new ProxyPool(proxies.urls.map(url => ({ url, user: proxies.credentials.user, password: proxies.credentials.password }))) : undefined;
      try {
        const labels = cell.variant === 'isolation' ? ['A', 'B'] : [null];
        const sessions = await Promise.all(labels.map(() => IdentitySession.open({ profile: cell.profile, httpTier: cell.httpTier, purpose: cell.purpose, connection, keys, agentOrigin: directory.origin, contactUrl: 'https://automan.invalid/bot', timeoutMs: config.timeoutMs, proxyPool: pool })));
        const start = performance.now();
        const outputs = await Promise.all(sessions.map(async (session, index) => {
          const label = labels[index]!;
          // Mid-session key events, after the first page: a planned rotation, or a revocation followed by a new key.
          // Proxy events: the session's current upstream goes down, or every upstream does.
          const afterPage = (page: number) => {
            if (page !== 0 || index !== 0) return; if (cell.variant === 'key-rotation') keys.rotate(); if (cell.variant === 'key-revoked') { keys.revoke(keys.active()!.kid); keys.rotate(); }
            if (proxies && cell.variant !== 'proxy-sticky') { const current = proxies.tunnels.filter(t => t.session === session.stickyId).at(-1)?.proxy; for (const target of cell.variant === 'proxy-exhausted' ? [0, 1, 2] : current === undefined ? [] : [current]) { downAt.set(target, performance.now()); proxies.takeDown(target); } }
          };
          try { return { label, records: await crawl(session, site.startUrl(label ?? undefined), afterPage) }; }
          catch (error) { return { label, error: error instanceof StopError ? error.message : `E_TASK: ${error instanceof Error ? error.message : String(error)}` }; }
        }));
        const elapsed = performance.now() - start;
        for (const session of sessions) { signatures += session.signatures; retries += session.retries; journaled += session.journal.length; await session.close(); }
        const verdict: IdentityVerdict & { violations: Record<string, number> } = await site.verify(outputs, { profile: cell.profile, purpose: cell.purpose, productToken });
        if (proxies) { const proxyReasons = proxies.judge(cell.variant, sessions.map(s => s.stickyId), downAt); verdict.violations.proxy = proxyReasons.length; verdict.reasons.push(...proxyReasons); proxyBytes += proxies.tunnels.reduce((sum, t) => sum + t.bytes, 0); }
        for (const [kind, count] of Object.entries(verdict.violations)) violations[kind] = (violations[kind] ?? 0) + count;
        if (!verdict.violations.data && !verdict.violations.policy) taskMs.push(elapsed);
        if (concernOf(cell.variant).every(kind => !verdict.violations[kind])) verified++;
        if (outputs.some(output => output.error === undefined) && verdict.violations.data) falseSuccess++;
        reasons.push(...verdict.reasons.slice(0, 4).map(r => `r${repeat}: ${r}`));
      } finally { await directory.close(); await site.close(); await proxies?.close(); }
    }
  } finally { connection.close(); await browser.close(); }
  return { ...cell, block, status: verified === config.repeats ? 'verified' : 'failed', repeats: config.repeats, verifiedRuns: verified, falseSuccess, violations, signatures, retries, journaled, proxyBytes,
    concern: concernOf(cell.variant), taskP50MsVerified: quantiles(taskMs).p50, reasons: [...new Set(reasons)].slice(0, 12) };
}
