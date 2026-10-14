// Localhost-only experiment for rnd/research/2026-10-14-driver-interop.md.
// An "owner" raw CDP client (standing in for the Goodfetch engine: no Runtime
// domain, isolated world only) launches headless Chrome on a temporary profile,
// then Playwright connectOverCDP, Puppeteer connect and raw second clients attach
// to the same browser. Every request stays on 127.0.0.1.
//
// Run: LAB_NODE_MODULES=<rnd/lab/adapters/node/node_modules> node experiment.mjs [path-to-chrome]
// (no install: Playwright 1.63.0, puppeteer-core 25.12.0 and Stagehand 4.1.0 from the lab)
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtempSync, readFileSync, readdirSync, existsSync, rmSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

// The lab's installed packages (rnd/lab: pnpm install). Override with LAB_NODE_MODULES.
const LAB = process.env.LAB_NODE_MODULES || new URL('../../lab/adapters/node/node_modules', import.meta.url).pathname;
const { chromium } = await import(`${LAB}/playwright/index.mjs`);
const puppeteer = (await import(`${LAB}/puppeteer-core/lib/puppeteer/puppeteer-core.js`)).default;
const STAGEHAND_EXT = process.env.STAGEHAND_EXTENSION_DIR || `${LAB}/@browserbasehq/stagehand/dist/extension`;
const CHROME = process.argv[2] || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

const out = (step, data) => console.log(JSON.stringify({ step, ...data }));

// ---------- local HTTP server ----------
const hits = [];
const server = createServer((req, res) => {
  hits.push(req.url);
  if (req.url.startsWith('/dl')) {
    const tag = (req.url.split('?')[1] || 'x').replace(/[^a-z0-9-]/gi, '');
    res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-disposition': `attachment; filename="report-${tag}.txt"` });
    return res.end('hello');
  }
  if (req.url.startsWith('/worker.js')) { res.writeHead(200, { 'content-type': 'text/javascript' }); return res.end("fetch('/ran/worker')"); }
  if (req.url.startsWith('/with-frame')) { res.writeHead(200, { 'content-type': 'text/html' }); return res.end(`<!doctype html><iframe src="http://localhost:${server.address().port}/frame"></iframe><script>new Worker('/worker.js')</script>`); }
  res.writeHead(200, { 'content-type': 'text/html' });
  res.end(`<!doctype html><title>t</title><body data-path="${req.url}">
<script>
  fetch('/ran' + location.pathname).catch(() => {});
  // Proxy-prototype console preview probe (needs the Runtime domain on in some client).
  let traps = 0;
  const p = new Proxy({}, { ownKeys(t) { traps++; return Reflect.ownKeys(t); },
    getOwnPropertyDescriptor(t, k) { traps++; return Reflect.getOwnPropertyDescriptor(t, k); },
    get(t, k, r) { if (typeof k === 'string') traps++; return Reflect.get(t, k, r); } });
  setInterval(() => { console.debug(Object.create(p)); document.body.dataset.traps = String(traps); }, 100);
</script></body>`);
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const ORIGIN = `http://127.0.0.1:${server.address().port}`;

// ---------- minimal raw CDP client (flat sessions) ----------
class Raw {
  static async open(url) {
    const c = new Raw(); c.ws = new WebSocket(url); c.id = 0; c.pending = new Map(); c.handlers = [];
    c.closed = new Promise(r => c.ws.addEventListener('close', () => r(true)));
    c.ws.addEventListener('message', e => {
      const m = JSON.parse(e.data);
      if (m.id && c.pending.has(m.id)) { const p = c.pending.get(m.id); c.pending.delete(m.id); m.error ? p.rej(new Error(m.error.message)) : p.res(m.result); }
      else for (const h of c.handlers) h(m);
    });
    await new Promise((r, j) => { c.ws.addEventListener('open', r); c.ws.addEventListener('error', j); });
    return c;
  }
  send(method, params = {}, sessionId) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    return new Promise((res, rej) => this.pending.set(id, { res, rej }));
  }
  on(fn) { this.handlers.push(fn); }
  close() { this.ws.close(); }
}
const withTimeout = (p, ms, label = 'timeout') => Promise.race([p, sleep(ms).then(() => label)]);

// ---------- launch Chrome ----------
async function launch(extraArgs = []) {
  const dir = mkdtempSync(join(tmpdir(), 'interop-profile-'));
  const defaultDl = join(dir, 'default-downloads');
  mkdirSync(join(dir, 'Default'), { recursive: true }); mkdirSync(defaultDl);
  writeFileSync(join(dir, 'Default', 'Preferences'), JSON.stringify({ download: { default_directory: defaultDl, prompt_for_download: false, directory_upgrade: true } }));
  const proc = spawn(CHROME, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${dir}`,
    '--no-first-run', '--no-default-browser-check', ...extraArgs, 'about:blank'], { stdio: 'ignore' });
  let port;
  for (let i = 0; i < 100 && !port; i++) {
    await sleep(100);
    const f = join(dir, 'DevToolsActivePort');
    if (existsSync(f)) { const [p, path] = readFileSync(f, 'utf8').split('\n'); if (path) port = { p, path }; }
  }
  const wsUrl = `ws://127.0.0.1:${port.p}${port.path}`;
  return { dir, defaultDl, proc, wsUrl, httpUrl: `http://127.0.0.1:${port.p}` };
}

const b = await launch();
const owner = await Raw.open(b.wsUrl);
const version = await owner.send('Browser.getVersion');
out('versions', { chrome: version.product, protocol: version.protocolVersion, playwright: (await import(`${LAB}/playwright/package.json`, { with: { type: 'json' } })).default.version, puppeteer: (await import(`${LAB}/puppeteer-core/package.json`, { with: { type: 'json' } })).default.version, node: process.version });

// Owner page: created and attached by the owner, no Runtime.enable.
const { targetId } = await owner.send('Target.createTarget', { url: `${ORIGIN}/owner` });
const { sessionId: os } = await owner.send('Target.attachToTarget', { targetId, flatten: true });
await owner.send('Page.enable', {}, os);
const { frameTree } = await owner.send('Page.getFrameTree', {}, os);
async function ownerEval(expr) {
  const { executionContextId } = await owner.send('Page.createIsolatedWorld', { frameId: frameTree.frame.id, worldName: 'owner_probe' }, os);
  const r = await owner.send('Runtime.evaluate', { expression: expr, contextId: executionContextId, returnByValue: true }, os);
  return r.result.value;
}
const probe = () => ownerEval(`({w: innerWidth, h: innerHeight, traps: Number(document.body?.dataset.traps || 0), focus: document.hasFocus(), path: document.body?.dataset.path})`);
const ownerDl = mkdtempSync(join(tmpdir(), 'owner-dl-'));
const setOwnerDownloads = () => owner.send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: ownerDl, eventsEnabled: true });
await setOwnerDownloads();
await owner.send('Emulation.setDeviceMetricsOverride', { width: 1111, height: 777, deviceScaleFactor: 1, mobile: false }, os);
const dlEvents = [];
owner.on(m => { if (m.method === 'Browser.downloadProgress' && m.params.state !== 'inProgress') dlEvents.push(m.params.state); if (m.method === 'Browser.downloadWillBegin') dlEvents.push('begin:' + m.params.suggestedFilename); });
function where(name) {
  const places = { ownerDir: ownerDl, chromeDefaultDir: b.defaultDl };
  for (const d of readdirSync(tmpdir()).filter(n => n.startsWith('playwright-artifacts-'))) places['playwright:' + d] = join(tmpdir(), d);
  const found = [];
  for (const [k, d] of Object.entries(places)) { try { for (const f of readdirSync(d)) if (f === name || readFileSync(join(d, f), 'utf8') === 'hello') found.push(k + '/' + f); } catch {} }
  return found;
}
async function ownerDownload(tag) {
  dlEvents.length = 0;
  for (const d of [ownerDl, b.defaultDl]) for (const f of readdirSync(d)) rmSync(join(d, f), { force: true });
  await owner.send('Page.navigate', { url: `${ORIGIN}/dl?${tag}` }, os);
  await sleep(1500);
  const found = where(`report-${tag}.txt`);
  return { landedIn: found, inOwnerDir: found.some(f => f.startsWith('ownerDir/')), ownerEvents: [...dlEvents] };
}
await sleep(800);
out('P1 owner only', { probe: await probe(), download: await ownerDownload('p1') });

// ---------- P2: Playwright connectOverCDP, default options ----------
const { browserContextId: ownerCtx } = await owner.send('Target.createBrowserContext', {});
const { targetId: isolatedTarget } = await owner.send('Target.createTarget', { url: `${ORIGIN}/owner-isolated-context`, browserContextId: ownerCtx });
await sleep(500);
{
  const pw = await chromium.connectOverCDP(b.wsUrl);
  const ctxs = pw.contexts();
  const pages = ctxs.flatMap(c => c.pages());
  await sleep(800);
  const pr = await probe();
  const dl = await ownerDownload('p2');
  // Where did the download go? Ask Playwright's side too.
  out('P2 playwright attached (defaults)', { contexts: ctxs.length, pagesSeen: pages.map(p => p.url()), probe: pr, download: dl });
  await owner.send('Target.closeTarget', { targetId: isolatedTarget }); await owner.send('Target.disposeBrowserContext', { browserContextId: ownerCtx });
  await pw.close();
  await sleep(500);
  let alive = true; try { await withTimeout(owner.send('Browser.getVersion'), 2000); } catch { alive = false; }
  await owner.send('Page.navigate', { url: `${ORIGIN}/owner` }, os); await sleep(800);
  out('P2 after playwright browser.close()', { browserAlive: alive, probe: await probe(), download: await ownerDownload('p2-after') });
}

// ---------- P3: Playwright connectOverCDP with noDefaults ----------
{
  await setOwnerDownloads();
  await owner.send('Page.navigate', { url: `${ORIGIN}/owner` }, os); await sleep(500);
  const pw = await chromium.connectOverCDP(b.wsUrl, { noDefaults: true });
  await sleep(500);
  const dl = await ownerDownload('p3');
  await owner.send('Page.navigate', { url: `${ORIGIN}/owner` }, os); await sleep(800);
  out('P3 playwright attached (noDefaults)', { probe: await probe(), download: dl });
  await pw.close(); await sleep(300);
}

// ---------- P4: Puppeteer connect, defaults, then disconnect ----------
{
  await setOwnerDownloads();
  await owner.send('Emulation.setDeviceMetricsOverride', { width: 1111, height: 777, deviceScaleFactor: 1, mobile: false }, os);
  const pp = await puppeteer.connect({ browserWSEndpoint: b.wsUrl });
  const pages = await pp.pages();
  await sleep(800);
  out('P4 puppeteer attached (defaults)', { pagesSeen: pages.map(p => p.url()), probe: await probe() });
  await pp.disconnect(); await sleep(500);
  out('P4 after puppeteer disconnect()', { probe: await probe(), download: await ownerDownload('p4-after') });
  await owner.send('Page.navigate', { url: `${ORIGIN}/owner` }, os); await sleep(500);
}

// ---------- P5: Fetch interception chain, owner first, lent raw client rewrites ----------
{
  await owner.send('Emulation.setDeviceMetricsOverride', { width: 1111, height: 777, deviceScaleFactor: 1, mobile: false }, os);
  const seenByOwner = [];
  owner.on(m => { if (m.method === 'Fetch.requestPaused' && m.sessionId === os) { seenByOwner.push(m.params.request.url); owner.send('Fetch.continueRequest', { requestId: m.params.requestId }, os).catch(() => {}); } });
  await owner.send('Fetch.enable', { patterns: [{ urlPattern: `${ORIGIN}/fetch*` }] }, os);
  const lent = await Raw.open(b.wsUrl);
  const { sessionId: ls } = await lent.send('Target.attachToTarget', { targetId, flatten: true });
  const seenByLent = [];
  lent.on(m => { if (m.method === 'Fetch.requestPaused' && m.sessionId === ls) { seenByLent.push(m.params.request.url); const u = m.params.request.url.replace('/fetch-a', '/fetch-b-rewritten'); lent.send('Fetch.continueRequest', { requestId: m.params.requestId, url: u }, ls).catch(() => {}); } });
  await lent.send('Fetch.enable', { patterns: [{ urlPattern: `${ORIGIN}/fetch*` }] }, ls);
  hits.length = 0;
  await owner.send('Page.navigate', { url: `${ORIGIN}/fetch-a` }, os); await sleep(1200);
  out('P5 fetch chain (owner enabled first)', { seenByOwner, seenByLent, serverReceived: hits.filter(h => h.startsWith('/fetch')), pageBodyPath: (await probe()).path });
  // Lent client stops continuing: does the owner's navigation stall?
  lent.handlers = [];
  lent.on(() => {});
  hits.length = 0; seenByOwner.length = 0;
  const nav = owner.send('Page.navigate', { url: `${ORIGIN}/fetch-c` }, os);
  await sleep(2500);
  out('P5b lent client never continues', { seenByOwner, serverReceived: hits.filter(h => h.startsWith('/fetch')) });
  lent.close(); await sleep(800);
  await nav.catch(() => {});
  out('P5c after lent client disconnects', { serverReceived: hits.filter(h => h.startsWith('/fetch')), seenByOwner });
  // P5d: lent enables, then the owner sends Fetch.enable again (e.g. to change its patterns).
  const lent2 = await Raw.open(b.wsUrl);
  const { sessionId: ls2 } = await lent2.send('Target.attachToTarget', { targetId, flatten: true });
  const seenByLent2 = [];
  lent2.on(m => { if (m.method === 'Fetch.requestPaused' && m.sessionId === ls2) { seenByLent2.push(m.params.request.url); lent2.send('Fetch.continueRequest', { requestId: m.params.requestId, url: m.params.request.url.replace('/fetch-d', '/fetch-d-rewritten') }, ls2).catch(() => {}); } });
  await lent2.send('Fetch.enable', { patterns: [{ urlPattern: `${ORIGIN}/fetch*` }] }, ls2);
  await owner.send('Fetch.enable', { patterns: [{ urlPattern: `${ORIGIN}/fetch*` }] }, os);
  hits.length = 0; seenByOwner.length = 0;
  await owner.send('Page.navigate', { url: `${ORIGIN}/fetch-d` }, os); await sleep(1200);
  out('P5d owner re-sends Fetch.enable after the lent client', { seenByOwner, seenByLent: seenByLent2, serverReceived: hits.filter(h => h.startsWith('/fetch')) });
  lent2.close(); await sleep(300);
  await owner.send('Fetch.disable', {}, os);
  // P5e: lent enables first, the owner enables after (fresh interceptor).
  const lent3 = await Raw.open(b.wsUrl);
  const { sessionId: ls3 } = await lent3.send('Target.attachToTarget', { targetId, flatten: true });
  const seenByLent3 = [];
  lent3.on(m => { if (m.method === 'Fetch.requestPaused' && m.sessionId === ls3) { seenByLent3.push(m.params.request.url); lent3.send('Fetch.continueRequest', { requestId: m.params.requestId, url: m.params.request.url.replace('/fetch-e', '/fetch-e-rewritten') }, ls3).catch(() => {}); } });
  await lent3.send('Fetch.enable', { patterns: [{ urlPattern: `${ORIGIN}/fetch*` }] }, ls3);
  await owner.send('Fetch.enable', { patterns: [{ urlPattern: `${ORIGIN}/fetch*` }] }, os);
  hits.length = 0; seenByOwner.length = 0;
  await owner.send('Page.navigate', { url: `${ORIGIN}/fetch-e` }, os); await sleep(1200);
  out('P5e lent client enabled Fetch before the owner', { seenByOwner, seenByLent: seenByLent3, serverReceived: hits.filter(h => h.startsWith('/fetch')) });
  lent3.close(); await sleep(300);
  await owner.send('Fetch.disable', {}, os);
}

// ---------- P6: waitForDebuggerOnStart held by a lent client ----------
{
  const lent = await Raw.open(b.wsUrl);
  await lent.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true });
  hits.length = 0;
  const t0 = Date.now();
  const { targetId: t2 } = await owner.send('Target.createTarget', { url: `${ORIGIN}/new-tab` });
  await sleep(2500);
  const whilePaused = hits.filter(h => h.includes('new-tab'));
  lent.close(); await sleep(1500);
  out('P6 new tab while a lent client pauses new targets and never resumes', { requestsWhilePaused: whilePaused, requestsAfterLentDisconnect: hits.filter(h => h.includes('new-tab')), ms: Date.now() - t0 });
  await owner.send('Target.closeTarget', { targetId: t2 });
}

// ---------- P6b: page-level waitForDebuggerOnStart held by a lent client (OOPIF, worker) ----------
{
  const lent = await Raw.open(b.wsUrl);
  const { sessionId: ls } = await lent.send('Target.attachToTarget', { targetId, flatten: true });
  await lent.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }, ls);
  const attachedTypes = [];
  lent.on(m => { if (m.method === 'Target.attachedToTarget') attachedTypes.push(m.params.targetInfo.type + (m.params.waitingForDebugger ? ' (paused)' : '')); });
  hits.length = 0;
  await owner.send('Page.navigate', { url: `${ORIGIN}/with-frame` }, os); await sleep(2500);
  const whilePaused = hits.filter(h => h.startsWith('/ran') || h.startsWith('/frame') || h.startsWith('/worker'));
  lent.close(); await sleep(1500);
  out('P6b OOPIF and worker while a lent page-level client pauses them', { lentAttached: attachedTypes, requestsWhilePaused: whilePaused, requestsAfterLentDisconnect: hits.filter(h => h.startsWith('/ran') || h.startsWith('/frame') || h.startsWith('/worker')) });
}

// ---------- P7: Puppeteer browser.close() on a connected browser ----------
{
  const pp = await puppeteer.connect({ browserWSEndpoint: b.wsUrl, defaultViewport: null });
  await pp.close();
  const closed = await withTimeout(owner.closed, 3000, 'still-open');
  await sleep(1500);
  out('P7 puppeteer browser.close()', { ownerSocket: closed === true ? 'closed' : closed, chromeExitCode: b.proc.exitCode, chromeKilled: b.proc.signalCode });
}

// ---------- P8: Stagehand v4's extension on a browser launched without / with the unsafe flag ----------
for (const flag of [false, true]) {
  const c = await launch(flag ? ['--enable-unsafe-extension-debugging'] : []);
  const cl = await Raw.open(c.wsUrl);
  const res = {};
  try { res.getExtensions = (await cl.send('Extensions.getExtensions')).extensions.length; } catch (e) { res.getExtensions = 'error: ' + e.message; }
  try { res.loadUnpacked = (await cl.send('Extensions.loadUnpacked', { path: STAGEHAND_EXT })).id; } catch (e) { res.loadUnpacked = 'error: ' + e.message; }
  await sleep(1500);
  const all = (await cl.send('Target.getTargets')).targetInfos;
  res.extensionTargets = all.filter(t => t.url.startsWith('chrome-extension://')).map(t => t.type + ' ' + t.url.replace(/chrome-extension:\/\/[a-z]+/, m => m === 'chrome-extension://' + res.loadUnpacked ? 'chrome-extension://<stagehand>' : 'chrome-extension://<other>'));
  const sw = all.find(t => t.type === 'service_worker' && typeof res.loadUnpacked === 'string' && t.url.startsWith(`chrome-extension://${res.loadUnpacked}/`));
  if (sw) {
    try {
      const { sessionId } = await cl.send('Target.attachToTarget', { targetId: sw.targetId, flatten: true });
      res.attachToStagehandWorker = 'ok';
      try { res.evaluateInWorker = (await withTimeout(cl.send('Runtime.evaluate', { expression: 'typeof chrome.debugger', returnByValue: true }, sessionId), 3000)).result?.value ?? 'timeout'; } catch (e) { res.evaluateInWorker = 'error: ' + e.message; }
    } catch (e) { res.attachToStagehandWorker = 'error: ' + e.message; }
  } else res.attachToStagehandWorker = 'no stagehand service worker found';
  out(`P8 stagehand extension, --enable-unsafe-extension-debugging=${flag}`, res);
  cl.close(); c.proc.kill(); await sleep(300); rmSync(c.dir, { recursive: true, force: true });
}

server.close();
try { b.proc.kill(); } catch {}
await sleep(300);
rmSync(b.dir, { recursive: true, force: true }); rmSync(ownerDl, { recursive: true, force: true });
process.exit(0);
