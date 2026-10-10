// Site preferences applied before and after each request of a session (ADR-002 §2): robots.txt (RFC 9309) with crawl-delay,
// Content-Signal, TDMRep, meta noai, and the responses that are stop or licence signals rather than content.
import { createHash } from 'node:crypto';

export type Purpose = 'search' | 'ai-input' | 'ai-train' | 'agent' | 'data-collection' | 'transact';
export type StopCode = 'E_POLICY_ROBOTS' | 'E_POLICY_CONTENT_SIGNAL' | 'E_POLICY_TDM' | 'E_POLICY_NOAI' | 'E_ACCESS_DENIED' | 'E_PAYMENT_REQUIRED' | 'E_CHALLENGE' | 'E_RATE_LIMITED' | 'E_SERVER' | 'E_PROXY_EXHAUSTED';
export class StopError extends Error { readonly code: StopCode; constructor(code: StopCode, message: string) { super(`${code}: ${message}`); this.code = code; } }

// Which purposes each signal restricts. Content-Signal names its own purposes; a TDM reservation covers text and data mining.
const tdmPurposes: Purpose[] = ['ai-train', 'ai-input', 'data-collection'];
const noaiPurposes: Purpose[] = ['ai-train', 'ai-input'];

type Rule = { allow: boolean; pattern: string };
type Group = { agents: string[]; rules: Rule[]; crawlDelay: number | null; contentSignal: Record<string, boolean> };

export function parseRobots(text: string): Group[] {
  const groups: Group[] = []; let current: Group | null = null; let collectingAgents = false;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/#.*$/, '').trim(); const colon = line.indexOf(':'); if (colon < 0) continue;
    const key = line.slice(0, colon).trim().toLowerCase(), value = line.slice(colon + 1).trim();
    if (key === 'user-agent') {
      if (!collectingAgents || !current) { current = { agents: [], rules: [], crawlDelay: null, contentSignal: {} }; groups.push(current); }
      current.agents.push(value.toLowerCase()); collectingAgents = true; continue;
    }
    if (!current) continue; collectingAgents = false;
    if (key === 'allow' || key === 'disallow') { if (value) current.rules.push({ allow: key === 'allow', pattern: value }); }
    else if (key === 'crawl-delay') { const seconds = Number(value); if (Number.isFinite(seconds) && seconds >= 0) current.crawlDelay = seconds; }
    else if (key === 'content-signal') for (const part of value.split(',')) { const [name, setting] = part.split('=').map(s => s.trim().toLowerCase()); if (name && (setting === 'yes' || setting === 'no')) current.contentSignal[name] = setting === 'yes'; }
  }
  return groups;
}

// The groups that apply to a product token: every group naming it (merged), else the '*' groups; RFC 9309 §2.2.1.
export function groupsFor(groups: Group[], token: string | null): Group {
  const named = token ? groups.filter(g => g.agents.includes(token.toLowerCase())) : [];
  const chosen = named.length ? named : groups.filter(g => g.agents.includes('*'));
  return { agents: chosen.flatMap(g => g.agents), rules: chosen.flatMap(g => g.rules), crawlDelay: chosen.map(g => g.crawlDelay).find(d => d !== null) ?? null, contentSignal: Object.assign({}, ...chosen.map(g => g.contentSignal)) };
}

const patternRegex = (pattern: string) => new RegExp('^' + pattern.replace(/[.+?^{}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\\?\$$/, '$'));
// The longest matching pattern decides; on a tie between allow and disallow, allow wins (RFC 9309 §2.2.2).
export function pathAllowed(rules: Rule[], path: string) {
  let best: Rule | null = null;
  for (const rule of rules) if (patternRegex(rule.pattern).test(path) && (!best || rule.pattern.length > best.pattern.length || (rule.pattern.length === best.pattern.length && rule.allow))) best = rule;
  return best?.allow ?? true;
}

type TdmEntry = { location: string; 'tdm-reservation': number; 'tdm-policy'?: string };
const sha256 = (text: string) => createHash('sha256').update(text).digest('hex');
export type FetchText = (url: string) => Promise<{ status: number; body: string }>;

// The preferences of one origin for one product token, loaded once per session; the hashes go into the provenance of every attempt.
export class SitePolicy {
  private readonly group: Group; private readonly tdm: TdmEntry[]; readonly robotsDisallowAll: boolean; readonly evidence: { robotsStatus: number; robotsSha256: string | null; tdmrepSha256: string | null };
  private constructor(group: Group, tdm: TdmEntry[], robotsDisallowAll: boolean, evidence: SitePolicy['evidence']) { this.group = group; this.tdm = tdm; this.robotsDisallowAll = robotsDisallowAll; this.evidence = evidence; }
  static async load(origin: string, fetchText: FetchText, token: string | null) {
    const robots = await fetchText(`${origin}/robots.txt`);
    // 4xx: no restriction; 5xx or unreachable: complete disallow (RFC 9309 §2.3.1.3–4).
    const disallowAll = robots.status >= 500 || robots.status === 0;
    const group = robots.status >= 200 && robots.status < 300 ? groupsFor(parseRobots(robots.body), token) : { agents: [], rules: [], crawlDelay: null, contentSignal: {} };
    const tdmrep = await fetchText(`${origin}/.well-known/tdmrep.json`);
    let tdm: TdmEntry[] = []; if (tdmrep.status === 200) { try { const parsed = JSON.parse(tdmrep.body) as unknown; if (Array.isArray(parsed)) tdm = parsed.filter((e): e is TdmEntry => typeof e?.location === 'string'); } catch { /* malformed: ignored */ } }
    return new SitePolicy(group, tdm, disallowAll, { robotsStatus: robots.status, robotsSha256: robots.status === 200 ? sha256(robots.body) : null, tdmrepSha256: tdmrep.status === 200 ? sha256(tdmrep.body) : null });
  }
  get crawlDelayMs() { return this.group.crawlDelay === null ? 0 : this.group.crawlDelay * 1000; }
  // Before a request: robots, then the purpose-specific site-wide signals.
  check(url: string, purpose: Purpose) {
    const { pathname, search } = new URL(url);
    if (this.robotsDisallowAll) throw new StopError('E_POLICY_ROBOTS', 'robots.txt unreachable: complete disallow');
    if (!pathAllowed(this.group.rules, pathname + search)) throw new StopError('E_POLICY_ROBOTS', `${pathname} disallowed by robots.txt`);
    if (this.group.contentSignal[purpose] === false) throw new StopError('E_POLICY_CONTENT_SIGNAL', `Content-Signal ${purpose}=no`);
    if (tdmPurposes.includes(purpose)) {
      const match = this.tdm.filter(entry => patternRegex(entry.location).test(pathname)).sort((a, b) => b.location.length - a.location.length)[0];
      if (match?.['tdm-reservation'] === 1) throw new StopError('E_POLICY_TDM', `TDM rights reserved for ${pathname}${match['tdm-policy'] ? ` (policy ${match['tdm-policy']})` : ''}`);
    }
  }
  // After a response: page-level reservations; the content is discarded when they apply.
  checkResponse(purpose: Purpose, headers: Headers, body: string) {
    if (tdmPurposes.includes(purpose) && (headers.get('tdm-reservation')?.trim() === '1' || /<meta\s+name=["']tdm-reservation["']\s+content=["']1["']/i.test(body))) throw new StopError('E_POLICY_TDM', 'page reserves TDM rights');
    const robotsMeta = [...body.matchAll(/<meta\s+name=["']robots["']\s+content=["']([^"']*)["']/gi)].map(m => m[1]!.toLowerCase()).join(',') + ',' + (headers.get('x-robots-tag') ?? '').toLowerCase();
    if (noaiPurposes.includes(purpose) && /(^|[\s,])noai([\s,]|$)/.test(robotsMeta)) throw new StopError('E_POLICY_NOAI', 'page opts out of AI use (noai)');
  }
}

// Responses that are never content: refusals, payment, challenges served with any status, and rate limits with their delay.
const challengeMarkers = [/<title>\s*(just a moment|attention required|verify you are human)/i, /checking your browser/i, /cf-chl|challenge-platform|cf-turnstile|g-recaptcha|h-captcha/i];
export function classifyResponse(status: number, headers: Headers, body: string): { code: StopCode; retryAfterMs: number | null } | null {
  const retryAfter = () => { const value = headers.get('retry-after'); if (!value) return null; const seconds = Number(value); if (Number.isFinite(seconds)) return Math.max(0, seconds * 1000); const date = Date.parse(value); return Number.isFinite(date) ? Math.max(0, date - Date.now()) : null; };
  if (status === 401 || status === 403) return challengeMarkers.some(m => m.test(body)) ? { code: 'E_CHALLENGE', retryAfterMs: null } : { code: 'E_ACCESS_DENIED', retryAfterMs: null };
  if (status === 402) return { code: 'E_PAYMENT_REQUIRED', retryAfterMs: null };
  if (status === 429) return { code: 'E_RATE_LIMITED', retryAfterMs: retryAfter() };
  if (status >= 500) return { code: 'E_SERVER', retryAfterMs: retryAfter() };
  if (challengeMarkers.some(m => m.test(body))) return { code: 'E_CHALLENGE', retryAfterMs: null };
  return null;
}
