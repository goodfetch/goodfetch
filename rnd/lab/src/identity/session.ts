// A session that carries one identity profile across HTTP and the browser (ADR-002, architectural consequences): the profile is
// fixed at opening, the HTTP tier sends the headers the session's own browser sends, cookies follow the escalation both ways,
// and in the identified profile every request of both tiers is signed with Web Bot Auth.
import { createServer } from 'node:http';
import { request as httpsRequest, Agent as HttpsAgent } from 'node:https';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import type { AddressInfo } from 'node:net';
import { CdpConnection, CdpPage, type Message } from '../../adapters/cdp-node/cdp.ts';
import { labRoot } from '../registry.ts';
import { signRequest, SigningError, type KeyStore } from './signature.ts';
import { SitePolicy, StopError, classifyResponse, type Purpose } from './policy.ts';
import { startEgress, type ProxyPool } from './egress.ts';
import { randomBytes } from 'node:crypto';

export type Profile = 'identified' | 'standard';
export type HttpTier = 'node-https' | 'impit-chrome' | 'browser-fetch';
export const productToken = 'AutomanLab';
export class ProfileLockedError extends Error { readonly code = 'E_PROFILE_LOCKED'; }
export type SessionOptions = { profile: Profile; httpTier: HttpTier; purpose: Purpose; connection: CdpConnection; keys?: KeyStore; agentOrigin?: string; contactUrl?: string; timeoutMs: number; retryBudget?: number; proxyPool?: ProxyPool };
export type SiteResponse = { status: number; headers: Headers; body: string; url: string };
type ImpitClient = { fetch(url: string, init?: { headers?: [string, string][] }): Promise<{ status: number; headers: Headers; text(): Promise<string> }> };
type Cookie = { name: string; value: string; domain: string; path: string; secure: boolean; httpOnly: boolean; sameSite?: string; hostOnly: boolean };

// The cookies of the session, shared by the HTTP tier and pushed to (and pulled from) the session's browser context.
export class CookieJar {
  cookies = new Map<string, Cookie>();
  setCookie(header: string, url: string) {
    const { hostname, pathname } = new URL(url); const [pair, ...attributes] = header.split(';'); const eq = pair!.indexOf('='); if (eq <= 0) return;
    const cookie: Cookie = { name: pair!.slice(0, eq).trim(), value: pair!.slice(eq + 1).trim(), domain: hostname, path: pathname.replace(/\/[^/]*$/, '') || '/', secure: false, httpOnly: false, hostOnly: true };
    for (const attribute of attributes) {
      const [key, value = ''] = attribute.split('=').map(s => s.trim()); const name = key!.toLowerCase();
      if (name === 'path' && value.startsWith('/')) cookie.path = value; else if (name === 'domain' && value) { cookie.domain = value.replace(/^\./, ''); cookie.hostOnly = false; }
      else if (name === 'secure') cookie.secure = true; else if (name === 'httponly') cookie.httpOnly = true; else if (name === 'samesite') cookie.sameSite = value;
      else if (name === 'max-age' && Number(value) <= 0) { this.cookies.delete(`${cookie.name}|${cookie.domain}|${cookie.path}`); return; }
    }
    this.cookies.set(`${cookie.name}|${cookie.domain}|${cookie.path}`, cookie);
  }
  getCookieString(url: string) {
    const { hostname, pathname, protocol } = new URL(url);
    return [...this.cookies.values()].filter(c => (c.hostOnly ? hostname === c.domain : hostname === c.domain || hostname.endsWith(`.${c.domain}`)) && pathname.startsWith(c.path) && (!c.secure || protocol === 'https:')).map(c => `${c.name}=${c.value}`).join('; ');
  }
}

const excludedProbe = new Set(['host', 'connection', 'cookie', 'content-length', 'signature', 'signature-input', 'signature-agent']);
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

export class IdentitySession {
  readonly profile: Profile; readonly httpTier: HttpTier; readonly purpose: Purpose; readonly retryBudget: number;
  userAgent = ''; signatures = 0; readonly stickyId = randomBytes(6).toString('hex');
  // One retry budget for the session: waits on Retry-After and proxy rotations draw from it alike (architecture §3.4).
  readonly budget = { retries: 0, limit: 2 };
  get retries() { return this.budget.retries; }
  egress?: Awaited<ReturnType<typeof startEgress>>;
  // The journal of the session: one line per request decision, with the site preferences in force.
  readonly journal: { url: string; tier: 'http' | 'browser'; status: number | null; decision: string; robotsSha256: string | null }[] = [];
  private options: SessionOptions; private contextId = ''; private probe: [string, string][] = []; private jar = new CookieJar();
  private policies = new Map<string, SitePolicy>(); private lastRequest = new Map<string, number>(); private pageSessions = new Set<string>();
  private unsubscribe: () => void = () => {}; private impit?: ImpitClient; private agent = new HttpsAgent({ keepAlive: true, maxSockets: 1, rejectUnauthorized: false });
  private fetcher?: { page: CdpPage; sessionId: string }; private browserCookiesNewer = false; private deadline: number;

  private constructor(options: SessionOptions) {
    this.options = options; this.profile = options.profile; this.httpTier = options.httpTier; this.purpose = options.purpose; this.retryBudget = options.retryBudget ?? 2; this.budget.limit = this.retryBudget;
    this.deadline = performance.now() + options.timeoutMs;
    if (options.profile === 'identified' && (!options.keys || !options.agentOrigin)) throw new Error('the identified profile needs keys and an agent directory');
  }

  static async open(options: SessionOptions) {
    const session = new IdentitySession(options); const { connection } = options;
    // With a proxy pool, the context always goes through the session's own egress, which picks and changes the upstream.
    if (options.proxyPool) { if (options.httpTier !== 'browser-fetch') throw new Error('proxies are wired for the in-browser HTTP tier only'); session.egress = await startEgress(options.proxyPool, session.stickyId, session.budget, options.timeoutMs); }
    ({ browserContextId: session.contextId } = await connection.send('Target.createBrowserContext', { disposeOnDetach: true, ...(session.egress ? { proxyServer: session.egress.url, proxyBypassList: '<-loopback>;127.0.0.1' } : {}) }) as { browserContextId: string });
    const { userAgent } = await connection.send('Browser.getVersion') as { userAgent: string };
    session.userAgent = options.profile === 'identified' ? `${userAgent} ${productToken}/0.1 (+${options.contactUrl ?? 'https://automan.invalid/bot'})` : userAgent;
    session.unsubscribe = connection.on(message => { void session.onPaused(message); });
    await session.calibrate();
    if (options.httpTier === 'impit-chrome') {
      const { Impit } = createRequire(join(labRoot, 'adapters/node/package.json'))('impit') as { Impit: new (options: Record<string, unknown>) => ImpitClient };
      session.impit = new Impit({ browser: 'chrome', ignoreTlsErrors: true, followRedirects: false, timeout: options.timeoutMs, cookieJar: session.jar });
    }
    return session;
  }

  // A profile belongs to the session for its whole life; another profile means another session.
  switchProfile(profile: Profile) { if (profile !== this.profile) throw new ProfileLockedError(`session opened as ${this.profile}; open a new session for ${profile}`); }

  // The navigation headers this session's browser really sends, in order, captured once on a loopback page it owns.
  private async calibrate() {
    let captured: [string, string][] = [];
    const echo = createServer((req, res) => { captured = []; for (let i = 0; i < req.rawHeaders.length; i += 2) captured.push([req.rawHeaders[i]!.toLowerCase(), req.rawHeaders[i + 1]!]); res.end('<!doctype html><title>probe</title>'); });
    await new Promise<void>(resolve => echo.listen(0, '127.0.0.1', resolve));
    const { page } = await this.newPage();
    try { await page.goto(`http://127.0.0.1:${(echo.address() as AddressInfo).port}/`); } finally { await page.close(); echo.closeAllConnections(); echo.close(); }
    this.probe = captured.filter(([name]) => !excludedProbe.has(name));
  }

  private async newPage() {
    const { connection } = this.options;
    const { targetId } = await connection.send('Target.createTarget', { url: 'about:blank', browserContextId: this.contextId }) as { targetId: string };
    const { sessionId } = await connection.send('Target.attachToTarget', { targetId, flatten: true }) as { sessionId: string };
    const page = new CdpPage(connection, targetId, sessionId, this.options.timeoutMs); this.pageSessions.add(sessionId);
    await page.send('Page.enable'); await page.send('Network.enable');
    if (this.profile === 'identified') { await page.send('Emulation.setUserAgentOverride', { userAgent: this.userAgent }); await page.send('Fetch.enable', { patterns: [{ urlPattern: '*', requestStage: 'Request' }] }); }
    return { page, sessionId };
  }

  private signatureHeaders(url: string): [string, string][] {
    if (this.profile !== 'identified') return [];
    const signed = signRequest({ method: 'GET', url, headers: {} }, this.options.keys!, this.options.agentOrigin!); this.signatures++;
    return [['signature-agent', signed['signature-agent']], ['signature-input', signed['signature-input']], ['signature', signed.signature]];
  }

  // Browser requests of the identified profile are signed as they leave; a request that cannot be signed is not sent.
  private async onPaused(message: Message) {
    if (message.method !== 'Fetch.requestPaused' || !message.sessionId || !this.pageSessions.has(message.sessionId)) return;
    const { requestId, request } = message.params as { requestId: string; request: { url: string; headers: Record<string, string> } };
    const { connection } = this.options;
    try {
      const extra = /^https?:/.test(request.url) && !request.url.startsWith(this.options.agentOrigin!) && !request.url.startsWith('http://127.0.0.1') ? this.signatureHeaders(request.url) : [];
      await connection.send('Fetch.continueRequest', { requestId, headers: [...Object.entries(request.headers), ...extra].map(([name, value]) => ({ name, value })) }, message.sessionId);
    } catch (error) {
      this.journal.push({ url: request.url, tier: 'browser', status: null, decision: error instanceof SigningError ? error.code : 'E_SIGNING', robotsSha256: null });
      await connection.send('Fetch.failRequest', { requestId, errorReason: 'AccessDenied' }, message.sessionId).catch(() => {});
    }
  }

  // Site preferences are loaded through the session's own transport, so the robots.txt request carries the same identity.
  private async before(url: string, tier: 'http' | 'browser') {
    const origin = new URL(url).origin; let policy = this.policies.get(origin);
    if (!policy) { policy = await SitePolicy.load(origin, async u => { const r = await this.transport(u); return { status: r.status, body: r.body }; }, this.profile === 'identified' ? productToken : null); this.policies.set(origin, policy); }
    try { policy.check(url, this.purpose); } catch (error) { this.journal.push({ url, tier, status: null, decision: (error as StopError).code, robotsSha256: policy.evidence.robotsSha256 }); throw error; }
    const last = this.lastRequest.get(origin); const wait = last === undefined ? 0 : policy.crawlDelayMs - (performance.now() - last);
    if (wait > 0) { if (performance.now() + wait > this.deadline) throw new StopError('E_RATE_LIMITED', 'crawl-delay exceeds the task deadline'); await sleep(wait); }
    this.lastRequest.set(origin, performance.now());
    return policy;
  }

  private after(policy: SitePolicy, url: string, tier: 'http' | 'browser', response: { status: number; headers: Headers; body: string }) {
    const stop = classifyResponse(response.status, response.headers, response.body);
    const entry = { url, tier, status: response.status, decision: stop?.code ?? 'allowed', robotsSha256: policy.evidence.robotsSha256 };
    if (!stop) try { policy.checkResponse(this.purpose, response.headers, response.body); } catch (error) { entry.decision = (error as StopError).code; this.journal.push(entry); throw error; }
    this.journal.push(entry);
    return stop;
  }

  // One GET on the HTTP tier, with preferences, stop signals and bounded waits on Retry-After from the shared retry budget.
  async get(url: string): Promise<SiteResponse> {
    for (;;) {
      const policy = await this.before(url, 'http');
      const response = await this.transport(url);
      const stop = this.after(policy, url, 'http', response);
      if (!stop) return response;
      if ((stop.code === 'E_RATE_LIMITED' || stop.code === 'E_SERVER') && stop.retryAfterMs !== null && this.budget.retries < this.budget.limit && performance.now() + stop.retryAfterMs < this.deadline) { this.budget.retries++; await sleep(stop.retryAfterMs); continue; }
      throw new StopError(stop.code, `HTTP ${response.status} at ${new URL(url).pathname}`);
    }
  }

  // A page in the session's browser context, after the same preference checks; cookies set over HTTP come along.
  async browse(url: string): Promise<CdpPage> {
    const policy = await this.before(url, 'browser');
    await this.pushCookies();
    const { page, sessionId } = await this.newPage();
    const document = this.options.connection.waitForEvent(m => m.method === 'Network.responseReceived' && m.sessionId === sessionId && (m.params as { type?: string }).type === 'Document', 'document response', this.options.timeoutMs);
    try { await page.goto(url); } catch (error) { document.cancel(); await page.close(); throw this.navigationError(error); }
    const { response } = (await document.promise).params as { response: { status: number; headers: Record<string, string> } };
    const body = await page.evaluate<string>('document.documentElement.outerHTML');
    this.browserCookiesNewer = true;
    const stop = this.after(policy, url, 'browser', { status: response.status, headers: new Headers(response.headers), body });
    if (stop) { await page.close(); throw new StopError(stop.code, `HTTP ${response.status} at ${new URL(url).pathname}`); }
    return page;
  }

  private async pushCookies() {
    if (this.httpTier === 'browser-fetch' || !this.jar.cookies.size) return;
    await this.options.connection.send('Storage.setCookies', { browserContextId: this.contextId, cookies: [...this.jar.cookies.values()].map(c => ({ name: c.name, value: c.value, ...(c.hostOnly ? { url: `${c.secure ? 'https' : 'http'}://${c.domain}${c.path}` } : { domain: c.domain }), path: c.path, secure: c.secure, httpOnly: c.httpOnly, ...(c.sameSite ? { sameSite: c.sameSite[0]!.toUpperCase() + c.sameSite.slice(1).toLowerCase() } : {}) })) });
  }
  private async pullCookies() {
    if (!this.browserCookiesNewer || this.httpTier === 'browser-fetch') return;
    const { cookies } = await this.options.connection.send('Storage.getCookies', { browserContextId: this.contextId }) as { cookies: { name: string; value: string; domain: string; path: string; secure: boolean; httpOnly: boolean; sameSite?: string }[] };
    this.jar.cookies.clear();
    for (const c of cookies) this.jar.cookies.set(`${c.name}|${c.domain.replace(/^\./, '')}|${c.path}`, { name: c.name, value: c.value, domain: c.domain.replace(/^\./, ''), path: c.path, secure: c.secure, httpOnly: c.httpOnly, sameSite: c.sameSite, hostOnly: !c.domain.startsWith('.') });
    this.browserCookiesNewer = false;
  }

  private async transport(url: string): Promise<SiteResponse> {
    await this.pullCookies();
    if (this.httpTier === 'browser-fetch') return this.browserFetch(url);
    const cookie = this.jar.getCookieString(url);
    const headers: [string, string][] = [...this.probe.map(([name, value]) => [name, name === 'user-agent' ? this.userAgent : value] as [string, string]), ...this.signatureHeaders(url)];
    if (this.httpTier === 'impit-chrome') {
      const response = await this.impit!.fetch(url, { headers }); const body = await response.text();
      return { status: response.status, headers: response.headers, body, url };
    }
    return new Promise((resolve, reject) => {
      const request = httpsRequest(url, { method: 'GET', agent: this.agent, headers: Object.fromEntries([...headers, ...(cookie ? [['cookie', cookie]] : [])]), timeout: this.options.timeoutMs }, response => {
        const chunks: Buffer[] = []; response.on('data', chunk => chunks.push(chunk)); response.on('error', reject);
        response.on('end', () => {
          for (const header of ([] as string[]).concat(response.headers['set-cookie'] ?? [])) this.jar.setCookie(header, url);
          const result = new Headers(); for (const [name, value] of Object.entries(response.headers)) if (value !== undefined) result.set(name, Array.isArray(value) ? value.join(', ') : value);
          resolve({ status: response.statusCode ?? 0, headers: result, body: Buffer.concat(chunks).toString('utf8'), url });
        });
      });
      request.on('timeout', () => request.destroy(new Error('HTTP request timeout'))); request.on('error', reject); request.end();
    });
  }

  // The HTTP tier inside the browser: a document load with scripts disabled, so TLS, HTTP/2 and headers are the browser's own.
  private async browserFetch(url: string): Promise<SiteResponse> {
    if (!this.fetcher) { this.fetcher = await this.newPage(); await this.fetcher.page.send('Emulation.setScriptExecutionDisabled', { value: true }); }
    const { page, sessionId } = this.fetcher;
    const document = this.options.connection.waitForEvent(m => m.method === 'Network.responseReceived' && m.sessionId === sessionId && (m.params as { type?: string }).type === 'Document', 'document response', this.options.timeoutMs);
    try { await page.goto(url); } catch (error) { document.cancel(); throw this.navigationError(error); }
    const { requestId, response } = (await document.promise).params as { requestId: string; response: { status: number; headers: Record<string, string> } };
    const { body, base64Encoded } = await page.send('Network.getResponseBody', { requestId }) as { body: string; base64Encoded: boolean };
    return { status: response.status, headers: new Headers(response.headers), body: base64Encoded ? Buffer.from(body, 'base64').toString('utf8') : body, url };
  }

  private navigationError(error: unknown) {
    return this.egress?.state.exhausted ? new StopError('E_PROXY_EXHAUSTED', `no healthy proxy left within the retry budget (${this.budget.retries} rotation(s))`) : error;
  }

  async close() {
    this.unsubscribe(); this.agent.destroy(); await this.egress?.close();
    await this.options.connection.send('Target.disposeBrowserContext', { browserContextId: this.contextId }).catch(() => {});
  }
}
