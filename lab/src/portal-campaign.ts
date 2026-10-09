// Portal lane: the contracts of long authenticated sessions (D-session-model), of choosing HTTP, the browser or a discovered API per
// route (D-adaptive-routing, architecture §5.3) and of bounding a crawl through traps and changing tables (D-frontier, §3.1).
// Every cell runs a policy against a fresh portal site whose oracle knows the true records; a run that reports a complete
// result the oracle refutes is a false success.
import { writeFile, readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { startPortalSite, portalUser, sessionVariants, routingVariants, frontierVariants, type PortalRecord, type PortalSite, type PortalVariant } from '../fixtures/portal-site.ts';
import { HttpTier, BrowserTier, parseRecords, type Tier, type Document, type Captured } from './portal/tiers.ts';
import { launchBrowser } from './core/browser-host.ts';
import { browserPath } from './environment.ts';
import { CdpConnection } from '../adapters/cdp-node/cdp.ts';
import { quantiles } from './statistics.ts';

export type PortalGroup = 'session' | 'routing' | 'frontier';
export const portalPolicies = { session: ['status-only', 'contract', 'relogin-each-run'], routing: ['browser-only', 'http-only', 'heuristic', 'sampled-contract', 'api-capture'], frontier: ['naive', 'canonical'] } as const;
export const portalVariants = { session: sessionVariants, routing: routingVariants, frontier: frontierVariants } as const;
export type PortalConfig = { group: PortalGroup; variants: PortalVariant[]; policies: string[]; tiers?: ('http' | 'browser')[]; repeats: number; blocks: number; seed: number; timeoutMs: number; maxRequests?: number };
export type PortalCell = { scenario: PortalGroup; variant: PortalVariant; policy: string; tier?: 'http' | 'browser' };
type RunOutcome = { records: PortalRecord[]; complete: boolean; error?: string; notes?: string[] };
class PortalError extends Error { readonly code: string; constructor(code: string, message: string) { super(message); this.code = code; } }

export const portalCombinations = (config: PortalConfig): PortalCell[] => config.variants.flatMap(variant => config.policies.flatMap<PortalCell>(policy =>
  config.group === 'session' ? (config.tiers ?? ['http']).map(tier => ({ scenario: config.group, variant, policy, tier })) : [{ scenario: config.group, variant, policy }]));

const nextLink = (doc: Document) => { const href = doc.body.match(/<a class="next" href="([^"]+)"/)?.[1]; return href ? new URL(href.replaceAll('&amp;', '&'), doc.url).href : null; };
const declaredTotal = (html: string) => { const m = html.match(/data-total="(\d+)"/); return m ? Number(m[1]) : null; };
const sameRecords = (a: PortalRecord[], b: PortalRecord[]) => { const ma = new Map(a.map(r => [r.id, r.price])); return ma.size === new Map(b.map(r => [r.id, r.price])).size && b.every(r => ma.get(r.id) === r.price); };
const merge = (into: Map<string, number>, records: PortalRecord[]) => { for (const r of records) into.set(r.id, r.price); };

// ---- Sessions ---------------------------------------------------------------------------------------------------------------
// The recipe a portal task declares: where to log in, the refresh endpoint and its cookie, the credentials reference.
const recipe = { loginPath: '/login', refreshPath: '/session/refresh', refreshCookie: 'rt', credentials: portalUser };
// status-only trusts the status and the URL; contract also requires the page to be the authenticated listing it expects.
function classify(policy: string, doc: Document): 'ok' | 'auth-lost' | 'unknown' {
  const atLogin = new URL(doc.url).pathname === recipe.loginPath;
  if (policy === 'status-only') return doc.status === 401 || doc.status === 403 || atLogin ? 'auth-lost' : 'ok';
  if (atLogin || /<form id="login"/.test(doc.body)) return 'auth-lost';
  return /<main id="records" data-auth=/.test(doc.body) ? 'ok' : 'unknown';
}
type Journal = { next: string | null; records: PortalRecord[] };
async function sessionRun(tier: Tier, policy: string, origin: string, journal: Journal, maxPages = Infinity) {
  if (policy === 'relogin-each-run' && !(await tier.login(origin + recipe.loginPath, recipe.credentials))) throw new PortalError('E_AUTH', 'login refused');
  let pages = 0;
  while (journal.next && pages < maxPages) {
    let doc: Document; let recoveries = 0;
    for (;;) {
      doc = await tier.get(journal.next);
      const state = classify(policy, doc);
      if (state === 'ok') break;
      if (state === 'unknown') throw new PortalError('E_CONTRACT', `unexpected page at ${new URL(doc.url).pathname} (HTTP ${doc.status})`);
      if (recoveries++ >= 2) throw new PortalError('E_AUTH', 'session could not be restored');
      if (await tier.hasCookie(recipe.refreshCookie) && await tier.post(origin + recipe.refreshPath) === 204) continue;
      if (!(await tier.login(origin + recipe.loginPath, recipe.credentials))) throw new PortalError('E_AUTH', 'login refused');
    }
    journal.records.push(...parseRecords(doc.body)); pages++; journal.next = nextLink(doc);
  }
}

async function runSessionTask(cell: PortalCell, site: PortalSite, connection: CdpConnection | null, timeoutMs: number, directory: string, label: string): Promise<RunOutcome> {
  const open = async () => cell.tier === 'browser' ? BrowserTier.open(connection!, timeoutMs) : new HttpTier(timeoutMs);
  const journal: Journal = { next: site.startUrl, records: [] }; const notes: string[] = [];
  const resumed = cell.variant === 'session-resume' || cell.variant === 'resume-revoked';
  let tier = await open();
  try {
    if (!resumed) await sessionRun(tier, cell.policy, site.origin, journal);
    else {
      // The process stops after two pages; its journal (cursor, records, and the session state unless the policy logs in at every
      // run) is written to disk, then a new process with a new tier reads it back and continues.
      await sessionRun(tier, cell.policy, site.origin, journal, 2);
      const state = { ...journal, cookies: cell.policy === 'relogin-each-run' ? [] : await tier.exportState() };
      const path = join(directory, `${label}.journal.json`); await writeFile(path, JSON.stringify(state)); await tier.close();
      if (cell.variant === 'resume-revoked') site.revokeAll();
      const restored = JSON.parse(await readFile(path, 'utf8')) as typeof state; tier = await open(); await tier.importState(restored.cookies);
      journal.next = restored.next; journal.records = restored.records; notes.push(`resumed at ${restored.next ? new URL(restored.next).search : 'end'} with ${restored.cookies.length} cookie(s)`);
      await sessionRun(tier, cell.policy, site.origin, journal);
    }
    return { records: journal.records, complete: journal.next === null, notes };
  } catch (error) { return { records: journal.records, complete: false, error: error instanceof PortalError ? `${error.code}: ${error.message}` : `E_TASK: ${error instanceof Error ? error.message : String(error)}`, notes }; }
  finally { await tier.close(); }
}

// ---- Routing ----------------------------------------------------------------------------------------------------------------
type Page = { records: PortalRecord[]; next: string | null; total: number | null; doc: Document };
type ApiPlan = { kind: 'page-mapped' | 'cursor-chain'; describe: string; collect(url: string): Promise<{ records: PortalRecord[]; next: string | null }> };
type Learned = { tier: 'http' | 'browser' | 'api'; plan?: ApiPlan; reason: string };

// Objects of a JSON document that carry an id: records, or partial records such as fresh prices.
function idObjects(json: unknown, found: PortalRecord[] = []): PortalRecord[] {
  if (Array.isArray(json)) for (const item of json) idObjects(item, found);
  else if (json && typeof json === 'object') { const o = json as Record<string, unknown>; if (typeof o.id === 'string' && typeof o.price === 'number') found.push({ id: o.id, price: o.price }); for (const value of Object.values(o)) if (value && typeof value === 'object') idObjects(value, found); }
  return found;
}
const flatten = (value: unknown, prefix = '', into = new Map<string, unknown>()) => { if (value && typeof value === 'object' && !Array.isArray(value)) for (const [k, v] of Object.entries(value)) flatten(v, prefix ? `${prefix}.${k}` : k, into); else into.set(prefix, value); return into; };
const requestFields = (c: Captured) => { const fields = new Map<string, unknown>(); for (const [k, v] of new URL(c.url).searchParams) fields.set(`query.${k}`, v); if (c.postData) { try { for (const [k, v] of flatten(JSON.parse(c.postData))) fields.set(`body.${k}`, v); } catch { /* not JSON */ } } return fields; };
function withField(c: Captured, key: string, value: unknown): { url: string; body: string | null } {
  if (key.startsWith('query.')) { const url = new URL(c.url); url.searchParams.set(key.slice(6), String(value)); return { url: url.href, body: c.postData }; }
  const body = JSON.parse(c.postData!) as Record<string, unknown>; const path = key.slice(5).split('.'); let node = body as Record<string, unknown>;
  for (const part of path.slice(0, -1)) node = node[part] as Record<string, unknown>; node[path.at(-1)!] = value; return { url: c.url, body: JSON.stringify(body) };
}

// From the JSON traffic of a browser sample, a replay plan: an endpoint whose answers carry the page's records, called either once
// per page (a request field equals a parameter of the page URL) or as a chain (a field of the next call is a value of the previous answer).
function inferPlan(pageUrl: string, browserIds: Set<string>, captured: Captured[], http: HttpTier): ApiPlan | null {
  const groups = new Map<string, Captured[]>();
  for (const c of captured) if (idObjects(c.json).some(r => browserIds.has(r.id))) { const key = `${c.method} ${new URL(c.url).origin}${new URL(c.url).pathname}`; groups.set(key, [...(groups.get(key) ?? []), c]); }
  const [endpoint, calls] = [...groups.entries()].sort((a, b) => b[1].length - a[1].length)[0] ?? [];
  if (!endpoint || !calls) return null;
  const htmlRecords = async (url: string) => { const doc = await http.get(url); return { records: parseRecords(doc.body), doc }; };
  if (calls.length >= 2) {
    const first = requestFields(calls[0]!), second = requestFields(calls[1]!); const answer = flatten(calls[0]!.json);
    const key = [...second.keys()].find(k => second.get(k) !== first.get(k) && [...answer.values()].includes(second.get(k)));
    const path = key && [...answer.entries()].find(([, v]) => v === second.get(key))?.[0];
    if (!key || !path) return null;
    return { kind: 'cursor-chain', describe: `${endpoint}, ${key} <- ${path}`, async collect(url) {
      const records = new Map<string, number>(); merge(records, (await htmlRecords(url)).records);
      let cursor = first.get(key);
      for (let call = 0; call < 200; call++) {
        const request = withField(calls[0]!, key, cursor); const json = await http.json(calls[0]!.method, request.url, request.body); const found = idObjects(json);
        const before = records.size; merge(records, found); cursor = flatten(json).get(path);
        if (!found.length || records.size === before || cursor === null || cursor === undefined) break;
      }
      return { records: [...records].map(([id, price]) => ({ id, price })), next: null };
    } };
  }
  const page = new URL(pageUrl); const fields = requestFields(calls[0]!);
  const mapped = [...page.searchParams].find(([name, value]) => fields.get(`query.${name}`) === value)?.[0];
  if (!mapped) return null;
  return { kind: 'page-mapped', describe: `${endpoint}, query.${mapped} follows the page URL`, async collect(url) {
    const current = new URL(url).searchParams.get(mapped)!; const { records: html, doc } = await htmlRecords(url);
    const request = withField(calls[0]!, `query.${mapped}`, current); const api = idObjects(await http.json(calls[0]!.method, request.url, request.body));
    const records = new Map<string, number>(); merge(records, html); merge(records, api);
    // The next page: the document's link, or, for a shell without links, the next value of the mapped parameter while it brings records.
    let next = nextLink(doc); if (!next && api.length) { const following = new URL(url); following.searchParams.set(mapped, String(Number(current) + 1)); next = following.href; }
    return { records: [...records].map(([id, price]) => ({ id, price })), next };
  } };
}

async function routingRun(policy: string, startUrl: string, http: HttpTier, browser: BrowserTier, knowledge: Map<string, Learned>): Promise<RunOutcome> {
  const viaHttp = async (url: string): Promise<Page> => { const doc = await http.get(url); return { records: parseRecords(doc.body), next: nextLink(doc), total: declaredTotal(doc.body), doc }; };
  const viaBrowser = async (url: string): Promise<Page> => { await browser.get(url); const doc = await browser.settle(); return { records: parseRecords(doc.body), next: nextLink(doc), total: declaredTotal(doc.body), doc }; };
  const routeKey = new URL(startUrl).pathname; const collected = new Map<string, number>(); const notes: string[] = []; let total: number | null = null;
  const walk = async (from: string | null, step: (url: string) => Promise<{ records: PortalRecord[]; next: string | null; total?: number | null }>) => {
    const seenIds = new Set(collected.keys());
    for (let url = from, pages = 0; url && pages < 50; pages++) { const page = await step(url); total ??= page.total ?? null; const fresh = page.records.some(r => !seenIds.has(r.id)); merge(collected, page.records); page.records.forEach(r => seenIds.add(r.id)); url = fresh ? page.next : null; }
  };
  if (policy === 'browser-only') await walk(startUrl, viaBrowser);
  else if (policy === 'http-only') await walk(startUrl, viaHttp);
  else if (policy === 'heuristic') await walk(startUrl, async url => { const page = await viaHttp(url); return page.records.length ? page : viaBrowser(url); });
  else {
    let learned = knowledge.get(routeKey); let next: string | null = startUrl;
    if (!learned) {
      // The sample: the first page through HTTP and through the browser; HTTP is kept only if it gives the same records and links.
      const h = await viaHttp(startUrl); browser.capture = policy === 'api-capture'; browser.captured = []; const b = await viaBrowser(startUrl); browser.capture = false;
      merge(collected, b.records); total = b.total ?? h.total; next = b.next;
      if (sameRecords(h.records, b.records) && h.next === b.next) learned = { tier: 'http', reason: 'HTTP sample equals the browser' };
      else {
        const plan = policy === 'api-capture' ? inferPlan(startUrl, new Set(b.records.map(r => r.id)), browser.captured, http) : null;
        const check = plan ? await plan.collect(startUrl) : null;
        learned = plan && check && sameRecords(check.records, b.records) ? { tier: 'api', plan, reason: `API plan ${plan.kind}: ${plan.describe}` } : { tier: 'browser', reason: plan ? 'API plan disagrees with the browser' : `HTTP sample differs: ${h.records.length} vs ${b.records.length} records, ${b.records.filter(r => h.records.some(x => x.id === r.id && x.price !== r.price)).length} price(s) differ` };
        if (learned.tier === 'api' && learned.plan!.kind === 'cursor-chain') next = null;
      }
      knowledge.set(routeKey, learned); notes.push(`learned ${learned.tier}: ${learned.reason}`);
    }
    if (next) await walk(next, learned.tier === 'http' ? viaHttp : learned.tier === 'browser' ? viaBrowser : url => learned!.plan!.collect(url));
    // The contract at the end of the route: a declared total that is not met invalidates what was learned, and the route is redone in the browser.
    if (total === null) total = declaredTotal((await http.get(startUrl)).body);
    if (total !== null && collected.size !== total && learned.tier !== 'browser') {
      notes.push(`contract: ${collected.size} of ${total} records through ${learned.tier}; route relearned as browser`); knowledge.set(routeKey, { tier: 'browser', reason: 'declared total not met' });
      collected.clear(); await walk(startUrl, viaBrowser);
    }
    if (total !== null && collected.size !== total) return { records: [...collected].map(([id, price]) => ({ id, price })), complete: false, error: `E_INCOMPLETE: ${collected.size} of ${total} declared records`, notes };
  }
  return { records: [...collected].map(([id, price]) => ({ id, price })), complete: true, notes };
}

// ---- Frontier ---------------------------------------------------------------------------------------------------------------
const ignoredByRecipe = (name: string) => name === 'sid' || name.startsWith('utm_');
async function frontierRun(policy: string, startUrl: string, http: HttpTier, maxRequests: number): Promise<RunOutcome & { requests: number; terminated: string }> {
  const origin = new URL(startUrl).origin; const records = new Map<string, number>(); const notes: string[] = [];
  const learnedIgnore = new Map<string, Set<string>>(); const signatures = new Map<string, Map<string, string[]>>(); const empties = new Map<string, number>(); const blocked = new Set<string>();
  // naive: the exact URL is the key. canonical: recipe parameters dropped, learned non-functional parameters dropped, keys sorted.
  const key = (href: string) => { if (policy === 'naive') return href; const url = new URL(href); const ignore = learnedIgnore.get(url.pathname) ?? new Set(); const kept = [...url.searchParams].filter(([k]) => !ignoredByRecipe(k) && !ignore.has(k)).sort(([a], [b]) => a.localeCompare(b)); return `${url.pathname}?${new URLSearchParams(kept)}`; };
  const queue = [startUrl]; const seen = new Set([key(startUrl)]); const visited = new Set<string>(); let requests = 0;
  while (queue.length) {
    if (requests >= maxRequests) return { records: [...records].map(([id, price]) => ({ id, price })), complete: false, error: `E_BUDGET: ${maxRequests} requests spent, frontier still holds ${queue.length} URL(s)`, notes, requests, terminated: 'budget' };
    const url = queue.shift()!; const path = new URL(url).pathname;
    // A queued URL is keyed again when it leaves the queue: what was learned since it was found may make it a known page.
    if (blocked.has(path) || visited.has(key(url))) continue;
    visited.add(key(url));
    const doc = await http.get(url); requests++;
    const found = parseRecords(doc.body); for (const r of found) if (!records.has(r.id)) records.set(r.id, r.price);
    let links = [...doc.body.matchAll(/<a [^>]*href="([^"]+)"/g)].map(m => new URL(m[1]!.replaceAll('&amp;', '&'), url).href).filter(href => href.startsWith(origin));
    if (policy === 'canonical') {
      // Cursor pagination wins: a page that declares rel=next is followed by it, not by numbered links of the same listing.
      const relNext = doc.body.match(/<link rel="next" href="([^"]+)"/)?.[1];
      if (relNext) links = [...links.filter(href => new URL(href).pathname !== path), new URL(relNext.replaceAll('&amp;', '&'), url).href];
      // Two pages of one path with the same records and links: the parameters that differ do not change the content.
      const params = [...new URL(url).searchParams.keys()].filter(k => !ignoredByRecipe(k));
      const signature = JSON.stringify([found.map(r => r.id).sort(), [...new Set(links.map(key))].sort()]);
      const byPath = signatures.get(path) ?? new Map<string, string[]>(); const earlier = byPath.get(signature);
      if (earlier) { const extra = params.filter(p => !earlier.includes(p)); if (extra.length) { learnedIgnore.set(path, new Set([...(learnedIgnore.get(path) ?? []), ...extra])); notes.push(`${path}: ${extra.join(', ')} do not change the content`); } }
      else byPath.set(signature, params);
      signatures.set(path, byPath);
      // A path whose pages bring no records and only links to itself, three times in a row, is a trap (an endless calendar).
      const onlySelf = links.every(href => new URL(href).pathname === path || seen.has(key(href)));
      const run = found.length === 0 && onlySelf ? (empties.get(path) ?? 0) + 1 : 0; empties.set(path, run);
      if (run >= 3 && !blocked.has(path)) { blocked.add(path); notes.push(`${path}: three empty pages in a row, path closed`); }
    }
    for (const href of links) { const k = key(href); if (!seen.has(k)) { seen.add(k); queue.push(href); } }
  }
  return { records: [...records].map(([id, price]) => ({ id, price })), complete: true, notes, requests, terminated: 'exhausted' };
}

// ---- Cell -------------------------------------------------------------------------------------------------------------------
export async function runPortalCell(config: PortalConfig, cell: PortalCell, block: number, directory: string): Promise<Record<string, unknown>> {
  const needsBrowser = cell.scenario === 'routing' || cell.tier === 'browser';
  const browser = needsBrowser ? await launchBrowser(browserPath()) : null; const connection = browser ? await CdpConnection.connect(browser.wsEndpoint) : null;
  const durations: number[] = []; const reasons: string[] = []; const notes: string[] = [];
  let verified = 0, falseSuccess = 0, logins = 0, refreshes = 0, siteRequests = 0, browserPages = 0, apiRequests = 0, recordsOut = 0, clientRequests = 0; const steady: number[] = [];
  const knowledge = new Map<string, Learned>(); let shared: PortalSite | null = null;
  try {
    for (let repeat = 0; repeat < config.repeats; repeat++) {
      // Routing runs are a recurring job on one site, so what a route learned carries over; other groups get a fresh site per run.
      const site = cell.scenario === 'routing' ? (shared ??= await startPortalSite({ variant: cell.variant, seed: config.seed + block * 100 })) : await startPortalSite({ variant: cell.variant, seed: config.seed + block * 100 + repeat });
      const before = { ...site.stats };
      try {
        let start = performance.now(); let outcome: RunOutcome; let pages = 0, requests = 0;
        if (cell.scenario === 'session') outcome = await runSessionTask(cell, site, connection, config.timeoutMs, directory, `${cell.variant}-${cell.policy}-${cell.tier}-b${block}-r${repeat}`);
        else if (cell.scenario === 'routing') {
          // The run is timed from an open browser context, as a worker that keeps contexts warm would start it.
          const http = new HttpTier(config.timeoutMs); const tier = await BrowserTier.open(connection!, config.timeoutMs); start = performance.now();
          try { outcome = await routingRun(cell.policy, site.startUrl, http, tier, knowledge); } catch (error) { outcome = { records: [], complete: false, error: `E_TASK: ${error instanceof Error ? error.message : String(error)}` }; }
          finally { pages = tier.pages; requests = http.requests + tier.requests; await tier.close(); }
        } else { const http = new HttpTier(config.timeoutMs); const result = await frontierRun(cell.policy, site.startUrl, http, config.maxRequests ?? 300); outcome = result; requests = result.requests; notes.push(`r${repeat}: ${result.terminated} after ${result.requests} requests`); }
        const elapsed = performance.now() - start; const verdict = site.verify(outcome.records);
        logins += site.stats.logins - before.logins; refreshes += site.stats.refreshes - before.refreshes; siteRequests += site.stats.requests - before.requests; apiRequests += site.stats.api - before.api;
        browserPages += pages; clientRequests += requests; recordsOut += new Set(outcome.records.map(r => r.id)).size;
        // The frontier cannot always prove it is done: a crawl stopped by its budget is judged on its data, and never as a claim of completeness.
        const exactData = verdict.exact && (outcome.complete || cell.scenario === 'frontier');
        if (exactData && !outcome.error?.startsWith('E_TASK')) { verified++; durations.push(elapsed); if (repeat > 0) steady.push(elapsed); }
        if (outcome.complete && !verdict.exact) falseSuccess++;
        if (!verdict.exact || outcome.error) reasons.push(`r${repeat}: ${[outcome.error, ...verdict.reasons].filter(Boolean).join('; ')}`);
        notes.push(...(outcome.notes ?? []).map(n => `r${repeat}: ${n}`));
      } finally { if (cell.scenario !== 'routing') await site.close(); }
    }
  } finally { await shared?.close(); connection?.close(); await browser?.close(); }
  const runs = config.repeats;
  return { ...cell, block, status: verified === runs ? 'verified' : 'failed', repeats: runs, verifiedRuns: verified, falseSuccess,
    logins, loginsPerRun: logins / runs, refreshes, siteRequests, apiRequests, browserPages, browserPagesPerRun: browserPages / runs, clientRequests,
    requestsPerVerifiedRecord: verified === runs && recordsOut ? siteRequests / recordsOut : null,
    taskP50MsVerified: quantiles(durations).p50, steadyRunMs: steady.length ? quantiles(steady).p50 : null,
    reasons: [...new Set(reasons)].slice(0, 8), notes: [...new Set(notes)].slice(0, 12) };
}
