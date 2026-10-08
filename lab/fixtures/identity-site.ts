// A site for D-identity, served over TLS through the fingerprinting relay. Its preferences change with the variant; its oracle
// records every request (fingerprint, headers, cookie, Web Bot Auth signature checked against the agent directory at arrival)
// and judges the session's outcome against what the variant and the declared purpose require.
import { randomBytes } from 'node:crypto';
import type { Http2ServerRequest, Http2ServerResponse } from 'node:http2';
import { verify, parseSignatureAgentHeader, jwkToKeyID } from 'web-bot-auth';
import { verifierFromJWK } from 'web-bot-auth/crypto';
import { verifySignature } from 'http-message-sig';
import { startTlsRelay, type Fingerprint } from './tls-relay.ts';

export const identityVariants = ['coherent', 'key-rotation', 'key-revoked', 'robots-path', 'robots-agent', 'crawl-delay', 'content-signal', 'tdmrep-file', 'tdmrep-header', 'noai-meta', 'paid-402', 'denied-403', 'challenge-page', 'retry-429', 'isolation'] as const;
export type IdentityVariant = typeof identityVariants[number];
// Variants the session must refuse for at least one purpose; reporting data instead is a false success.
export const identityRefusals: IdentityVariant[] = ['robots-path', 'robots-agent', 'content-signal', 'tdmrep-file', 'tdmrep-header', 'noai-meta', 'paid-402', 'denied-403', 'challenge-page'];
const tdmPurposes = ['ai-train', 'ai-input', 'data-collection'], noaiPurposes = ['ai-train', 'ai-input'];
const directoryPath = '/.well-known/http-message-signatures-directory';

type Signature = { present: boolean; ok: boolean; keyid?: string; error?: string; created?: number; expires?: number; nonce?: string };
type Seen = { path: string; label: string | null; at: number; headers: Record<string, string>; fingerprint: Fingerprint | null; sid: string | null; signature: Promise<Signature> };
export type Expectation = { profile: 'identified' | 'standard'; purpose: string; productToken: string };
export type IdentityVerdict = { status: 'verified' | 'failed'; reasons: string[]; violations: Record<'data' | 'policy' | 'coherence' | 'signature' | 'cookie' | 'isolation', number> };

function expectedOutcome(variant: IdentityVariant, { profile, purpose }: Expectation): { refusal: string | null; catalogBeforeRefusal: boolean } {
  const refuse = (code: string, catalogBeforeRefusal = false) => ({ refusal: code, catalogBeforeRefusal });
  if (variant === 'robots-path') return refuse('E_POLICY_ROBOTS', true);
  if (variant === 'robots-agent' && profile === 'identified') return refuse('E_POLICY_ROBOTS');
  if (variant === 'content-signal' && purpose === 'ai-train') return refuse('E_POLICY_CONTENT_SIGNAL');
  if (variant === 'tdmrep-file' && tdmPurposes.includes(purpose)) return refuse('E_POLICY_TDM');
  if (variant === 'tdmrep-header' && tdmPurposes.includes(purpose)) return refuse('E_POLICY_TDM', true);
  if (variant === 'noai-meta' && noaiPurposes.includes(purpose)) return refuse('E_POLICY_NOAI', true);
  if (variant === 'paid-402') return refuse('E_PAYMENT_REQUIRED', true);
  if (variant === 'denied-403') return refuse('E_ACCESS_DENIED', true);
  if (variant === 'challenge-page') return refuse('E_CHALLENGE', true);
  return { refusal: null, catalogBeforeRefusal: true };
}

const robotsFor: Partial<Record<IdentityVariant, string>> = {
  'robots-path': 'User-agent: *\nDisallow: /private/\n', 'robots-agent': 'User-agent: AutomanLab\nDisallow: /\n\nUser-agent: *\nAllow: /\n',
  'crawl-delay': 'User-agent: *\nCrawl-delay: 1\nAllow: /\n', 'content-signal': 'User-agent: *\nContent-Signal: search=yes, ai-input=yes, ai-train=no\nAllow: /\n'
};

// The directory of the agent named by Signature-Agent, fetched when the request arrives, with a proof of possession for every key.
async function directoryKeys(uri: string) {
  const url = uri.endsWith(directoryPath) ? uri : uri.replace(/\/$/, '') + directoryPath;
  const response = await fetch(url); if (response.status !== 200) throw new Error(`directory HTTP ${response.status}`);
  if (!response.headers.get('content-type')?.startsWith('application/http-message-signatures-directory+json')) throw new Error('directory content type');
  const { keys } = await response.json() as { keys: JsonWebKey[] };
  const fields = [...response.headers].map(([name, value]) => ({ name, value }));
  const labels = [...(response.headers.get('signature-input') ?? '').matchAll(/(?:^|,\s*)([a-z][a-z0-9_-]*)=\(/g)].map(m => m[1]!);
  const proven = new Set<string>();
  for (const label of labels) {
    const verified = await verifySignature({ kind: 'response', status: 200, fields, request: { kind: 'request', method: 'GET', targetUri: url, fields: [] } }, {
      label, policy: { algorithms: ['ed25519'], requiredComponents: [], requiredParameters: ['created', 'expires', 'keyid', 'tag'], maxAge: 300, now: Math.floor(Date.now() / 1000) },
      resolveVerifier: async candidate => { if (candidate.parameters.tag !== 'http-message-signatures-directory') throw new Error('directory signature tag'); const jwk = keys.find(k => (k as { kid?: string }).kid === candidate.parameters.keyid); if (!jwk) throw new Error('directory signed by an unlisted key'); return verifierFromJWK(jwk); }
    });
    proven.add(String(verified.parameters.keyid));
  }
  for (const key of keys) {
    const kid = await jwkToKeyID(key, data => crypto.subtle.digest('SHA-256', data), bytes => Buffer.from(bytes).toString('base64url'));
    if ((key as { kid?: string }).kid !== kid) throw new Error('directory key id is not its thumbprint');
    if (!proven.has(kid)) throw new Error(`no proof of possession for ${kid}`);
  }
  return keys;
}

async function checkSignature(url: string, headers: Record<string, string>): Promise<Signature> {
  if (!headers.signature && !headers['signature-input'] && !headers['signature-agent']) return { present: false, ok: false };
  try {
    const agent = parseSignatureAgentHeader(headers['signature-agent'] ?? '').entries[0]; if (!agent) throw new Error('no Signature-Agent');
    const keys = await directoryKeys(agent.uri);
    const fields = ['signature', 'signature-input', 'signature-agent'].filter(name => headers[name]).map(name => ({ name, value: headers[name]! }));
    const verified = await verify({ kind: 'request', method: 'GET', targetUri: url, fields }, {
      maxAge: 300, clockSkew: 5, algorithms: ['ed25519'],
      resolver: async candidate => { const jwk = keys.find(k => (k as { kid?: string }).kid === candidate.keyid); if (!jwk) throw new Error('key not in the agent directory'); return verifierFromJWK(jwk); }
    });
    return { present: true, ok: true, keyid: verified.keyid, created: verified.created.getTime(), expires: verified.expires.getTime(), nonce: verified.nonce };
  } catch (error) { return { present: true, ok: false, error: error instanceof Error ? error.message : String(error) }; }
}

export async function startIdentitySite({ variant, seed }: { variant: IdentityVariant; seed: number }) {
  const records = Array.from({ length: 6 }, (_, i) => ({ id: `I-${String(i + 1).padStart(3, '0')}`, price: 2101 + i * 41 + seed % 89 }));
  const seen: Seen[] = []; const sids = new Map<string, string>(); const issuedTo = new Map<string, string>(); let limited: number | null = null;
  const relay = await startTlsRelay((req, res) => { void handle(req, res); });
  const send = (res: Http2ServerResponse, status: number, body: string, headers: Record<string, string> = {}) => { res.writeHead(status, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store', ...headers }); res.end(body); };

  async function handle(req: Http2ServerRequest, res: Http2ServerResponse) {
    const authority = req.headers[':authority'] ?? req.headers.host ?? ''; const url = new URL(req.url ?? '/', `https://${authority}`);
    const headers: Record<string, string> = {}; for (let i = 0; i < req.rawHeaders.length; i += 2) if (!req.rawHeaders[i]!.startsWith(':')) headers[req.rawHeaders[i]!.toLowerCase()] = req.rawHeaders[i + 1]!;
    const sid = headers.cookie?.match(/(?:^|;\s*)sid=([0-9a-f]+)/)?.[1] ?? null; const label = url.searchParams.get('s');
    seen.push({ path: url.pathname + url.search, label, at: performance.now(), headers, fingerprint: relay.fingerprint(req), sid, signature: checkSignature(url.href, headers) });
    if (url.pathname === '/robots.txt') return robotsFor[variant] ? send(res, 200, robotsFor[variant]!, { 'content-type': 'text/plain' }) : send(res, 404, 'not found', { 'content-type': 'text/plain' });
    if (url.pathname === '/.well-known/tdmrep.json') return variant === 'tdmrep-file' ? send(res, 200, JSON.stringify([{ location: '/catalog*', 'tdm-reservation': 1, 'tdm-policy': `${url.origin}/tdm-policy.json` }]), { 'content-type': 'application/json' }) : send(res, 404, 'not found');
    const p = Number(url.searchParams.get('p') ?? 0); const suffix = label ? `&s=${label}` : '';
    // The session cookie is issued on the first catalog request of each witness label.
    const cookie: Record<string, string> = {};
    if ((url.pathname === '/catalog' || url.pathname === '/private/catalog') && !sid && !(label && sids.has(label))) { const fresh = randomBytes(8).toString('hex'); sids.set(label ?? '', fresh); issuedTo.set(fresh, label ?? ''); cookie['set-cookie'] = `sid=${fresh}; Path=/; HttpOnly; Secure; SameSite=Lax`; }
    if (url.pathname === '/data') return send(res, 200, records.slice(p * 2, p * 2 + 2).map(r => `<article data-id="${r.id}" data-price="${r.price}"><h2>${r.id}</h2></article>`).join(''));
    if (url.pathname !== '/catalog' && url.pathname !== '/private/catalog') return send(res, 404, 'unknown route');
    if (!Number.isInteger(p) || p < 0 || p > 2) return send(res, 404, 'bad page');
    if (p === 1) {
      if (variant === 'paid-402') return send(res, 402, 'payment required', cookie);
      if (variant === 'denied-403') return send(res, 403, 'forbidden', cookie);
      if (variant === 'challenge-page') return send(res, 200, '<!doctype html><html><head><title>Just a moment...</title></head><body><div id="challenge">Checking your browser before accessing this site.</div></body></html>', cookie);
      if (variant === 'retry-429' && limited === null) { limited = performance.now(); return send(res, 429, 'slow down', { ...cookie, 'retry-after': '1', 'content-type': 'text/plain' }); }
    }
    const articles = p < 2 ? records.slice(p * 2, p * 2 + 2).map(r => `<article data-id="${r.id}" data-price="${r.price}"><h2>${r.id}</h2><span>${r.price}</span></article>`).join('') : '';
    const next = p < 2 ? `<a class="next" href="${variant === 'robots-path' && p === 1 ? '/private' : ''}/catalog?p=${p + 1}${suffix}">Next</a>` : '';
    const meta = variant === 'noai-meta' && p === 1 ? '<meta name="robots" content="noai, noimageai">' : '';
    const extra: Record<string, string> = variant === 'tdmrep-header' && p === 1 ? { 'tdm-reservation': '1' } : {};
    // The last page renders its records with script, so a session must escalate to its browser for it.
    const script = p === 2 ? `<script>const prior=localStorage.getItem('witness');localStorage.setItem('witness',${JSON.stringify(label ?? '')});fetch('/data?p=2${suffix}&prior='+encodeURIComponent(prior??'')).then(r=>r.text()).then(html=>{const main=document.querySelector('#catalog');main.innerHTML=html;main.dataset.ready='true'})</script>` : '';
    return send(res, 200, `<!doctype html><html><head>${meta}</head><body><main id="catalog" ${p === 2 ? 'data-needs-js data-ready="false"' : 'data-ready="true"'}>${articles}${next}</main>${script}</body></html>`, { ...cookie, ...extra });
  }

  return {
    origin: relay.origin,
    startUrl: (label?: string) => `${relay.origin}/catalog?p=0${label ? `&s=${label}` : ''}`,
    async verify(outputs: { label: string | null; records?: { id: string; price: number }[]; error?: string }[], expectation: Expectation): Promise<IdentityVerdict> {
      const reasons: string[] = []; const violations: IdentityVerdict['violations'] = { data: 0, policy: 0, coherence: 0, signature: 0, cookie: 0, isolation: 0 };
      const flag = (kind: keyof IdentityVerdict['violations'], reason: string) => { violations[kind]++; reasons.push(reason); };
      const expected = expectedOutcome(variant, expectation);
      const catalog = seen.filter(s => s.path.startsWith('/catalog') || s.path.startsWith('/private/'));
      // Outcome: the exact records, or the expected refusal with no data reported and no forbidden request made.
      for (const output of outputs) {
        if (expected.refusal) {
          if (output.error === undefined) flag('data', `false success: ${output.records?.length ?? 0} records reported where ${expected.refusal} was required`);
          else if (!output.error.startsWith(expected.refusal)) flag('policy', `refused with ${output.error.slice(0, 60)}, expected ${expected.refusal}`);
        } else if (output.error !== undefined) flag('data', `failed: ${output.error.slice(0, 100)}`);
        else {
          const got = new Map((output.records ?? []).map(r => [r.id, r.price]));
          if (got.size !== records.length || (output.records ?? []).length !== records.length || records.some(r => got.get(r.id) !== r.price)) flag('data', 'missing, duplicated or wrong records');
        }
      }
      if (seen.some(s => s.path.startsWith('/private/'))) flag('policy', 'fetched a page disallowed by robots.txt');
      if (expected.refusal && !expected.catalogBeforeRefusal && catalog.length) flag('policy', `${catalog.length} catalog request(s) made although the site-wide preference forbids the purpose`);
      if (variant === 'crawl-delay') { const docs = catalog.filter(s => s.headers['sec-fetch-dest'] !== 'empty'); for (let i = 1; i < docs.length; i++) if (docs[i]!.at - docs[i - 1]!.at < 980) { flag('policy', `crawl-delay of 1 s not honoured (${Math.round(docs[i]!.at - docs[i - 1]!.at)} ms)`); break; } }
      if (variant === 'retry-429') { const retry = catalog.find(s => s.path.startsWith('/catalog?p=1') && s.at > (limited ?? Infinity)); if (limited !== null && retry && retry.at - limited < 950) flag('policy', `retried ${Math.round(retry.at - limited)} ms after a 429 asking for 1 s`); }
      // Identity: per witness label (one session each), every request presents the same client.
      // With several sessions, a request carrying neither a witness label nor a cookie (their robots.txt) cannot be attributed and is left out.
      const several = outputs.length > 1; const byLabel = new Map<string, Seen[]>();
      for (const s of seen) { const key = s.label ?? (s.sid ? issuedTo.get(s.sid) ?? '?' : ''); if (several && !key) continue; byLabel.set(key, [...(byLabel.get(key) ?? []), s]); }
      for (const [label, group] of byLabel) {
        const tag = label ? `session ${label}: ` : '';
        for (const header of ['user-agent', 'accept-language', 'sec-ch-ua', 'sec-ch-ua-mobile', 'sec-ch-ua-platform']) { const values = new Set(group.map(s => s.headers[header] ?? '∅')); if (values.size > 1) flag('coherence', `${tag}${header} changed within the session: ${[...values].map(v => v.slice(0, 50)).join(' / ')}`); }
        const tls = new Set(group.map(s => s.fingerprint?.ja4 ?? '∅')); if (tls.size > 1) flag('coherence', `${tag}TLS fingerprint changed within the session: ${[...tls].join(' / ')}`);
        const h2 = new Set(group.map(s => `${s.fingerprint?.http}|${s.fingerprint?.h2}`)); if (h2.size > 1) flag('coherence', `${tag}HTTP version or HTTP/2 fingerprint changed within the session: ${[...h2].join(' / ')}`);
        const ua = group[0]?.headers['user-agent'] ?? '';
        if (expectation.profile === 'identified' && !ua.includes(`${expectation.productToken}/`)) flag('signature', `${tag}identified profile without its product token in User-Agent`);
        if (expectation.profile === 'standard' && ua.includes(`${expectation.productToken}/`)) flag('signature', `${tag}standard profile announces the product token`);
      }
      // Cookies: after its first catalog response, each session presents its own sid on every request, never another one.
      for (const s of seen) if (s.sid && !issuedTo.has(s.sid)) flag('cookie', `unknown session cookie on ${s.path}`);
      for (const [label, sid] of sids) {
        const own = seen.filter(s => (s.label ?? '') === label); const first = own.findIndex(s => s.path.startsWith('/catalog') || s.path.startsWith('/private/'));
        const later = own.slice(first + 1).filter(s => s.sid !== sid);
        if (later.length) flag(later.some(s => s.sid && s.sid !== sid) ? 'isolation' : 'cookie', `${label ? `session ${label}: ` : ''}${later.length} request(s) after the first without the session cookie (${later[0]!.path})`);
      }
      for (const s of seen.filter(s => s.path.startsWith('/data'))) { const prior = new URL(s.path, 'https://x').searchParams.get('prior') ?? ''; if (prior && prior !== (s.label ?? '')) flag('isolation', `storage of session ${prior} visible to session ${s.label}`); }
      // Web Bot Auth: identified requests are all signed, valid against the directory at arrival, short-lived and never replayed; standard ones never signed.
      const signatures = await Promise.all(seen.map(s => s.signature)); const nonces = new Set<string>(); let invalid = 0, missing = 0;
      for (const [i, signature] of signatures.entries()) {
        if (expectation.profile === 'standard') { if (signature.present) flag('signature', `standard profile signed ${seen[i]!.path}`); continue; }
        if (!signature.present) { missing++; continue; }
        if (!signature.ok) { invalid++; if (invalid === 1) reasons.push(`signature refused on ${seen[i]!.path}: ${signature.error}`); continue; }
        if (signature.expires! - signature.created! > 300_000 || Math.abs(signature.created! - Date.now()) > 120_000) flag('signature', `signature validity window too long or skewed on ${seen[i]!.path}`);
        if (signature.nonce) { if (nonces.has(signature.nonce)) flag('signature', 'nonce reused'); nonces.add(signature.nonce); }
      }
      if (missing) flag('signature', `${missing} request(s) of the identified profile not signed`);
      if (invalid) violations.signature++;
      if (expectation.profile === 'identified' && (variant === 'key-rotation' || variant === 'key-revoked')) {
        const order = signatures.filter(s => s.ok).map(s => s.keyid!); const distinct = [...new Set(order)];
        if (distinct.length < 2) flag('signature', `expected a key change during the session, saw ${distinct.length} key(s)`);
        else if (order.lastIndexOf(distinct[0]!) > order.indexOf(distinct[1]!)) flag('signature', 'the previous key signed again after the change');
      }
      return { status: reasons.length ? 'failed' : 'verified', reasons, violations };
    },
    requests: () => seen.length,
    close: () => relay.close()
  };
}
