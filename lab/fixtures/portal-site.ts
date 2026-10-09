// Portal pages for the session, routing and frontier contracts. One site per cell, plain HTTP on loopback; its oracle knows the true
// records and counts what the client did (logins, refreshes, HTML and API requests). Session sites behave like B2B portals: a CSRF
// login, sessions that expire with a 200 login page or a redirect, a refresh token, revocation while the client is down. Routing
// sites hold the same listing behind different rendering: server HTML, a script shell, partial hydration, stale HTML corrected by
// script, GraphQL behind a "load more" button, infinite scroll. Frontier sites are traps: session ids and tracking parameters in
// every link, sort permutations of the same listing, an endless calendar, and a large table that changes while it is paged.
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { random } from '../src/statistics.ts';

export const sessionVariants = ['login-csrf', 'session-expiry', 'session-refresh', 'session-resume', 'resume-revoked'] as const;
export const routingVariants = ['static', 'js-shell', 'partial-hydration', 'stale-html', 'graphql', 'infinite-scroll'] as const;
export const frontierVariants = ['crawler-trap', 'large-table'] as const;
export type PortalVariant = typeof sessionVariants[number] | typeof routingVariants[number] | typeof frontierVariants[number];
export type PortalRecord = { id: string; price: number };
export type PortalVerdict = { exact: boolean; missing: number; unexpected: number; duplicates: number; wrongPrice: number; reasons: string[] };
// Test credentials of the fixture; they exist nowhere else.
export const portalUser = { user: 'lab', password: 'lab-pass' };

const pad = (n: number, width = 3) => String(n).padStart(width, '0');
const article = (r: PortalRecord & { name?: string }) => `<article data-id="${r.id}" data-price="${r.price}"><h2>${r.name ?? r.id}</h2></article>`;
const page = (title: string, body: string, head = '') => `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title>${head}</head><body>${body}</body></html>`;
const cookies = (req: IncomingMessage) => Object.fromEntries((req.headers.cookie ?? '').split(';').map(c => c.trim().split('=')).filter(([k]) => k).map(([k, ...v]) => [k, v.join('=')]));
const readBody = (req: IncomingMessage) => new Promise<string>(resolve => { let body = ''; req.setEncoding('utf8'); req.on('data', c => { body += c; }); req.on('end', () => resolve(body)); });

export async function startPortalSite({ variant, seed }: { variant: PortalVariant; seed: number }) {
  const rand = random(seed); const token = () => Math.floor(rand() * 2 ** 32).toString(36) + Math.floor(rand() * 2 ** 32).toString(36);
  const stats = { requests: 0, html: 0, api: 0, logins: 0, refreshes: 0, csrfFailures: 0, loginPages: 0, maxSessionUses: 0 };
  const send = (res: ServerResponse, status: number, body: string, headers: Record<string, string | string[]> = {}) => { res.writeHead(status, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store', ...headers }); res.end(body); };
  const json = (res: ServerResponse, value: unknown) => { stats.api++; res.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' }); res.end(JSON.stringify(value)); };

  // Session portal: 5 report pages of 20 records behind a CSRF login.
  const reports = Array.from({ length: 100 }, (_, i) => ({ id: `r${pad(i + 1)}`, price: 100 + Math.floor(rand() * 900) }));
  const preSessions = new Map<string, string>(); const sessions = new Map<string, { uses: number }>(); const refreshTokens = new Set<string>();
  const sessionLimit = variant === 'session-expiry' ? 4 : variant === 'session-refresh' ? 3 : Infinity;
  const loginPage = (res: ServerResponse, status = 200) => {
    const pre = token(), csrf = token(); preSessions.set(pre, csrf); stats.loginPages++;
    send(res, status, page('Sign in', `<form id="login" method="post" action="/login"><input type="hidden" name="csrf" value="${csrf}"><label>User <input name="user"></label><label>Password <input name="password" type="password"></label><button type="submit">Sign in</button></form>`), { 'set-cookie': `pre=${pre}; Path=/; HttpOnly; SameSite=Lax` });
  };

  // Routing listings: 100 records with a current price; stale HTML shows an older one.
  const listing = Array.from({ length: 100 }, (_, i) => ({ id: `p${pad(i + 1)}`, name: `Product ${i + 1}`, price: 50 + Math.floor(rand() * 450) }));
  const stalePrice = (r: PortalRecord) => r.price + 7;
  const pageOf = (n: number) => listing.slice((n - 1) * 20, n * 20);
  const shell = (title: string, script: string, inner = '', after = '') => page(title, `<main id="records" data-total="100">${inner}</main>${after}<script>const main=document.querySelector('#records');const render=items=>{for(const r of items){const a=document.createElement('article');a.dataset.id=r.id;a.dataset.price=r.price;a.innerHTML='<h2>'+r.name+'</h2>';main.append(a)}};${script}</script>`);
  const nextLink = (n: number) => n < 5 ? `<a class="next" href="/list?page=${n + 1}">Next</a>` : '';
  const cursorOf = (index: number) => Buffer.from(`after:${index}`).toString('base64url');
  const indexOf = (cursor: string | null | undefined) => cursor ? Number(Buffer.from(cursor, 'base64url').toString().split(':')[1]) : 0;

  // Frontier: products reachable through categories, sort permutations and links carrying a fresh session id; an endless calendar.
  const products = Array.from({ length: 60 }, (_, i) => ({ id: `item${pad(i + 1)}`, price: 10 + Math.floor(rand() * 90), category: 'abc'[i % 3]! }));
  const events = ['2026-01', '2026-02', '2026-03'].flatMap(month => [1, 2].map(k => ({ id: `ev-${month}-${k}`, price: 0, month })));
  const sid = () => `sid=${token()}`;
  const monthAfter = (month: string, delta: number) => { const [y, m] = month.split('-').map(Number); const d = new Date(Date.UTC(y!, m! - 1 + delta, 1)); return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1, 2)}`; };
  // Large table: 1500 rows, newest first; every third page request deletes two rows already paged past and inserts one at the top.
  let rows = Array.from({ length: 1500 }, (_, i) => ({ id: `o${pad(i + 1, 5)}`, n: i + 1, price: 1 + Math.floor(rand() * 999) })).reverse();
  const initial = new Set(rows.map(r => r.id)); const deleted = new Set<string>(); let nextRow = 1501, tableRequests = 0;
  const mutate = (offset: number) => {
    if (++tableRequests % 3) return;
    for (let k = 0; k < 2 && offset > 0; k++) { const victim = Math.floor(rand() * Math.min(offset, rows.length)); deleted.add(rows[victim]!.id); rows.splice(victim, 1); }
    rows.unshift({ id: `o${pad(nextRow, 5)}`, n: nextRow, price: 1 + Math.floor(rand() * 999) }); nextRow++;
  };
  const truth = (): PortalRecord[] => variant === 'large-table' ? [] : (sessionVariants as readonly string[]).includes(variant) ? reports : (routingVariants as readonly string[]).includes(variant) ? listing : [...products, ...events];

  const server = createServer(async (req, res) => {
    stats.requests++;
    const url = new URL(req.url ?? '/', 'http://portal.test'); const path = url.pathname; const jar = cookies(req);
    if (path === '/favicon.ico') { res.writeHead(404); res.end(); return; }
    if ((sessionVariants as readonly string[]).includes(variant)) {
      if (path === '/login' && req.method === 'GET') { stats.html++; loginPage(res); return; }
      if (path === '/login' && req.method === 'POST') {
        const form = new URLSearchParams(await readBody(req)); const expected = preSessions.get(jar.pre ?? '');
        if (!expected || form.get('csrf') !== expected) { stats.csrfFailures++; send(res, 403, page('Forbidden', '<p>Invalid CSRF token</p>')); return; }
        preSessions.delete(jar.pre!);
        if (form.get('user') !== portalUser.user || form.get('password') !== portalUser.password) { loginPage(res, 401); return; }
        stats.logins++; const id = token(); sessions.set(id, { uses: 0 });
        const set = [`sid=${id}; Path=/; HttpOnly; SameSite=Lax`]; if (variant === 'session-refresh') { const rt = token(); refreshTokens.add(rt); set.push(`rt=${rt}; Path=/session; HttpOnly; SameSite=Strict`); }
        send(res, 303, '', { location: url.searchParams.get('next') ?? '/reports?page=1', 'set-cookie': set }); return;
      }
      if (path === '/session/refresh' && req.method === 'POST') {
        if (!refreshTokens.delete(jar.rt ?? '')) { send(res, 401, page('Unauthorized', '<p>Refresh token invalid</p>')); return; }
        stats.refreshes++; const id = token(), rt = token(); sessions.set(id, { uses: 0 }); refreshTokens.add(rt);
        send(res, 204, '', { 'set-cookie': [`sid=${id}; Path=/; HttpOnly; SameSite=Lax`, `rt=${rt}; Path=/session; HttpOnly; SameSite=Strict`] }); return;
      }
      if (path === '/reports') {
        stats.html++; const n = Number(url.searchParams.get('page') ?? '1'); const session = sessions.get(jar.sid ?? '');
        if (!session) { send(res, 302, '', { location: `/login?next=${encodeURIComponent(url.pathname + url.search)}` }); return; }
        // An expired session: the expiry variant answers with the login form and a 200, as many portals do; refresh redirects.
        if (session.uses >= sessionLimit) { sessions.delete(jar.sid!); if (variant === 'session-expiry') loginPage(res); else send(res, 302, '', { location: `/login?next=${encodeURIComponent(url.pathname + url.search)}` }); return; }
        session.uses++; stats.maxSessionUses = Math.max(stats.maxSessionUses, session.uses);
        send(res, 200, page(`Reports ${n}`, `<main id="records" data-auth="${portalUser.user}" data-page="${n}">${reports.slice((n - 1) * 20, n * 20).map(r => article(r)).join('')}</main>${n < 5 ? `<a class="next" href="/reports?page=${n + 1}">Next</a>` : ''}<a href="/logout">Sign out</a>`)); return;
      }
      if (path === '/' ) { stats.html++; send(res, 302, '', { location: '/reports?page=1' }); return; }
    }
    if ((routingVariants as readonly string[]).includes(variant)) {
      const n = Math.max(1, Math.min(5, Number(url.searchParams.get('page') ?? '1')));
      if (path === '/list') {
        stats.html++;
        if (variant === 'static') { send(res, 200, page(`List ${n}`, `<main id="records" data-total="100">${pageOf(n).map(article).join('')}</main>${nextLink(n)}`)); return; }
        if (variant === 'js-shell') { send(res, 200, shell(`List ${n}`, `fetch('/api/list?page=${n}').then(r=>r.json()).then(d=>{render(d.items);if(d.next){const a=document.createElement('a');a.className='next';a.href='/list?page='+d.next;a.textContent='Next';document.body.append(a)}})`)); return; }
        if (variant === 'partial-hydration') { send(res, 200, shell(`List ${n}`, `fetch('/api/list?page=${n}&offset=8').then(r=>r.json()).then(d=>render(d.items))`, pageOf(n).slice(0, 8).map(article).join(''), nextLink(n))); return; }
        if (variant === 'stale-html') { send(res, 200, shell(`List ${n}`, `fetch('/api/prices?page=${n}').then(r=>r.json()).then(d=>{for(const p of d.prices){const a=main.querySelector('[data-id="'+p.id+'"]');if(a)a.dataset.price=p.price}})`, pageOf(n).map(r => article({ ...r, price: stalePrice(r) })).join(''), nextLink(n))); return; }
      }
      if (path === '/api/list') { const offset = Number(url.searchParams.get('offset') ?? '0'); json(res, { items: pageOf(n).slice(offset), next: n < 5 ? n + 1 : null }); return; }
      if (path === '/api/prices') { json(res, { prices: pageOf(n).map(r => ({ id: r.id, price: r.price })) }); return; }
      if (path === '/catalog' && variant === 'graphql') {
        stats.html++;
        send(res, 200, shell('Catalog', `const more=document.querySelector('button.more');let after=null;const load=()=>fetch('/graphql',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({query:'query Products($after:String){products(first:20,after:$after){nodes{id name price} pageInfo{endCursor hasNextPage}}}',variables:{after}})}).then(r=>r.json()).then(d=>{const p=d.data.products;render(p.nodes);after=p.pageInfo.endCursor;more.hidden=!p.pageInfo.hasNextPage});more.addEventListener('click',load);load()`, '', '<button class="more" hidden>Load more</button>')); return;
      }
      if (path === '/graphql' && req.method === 'POST' && variant === 'graphql') {
        const { variables } = JSON.parse(await readBody(req) || '{}') as { variables?: { after?: string | null } }; const start = indexOf(variables?.after); const nodes = listing.slice(start, start + 20);
        json(res, { data: { products: { nodes, pageInfo: { endCursor: cursorOf(start + nodes.length), hasNextPage: start + nodes.length < listing.length } } } }); return;
      }
      if (path === '/feed' && variant === 'infinite-scroll') {
        stats.html++;
        send(res, 200, shell('Feed', `let cursor=main.dataset.cursor,busy=false;const more=()=>{if(busy||!cursor||innerHeight+scrollY<document.body.scrollHeight-200)return;busy=true;fetch('/api/feed?cursor='+cursor).then(r=>r.json()).then(d=>{render(d.items);cursor=d.nextCursor;main.dataset.cursor=cursor??'';busy=false;more()})};addEventListener('scroll',more)`, listing.slice(0, 20).map(article).join('')).replace('data-total="100"', `data-total="100" data-cursor="${cursorOf(20)}"`)); return;
      }
      if (path === '/api/feed' && variant === 'infinite-scroll') { const start = indexOf(url.searchParams.get('cursor')); const items = listing.slice(start, start + 20); json(res, { items, nextCursor: start + items.length < listing.length ? cursorOf(start + items.length) : null }); return; }
    }
    if (variant === 'crawler-trap') {
      stats.html++;
      if (path === '/shop') { send(res, 200, page('Shop', `<nav>${['a', 'b', 'c'].map(c => `<a href="/shop/c/${c}?${sid()}">Category ${c}</a><a href="/shop/c/${c}?utm_source=nav&${sid()}">${c}</a>`).join('')}<a href="/events?month=2026-01&${sid()}">Events</a></nav>`)); return; }
      const category = path.match(/^\/shop\/c\/([abc])$/)?.[1];
      if (category) {
        const items = products.filter(p => p.category === category); const sort = url.searchParams.get('sort'), order = url.searchParams.get('order');
        const sorted = sort === 'price' ? [...items].sort((a, b) => a.price - b.price) : sort === 'name' ? [...items].sort((a, b) => b.id.localeCompare(a.id)) : items;
        const listed = order === 'desc' ? [...sorted].reverse() : sorted;
        send(res, 200, page(`Category ${category}`, `<ul>${listed.map(p => `<li><a href="/p/${p.id}?${sid()}&utm_source=cat">${p.id}</a></li>`).join('')}</ul><nav>${['price', 'name'].flatMap(s => ['asc', 'desc'].map(o => `<a href="/shop/c/${category}?sort=${s}&order=${o}&${sid()}">${s} ${o}</a>`)).join('')}</nav><a href="/shop?${sid()}">Shop</a>`)); return;
      }
      const product = products.find(p => path === `/p/${p.id}`);
      if (product) { const related = products.filter(p => p.category === product.category && p.id !== product.id).slice(0, 3); send(res, 200, page(product.id, `${article(product)}<aside>${related.map(p => `<a href="/p/${p.id}?${sid()}">${p.id}</a>`).join('')}</aside><a href="/shop/c/${product.category}?${sid()}">Back</a>`)); return; }
      if (path === '/events') {
        const month = /^\d{4}-\d{2}$/.test(url.searchParams.get('month') ?? '') ? url.searchParams.get('month')! : '2026-01';
        send(res, 200, page(`Events ${month}`, `<main>${events.filter(e => e.month === month).map(article).join('')}</main><a href="/events?month=${monthAfter(month, -1)}&${sid()}">Previous</a><a href="/events?month=${monthAfter(month, 1)}&${sid()}">Next</a>`)); return;
      }
    }
    if (variant === 'large-table' && path === '/orders') {
      stats.html++;
      const after = url.searchParams.get('after'); const n = Math.max(1, Number(url.searchParams.get('page') ?? '1'));
      const start = after ? rows.findIndex(r => r.n < Number(after.slice(1))) : (n - 1) * 50; const slice = start < 0 ? [] : rows.slice(start, start + 50);
      const head = slice.length === 50 && start + 50 < rows.length ? `<link rel="next" href="/orders?after=${slice.at(-1)!.id}">` : '';
      const body = page(`Orders`, `<main id="records">${slice.map(article).join('')}</main>${!after && (n * 50 < rows.length) ? `<a class="next" href="/orders?page=${n + 1}">Next</a>` : ''}`, head);
      mutate(Math.max(0, start)); send(res, 200, body); return;
    }
    stats.html++; send(res, 404, page('Not found', '<p>Not found</p>'));
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  const startPath = (sessionVariants as readonly string[]).includes(variant) ? '/reports?page=1' : variant === 'graphql' ? '/catalog' : variant === 'infinite-scroll' ? '/feed' : variant === 'crawler-trap' ? '/shop' : variant === 'large-table' ? '/orders?page=1' : '/list?page=1';
  return {
    origin, startUrl: origin + startPath, stats,
    // Revocation while the client is down (resume-revoked): every live session and refresh token stops working.
    revokeAll() { sessions.clear(); refreshTokens.clear(); },
    // The client's output against the truth. The large table only requires rows present for the whole crawl, exactly once.
    verify(output: PortalRecord[]): PortalVerdict {
      const seen = new Map<string, number>(); for (const r of output) seen.set(r.id, (seen.get(r.id) ?? 0) + 1);
      const duplicates = [...seen.values()].filter(count => count > 1).length; const reasons: string[] = [];
      let missing = 0, unexpected = 0, wrongPrice = 0;
      if (variant === 'large-table') {
        const live = new Map(rows.map(r => [r.id, r.price]));
        for (const id of initial) if (!deleted.has(id) && !seen.has(id)) missing++;
        for (const r of output) { if (!live.has(r.id) && !deleted.has(r.id)) unexpected++; else if (live.has(r.id) && live.get(r.id) !== r.price) wrongPrice++; }
      } else {
        const expected = new Map(truth().map(r => [r.id, r.price]));
        for (const id of expected.keys()) if (!seen.has(id)) missing++;
        for (const r of output) { if (!expected.has(r.id)) unexpected++; else if (expected.get(r.id) !== r.price) wrongPrice++; }
      }
      if (missing) reasons.push(`${missing} record(s) missing`); if (unexpected) reasons.push(`${unexpected} unexpected record(s)`);
      if (duplicates) reasons.push(`${duplicates} duplicated id(s)`); if (wrongPrice) reasons.push(`${wrongPrice} record(s) with a stale or wrong price`);
      return { exact: !missing && !unexpected && !duplicates && !wrongPrice, missing, unexpected, duplicates, wrongPrice, reasons };
    },
    close: () => { server.closeAllConnections(); return new Promise<void>(resolve => server.close(() => resolve())); }
  };
}
export type PortalSite = Awaited<ReturnType<typeof startPortalSite>>;
