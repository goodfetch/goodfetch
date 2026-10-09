// The two tiers a portal task can run on, behind one interface: plain HTTP with a cookie jar, or a page in its own browser context.
// Both return the final document after redirects, log in through the site's form, refresh a session, and export or import their
// session state (the cookies), which is what a resumed run carries. The browser tier also settles a page (network quiet, then
// scroll and "load more" until the records stop growing) and can capture the JSON traffic of the page for API discovery.
import { CookieJar } from '../identity/session.ts';
import { CdpConnection, CdpPage, type Message } from '../../adapters/cdp-node/cdp.ts';

export type Document = { status: number; url: string; body: string };
export type Cookie = { name: string; value: string; domain: string; path: string; secure: boolean; httpOnly: boolean; sameSite?: string; hostOnly: boolean };
export type Captured = { method: string; url: string; postData: string | null; json: unknown };
export type Credentials = { user: string; password: string };
export interface Tier {
  readonly kind: 'http' | 'browser';
  requests: number;
  get(url: string): Promise<Document>;
  login(loginUrl: string, credentials: Credentials): Promise<boolean>;
  post(url: string): Promise<number>;
  exportState(): Promise<Cookie[]>;
  importState(cookies: Cookie[]): Promise<void>;
  hasCookie(name: string): Promise<boolean>;
  close(): Promise<void>;
}

export const parseRecords = (html: string) => [...html.matchAll(/<article data-id="([^"]+)" data-price="(\d+)"/g)].map(m => ({ id: m[1]!, price: Number(m[2]) }));
const csrfOf = (html: string) => html.match(/name="csrf" value="([^"]+)"/)?.[1] ?? null;

export class HttpTier implements Tier {
  readonly kind = 'http' as const; requests = 0; private jar = new CookieJar(); private timeoutMs: number;
  constructor(timeoutMs: number) { this.timeoutMs = timeoutMs; }
  private async fetch(url: string, init: RequestInit = {}) {
    this.requests++;
    const cookie = this.jar.getCookieString(url);
    const response = await fetch(url, { ...init, redirect: 'manual', signal: AbortSignal.timeout(this.timeoutMs), headers: { ...(init.headers as Record<string, string> ?? {}), ...(cookie ? { cookie } : {}) } });
    for (const header of response.headers.getSetCookie()) this.jar.setCookie(header, url);
    return response;
  }
  async get(url: string): Promise<Document> {
    let current = url;
    for (let hop = 0; hop < 6; hop++) {
      const response = await this.fetch(current); const location = response.headers.get('location');
      if (response.status >= 300 && response.status < 400 && location) { await response.body?.cancel(); current = new URL(location, current).href; continue; }
      return { status: response.status, url: current, body: await response.text() };
    }
    throw new Error('too many redirects');
  }
  async login(loginUrl: string, { user, password }: Credentials) {
    const form = await this.get(loginUrl); const csrf = csrfOf(form.body); if (!csrf) return false;
    const response = await this.fetch(new URL('/login', loginUrl).href, { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams({ csrf, user, password }).toString() });
    await response.body?.cancel(); return response.status === 303 || response.status === 302;
  }
  async post(url: string) { const response = await this.fetch(url, { method: 'POST' }); await response.body?.cancel(); return response.status; }
  // A replayed API call: the method, URL and body a page sent, answered as JSON.
  async json(method: string, url: string, body: string | null) {
    const response = await this.fetch(url, { method, ...(body !== null ? { body, headers: { 'content-type': 'application/json' } } : {}) });
    return response.ok ? await response.json() as unknown : null;
  }
  async exportState() { return [...this.jar.cookies.values()].map(c => ({ ...c })); }
  async importState(cookies: Cookie[]) { for (const c of cookies) this.jar.cookies.set(`${c.name}|${c.domain}|${c.path}`, { ...c }); }
  async hasCookie(name: string) { return [...this.jar.cookies.values()].some(c => c.name === name); }
  async close() {}
}

// One browser context per tier instance: a resumed run gets a new context and only the cookies it imports.
export class BrowserTier implements Tier {
  readonly kind = 'browser' as const; requests = 0; pages = 0; captured: Captured[] = [];
  private connection: CdpConnection; private timeoutMs: number; private contextId = ''; private page?: CdpPage; private sessionId = '';
  private inflight = new Map<string, { method: string; url: string; postData: string | null; json: boolean }>(); private lastActivity = 0; private unsubscribe = () => {};
  capture = false;
  private constructor(connection: CdpConnection, timeoutMs: number) { this.connection = connection; this.timeoutMs = timeoutMs; }
  static async open(connection: CdpConnection, timeoutMs: number) {
    const tier = new BrowserTier(connection, timeoutMs);
    ({ browserContextId: tier.contextId } = await connection.send('Target.createBrowserContext', { disposeOnDetach: true }) as { browserContextId: string });
    const { targetId } = await connection.send('Target.createTarget', { url: 'about:blank', browserContextId: tier.contextId }) as { targetId: string };
    ({ sessionId: tier.sessionId } = await connection.send('Target.attachToTarget', { targetId, flatten: true }) as { sessionId: string });
    tier.page = new CdpPage(connection, targetId, tier.sessionId, timeoutMs);
    await tier.page.send('Page.enable'); await tier.page.send('Network.enable');
    await tier.page.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false });
    tier.unsubscribe = connection.on(message => { void tier.onNetwork(message); });
    return tier;
  }
  // In-flight requests of the page, for settling; JSON answers to XHR and fetch are kept when capture is on.
  private async onNetwork(message: Message) {
    if (message.sessionId !== this.sessionId || !message.method?.startsWith('Network.')) return;
    const params = message.params as { requestId: string; type?: string; request?: { url: string; method: string; postData?: string }; response?: { mimeType: string } };
    this.lastActivity = performance.now();
    if (message.method === 'Network.requestWillBeSent' && params.request && !params.request.url.startsWith('data:')) { this.requests++; this.inflight.set(params.requestId, { method: params.request.method, url: params.request.url, postData: params.request.postData ?? null, json: false }); }
    else if (message.method === 'Network.responseReceived') { const entry = this.inflight.get(params.requestId); if (entry && (params.type === 'XHR' || params.type === 'Fetch') && params.response?.mimeType.includes('json')) entry.json = true; }
    else if (message.method === 'Network.loadingFinished' || message.method === 'Network.loadingFailed') {
      const entry = this.inflight.get(params.requestId); this.inflight.delete(params.requestId);
      if (entry?.json && this.capture && message.method === 'Network.loadingFinished') {
        try { const { body, base64Encoded } = await this.page!.send('Network.getResponseBody', { requestId: params.requestId }) as { body: string; base64Encoded: boolean }; this.captured.push({ method: entry.method, url: entry.url, postData: entry.postData, json: JSON.parse(base64Encoded ? Buffer.from(body, 'base64').toString('utf8') : body) }); } catch { /* body gone with the page */ }
      }
    }
  }
  private async quiet(quietMs = 120) {
    const deadline = performance.now() + this.timeoutMs;
    while (performance.now() < deadline) { if (this.inflight.size === 0 && performance.now() - this.lastActivity >= quietMs) return; await new Promise(resolve => setTimeout(resolve, 20)); }
    throw new Error('page network did not settle');
  }
  private async load(navigate: () => Promise<unknown>): Promise<Document> {
    const document = this.connection.waitForEvent(m => m.method === 'Network.responseReceived' && m.sessionId === this.sessionId && (m.params as { type?: string }).type === 'Document', 'document response', this.timeoutMs);
    const loaded = this.connection.waitFor('Page.domContentEventFired', this.sessionId, this.timeoutMs);
    try { await navigate(); await loaded.promise; } catch (error) { document.cancel(); loaded.cancel(); throw error; }
    const { response } = (await document.promise).params as { response: { status: number; url: string } };
    this.pages++; await this.quiet();
    return { status: response.status, url: response.url, body: await this.page!.evaluate<string>('document.documentElement.outerHTML') };
  }
  get(url: string) { return this.load(async () => { const result = await this.page!.send('Page.navigate', { url }) as { errorText?: string }; if (result.errorText) throw new Error(`navigation failed: ${result.errorText}`); }); }
  // Scroll and press a visible "load more" until the records stop growing; the rendered document comes back.
  async settle(): Promise<Document> {
    let count = -1, stable = 0;
    for (let round = 0; round < 60 && stable < 2; round++) {
      const now = await this.page!.evaluate<number>('document.querySelectorAll("article[data-id]").length');
      stable = now === count ? stable + 1 : 0; count = now;
      const more = await this.page!.evaluate<boolean>('(()=>{const b=document.querySelector("button.more");return !!b&&!b.hidden})()');
      if (more) await this.page!.click('button.more'); else await this.page!.evaluate('window.scrollTo(0, document.body.scrollHeight)');
      await new Promise(resolve => setTimeout(resolve, 30)); await this.quiet();
    }
    return { status: 200, url: await this.page!.evaluate<string>('location.href'), body: await this.page!.evaluate<string>('document.documentElement.outerHTML') };
  }
  async login(loginUrl: string, { user, password }: Credentials) {
    const form = await this.get(loginUrl); if (!/<form id="login"/.test(form.body)) return false;
    await this.page!.fill('input[name="user"]', user); await this.page!.fill('input[name="password"]', password);
    const after = await this.load(() => this.page!.click('button[type="submit"]'));
    return !/<form id="login"/.test(after.body) && after.status < 400;
  }
  async post(url: string) { return this.page!.evaluate<number>(`fetch(${JSON.stringify(url)},{method:'POST'}).then(r=>r.status)`); }
  private async cookies() { return (await this.connection.send('Storage.getCookies', { browserContextId: this.contextId }) as { cookies: { name: string; value: string; domain: string; path: string; secure: boolean; httpOnly: boolean; sameSite?: string }[] }).cookies; }
  async exportState() { return (await this.cookies()).map(c => ({ name: c.name, value: c.value, domain: c.domain.replace(/^\./, ''), path: c.path, secure: c.secure, httpOnly: c.httpOnly, sameSite: c.sameSite, hostOnly: !c.domain.startsWith('.') })); }
  async importState(cookies: Cookie[]) {
    if (!cookies.length) return;
    await this.connection.send('Storage.setCookies', { browserContextId: this.contextId, cookies: cookies.map(c => ({ name: c.name, value: c.value, ...(c.hostOnly ? { url: `${c.secure ? 'https' : 'http'}://${c.domain}${c.path}` } : { domain: c.domain }), path: c.path, secure: c.secure, httpOnly: c.httpOnly, ...(c.sameSite ? { sameSite: c.sameSite } : {}) })) });
  }
  async hasCookie(name: string) { return (await this.cookies()).some(c => c.name === name); }
  async close() { this.unsubscribe(); await this.connection.send('Target.disposeBrowserContext', { browserContextId: this.contextId }).catch(() => {}); }
}
