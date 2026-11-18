// The six things the page crate asks of CDP that WebDriver BiDi has no command for (issue 2460), each tried on the
// installed Firefox over BiDi: what another way gives, shown by this run, or that the operation is unavailable.
// Usage: node adapters/firefox-bidi/probes.ts <firefox>; prints one JSON object per probe.
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { createHash } from 'node:crypto';
import type { AddressInfo, Socket } from 'node:net';
import { Bidi, fromRemote } from './bidi.ts';
import { callInSandbox, SANDBOX } from './guard.ts';

const pages: Record<string, { type: string; body: string }> = {
  '/prepare': { type: 'text/html', body: '<!doctype html><script>document.title="page script ran"</script><iframe src="/frame"></iframe><output id=worker></output><script>const worker=new Worker("/worker.js");worker.onmessage=e=>{document.querySelector("#worker").textContent=e.data}</script>' },
  '/frame': { type: 'text/html', body: '<!doctype html><script>document.title="frame script ran"</script>' },
  '/worker.js': { type: 'text/javascript', body: 'fetch("/from-worker").then(()=>postMessage("worker ran"))' },
  '/from-worker': { type: 'text/plain', body: 'ok' },
  '/big': { type: 'text/plain', body: 'goodfetch '.repeat(200_000) },
  '/hide': { type: 'text/html', body: '<!doctype html><div id=ad>ad</div><output id=seen></output><script>setInterval(()=>{document.querySelector("#seen").textContent=document.adoptedStyleSheets.length+"/"+document.styleSheets.length},20)</script>' },
  '/socket': { type: 'text/html', body: '<!doctype html><output id=echo></output><script>const socket=new WebSocket(`ws://${location.host}/echo`);socket.onopen=()=>socket.send("ping");socket.onmessage=e=>{document.querySelector("#echo").textContent=e.data}</script>' },
  '/worker-page': { type: 'text/html', body: '<!doctype html><output id=state></output><script>navigator.serviceWorker.register("/sw.js").then(()=>navigator.serviceWorker.ready).then(()=>{document.querySelector("#state").textContent=navigator.serviceWorker.controller?"controlled":"ready"})</script>' },
  '/sw.js': { type: 'text/javascript', body: 'self.addEventListener("install",()=>self.skipWaiting());self.addEventListener("activate",e=>e.waitUntil(clients.claim()));self.addEventListener("fetch",e=>{if(new URL(e.request.url).pathname==="/probe")e.respondWith(new Response("from-service-worker"))})' },
  '/probe': { type: 'text/plain', body: 'from-network' },
  '/focus': { type: 'text/html', body: '<!doctype html><title>focus</title>' }
};

function serve(req: IncomingMessage, res: ServerResponse) {
  const page = pages[new URL(req.url ?? '/', 'http://probe').pathname];
  if (!page) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'content-type': `${page.type}; charset=utf-8`, 'cache-control': 'no-store' }); res.end(page.body);
}

// A WebSocket echo of one text frame, enough for the page's one message.
function echo(req: IncomingMessage, socket: Socket) {
  const accept = createHash('sha1').update(`${String(req.headers['sec-websocket-key'])}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
  socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
  socket.on('data', (frame: Buffer) => {
    const length = frame[1]! & 0x7f; const mask = frame.subarray(2, 6); const payload = Buffer.from(frame.subarray(6, 6 + length).map((byte, index) => byte ^ mask[index % 4]!));
    if ((frame[0]! & 0x0f) === 1) socket.write(Buffer.concat([Buffer.from([0x81, payload.length]), payload]));
  });
}

// One probe, given 30 s: a command Firefox never answers is a result too.
async function probe(name: string, run: () => Promise<Record<string, unknown>>) {
  process.stderr.write(`${name}…\n`);
  let timer: NodeJS.Timeout | undefined;
  const timeout = new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('no answer within 30 s')), 30_000); });
  try { return { probe: name, ...await Promise.race([run(), timeout]) }; } catch (error) { return { probe: name, error: String(error) }; }
  finally { clearTimeout(timer); }
}

async function main(executablePath: string) {
  const server = createServer(serve); server.on('upgrade', echo);
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  const bidi = await Bidi.launch(executablePath);
  const tab = async () => String((await bidi.send('browsingContext.create', { type: 'tab' })).context);
  const read = (context: string, expression: string) => callInSandbox(bidi, context, `()=>${expression}`);
  const settle = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
  const results = [];
  try {
    // 1. Holding a new page, frame or worker until it is prepared: a preload script in the sandbox marks its window and
    // says on a channel what had run when it did. The frame's first document reuses the Window of its initial
    // about:blank (HTML's window reuse), so the frame is read afterwards: whether its sandbox holds the mark, and whether
    // the frame's script ran. A worker realm has no preload script.
    results.push(await probe('hold-new-page-frame-worker', async () => {
      const messages: Record<string, unknown>[] = []; const realms: string[] = []; const requests: { url: string; context: unknown }[] = [];
      const offMessage = bidi.on('script.message', event => messages.push({ context: (event.source as { context?: string }).context, data: fromRemote(event.data as Record<string, unknown>) }));
      const offRealm = bidi.on('script.realmCreated', event => realms.push(String(event.type)));
      const offRequest = bidi.on('network.beforeRequestSent', event => requests.push({ url: String((event.request as { url: string }).url).replace(origin, ''), context: event.context }));
      await bidi.send('session.subscribe', { events: ['script.message', 'script.realmCreated', 'network.beforeRequestSent'] });
      const { script } = await bidi.send('script.addPreloadScript', { functionDeclaration: '(channel)=>{window.prepared=true;channel({path:location.pathname,scripts:document.scripts.length,title:document.title,readyState:document.readyState})}', arguments: [{ type: 'channel', value: { channel: 'prepared' } }], sandbox: SANDBOX });
      const context = await tab();
      await bidi.send('browsingContext.navigate', { context, url: `${origin}/prepare`, wait: 'complete' }); await settle(500);
      await bidi.send('script.removePreloadScript', { script }); offMessage(); offRealm(); offRequest();
      const frame = String(((await bidi.send('browsingContext.getTree', { root: context })).contexts as { children: { context: string }[] }[])[0]!.children[0]!.context);
      const frameSandbox = await read(frame, '({path:location.pathname,prepared:window.prepared===true,frameScriptRan:document.title==="frame script ran"})');
      return { preloadMessages: messages, frameSandbox, realmTypes: realms, workerOutput: await read(context, 'document.querySelector("#worker").textContent'), requests: requests.filter(request => request.url !== '/favicon.ico') };
    }));
    // 2. A streamed read of a paused response's body: a data collector and `network.getData` while the response is
    // held at `responseStarted`, then once it completed.
    results.push(await probe('paused-response-body-stream', async () => {
      const steps: string[] = []; const started = performance.now(); const step = (text: string) => steps.push(`${Math.round(performance.now() - started)} ms ${text}`);
      const within = <T,>(promise: Promise<T>, ms: number) => Promise.race([promise.then(value => ({ value }), (error: unknown) => ({ error: String(error) })), new Promise<{ error: string }>(resolve => setTimeout(() => resolve({ error: `no answer within ${ms} ms` }), ms))]);
      const size = (data: { value?: Record<string, unknown>; error?: string }) => data.value ? { bytes: String((data.value.bytes as { value: string }).value).length } : { error: data.error };
      const { collector } = await bidi.send('network.addDataCollector', { dataTypes: ['response'], maxEncodedDataSize: 10_000_000 }); step('collector added');
      const { intercept } = await bidi.send('network.addIntercept', { phases: ['responseStarted'], urlPatterns: [{ type: 'string', pattern: `${origin}/big` }] }); step('intercept added');
      await bidi.send('session.subscribe', { events: ['network.responseStarted', 'network.responseCompleted'] });
      const context = await tab();
      const paused = bidi.next('network.responseStarted', event => event.isBlocked === true, 10_000);
      const completedEvents: string[] = []; const offCompleted = bidi.on('network.responseCompleted', event => completedEvents.push(String((event.request as { request: string }).request)));
      void bidi.send('browsingContext.navigate', { context, url: `${origin}/big`, wait: 'none' }).catch(() => {});
      const request = String(((await paused).request as { request: string }).request); step('response paused at responseStarted');
      const whilePaused = size(await within(bidi.send('network.getData', { dataType: 'response', request }), 5_000)); step(`getData while paused: ${JSON.stringify(whilePaused)}`);
      await within(bidi.send('network.continueResponse', { request }), 5_000); step('response continued');
      for (let waited = 0; !completedEvents.includes(request) && waited < 5_000; waited += 100) await settle(100);
      offCompleted(); step(completedEvents.includes(request) ? 'response completed' : 'no responseCompleted within 5 s');
      const afterCompletion = size(await within(bidi.send('network.getData', { dataType: 'response', request }), 5_000)); step('getData after completion');
      await bidi.send('network.removeIntercept', { intercept }); await bidi.send('network.removeDataCollector', { collector });
      return { whilePaused, afterCompletion, bodyBytes: pages['/big']!.body.length, steps };
    }));
    // 3. An inspector style sheet for live hiding: a constructed sheet adopted from the sandbox, and what the page sees.
    results.push(await probe('inspector-style-sheet', async () => {
      const context = await tab();
      await bidi.send('browsingContext.navigate', { context, url: `${origin}/hide`, wait: 'complete' });
      const before = await read(context, '({display:getComputedStyle(document.querySelector("#ad")).display,pageSees:document.querySelector("#seen").textContent})');
      await callInSandbox(bidi, context, '()=>{const sheet=new CSSStyleSheet();sheet.replaceSync("#ad{display:none !important}");document.adoptedStyleSheets=[...document.adoptedStyleSheets,sheet]}');
      await settle(100);
      return { before, after: await read(context, '({display:getComputedStyle(document.querySelector("#ad")).display,pageSees:document.querySelector("#seen").textContent})') };
    }));
    // 4. WebSocket frames: every network event the session gets while the page sends and receives one frame.
    results.push(await probe('websocket-frames', async () => {
      const events: string[] = []; const offs = ['network.beforeRequestSent', 'network.responseStarted', 'network.responseCompleted', 'network.fetchError'].map(name => bidi.on(name, event => { const url = String((event.request as { url?: string } | undefined)?.url ?? ''); if (url.includes('/echo')) events.push(`${name} ${url.replace(origin.replace('http', 'ws'), '').replace(origin, '')}`); }));
      await bidi.send('session.subscribe', { events: ['network'] });
      const context = await tab();
      await bidi.send('browsingContext.navigate', { context, url: `${origin}/socket`, wait: 'complete' }); await settle(500);
      for (const off of offs) off();
      return { pageGotEcho: await read(context, 'document.querySelector("#echo").textContent'), networkEventsForTheSocket: events };
    }));
    // 5. Bypassing service workers for a fetch: the page's worker answers /probe; a fetch from the sandbox, with the
    // cache behaviour set to bypass and without.
    results.push(await probe('fetch-bypassing-service-workers', async () => {
      const context = await tab();
      await bidi.send('browsingContext.navigate', { context, url: `${origin}/worker-page`, wait: 'complete' }); await settle(800);
      await bidi.send('browsingContext.reload', { context, wait: 'complete' }); await settle(800);
      const fetchProbe = () => read(context, 'fetch("/probe").then(response=>response.text())');
      const controlled = await read(context, 'navigator.serviceWorker.controller!==null');
      const sandboxFetch = await fetchProbe();
      await bidi.send('network.setCacheBehavior', { cacheBehavior: 'bypass' });
      const withCacheBypass = await fetchProbe();
      await bidi.send('network.setCacheBehavior', { cacheBehavior: 'default' });
      return { controlled, sandboxFetch, withCacheBypass };
    }));
    // 6. Focus emulation: whether two pages both report focus, before and after activating the second.
    results.push(await probe('focus-emulation', async () => {
      const first = await tab(); const second = await tab();
      for (const context of [first, second]) await bidi.send('browsingContext.navigate', { context, url: `${origin}/focus`, wait: 'complete' });
      const focus = async () => ({ first: await read(first, '({hasFocus:document.hasFocus(),visibility:document.visibilityState})'), second: await read(second, '({hasFocus:document.hasFocus(),visibility:document.visibilityState})') });
      const initially = await focus();
      await bidi.send('browsingContext.activate', { context: first });
      const afterActivatingFirst = await focus();
      return { initially, afterActivatingFirst };
    }));
  } finally { await bidi.close(); }
  // 6b. The same with `focusmanager.testmode`, Firefox's own pref for tests that lets every window keep focus.
  const testMode = await Bidi.launch(executablePath, { 'focusmanager.testmode': true });
  try {
    results.push(await probe('focus-emulation-testmode-pref', async () => {
      const open = async () => String((await testMode.send('browsingContext.create', { type: 'tab' })).context);
      const first = await open(); const second = await open();
      for (const context of [first, second]) await testMode.send('browsingContext.navigate', { context, url: `${origin}/focus`, wait: 'complete' });
      const state = (context: string) => callInSandbox(testMode, context, '()=>({hasFocus:document.hasFocus(),visibility:document.visibilityState})');
      return { first: await state(first), second: await state(second) };
    }));
  } finally { await testMode.close(); server.close(); }
  return { firefox: bidi.version, results };
}

if (process.argv[1]?.endsWith('probes.ts')) {
  const executablePath = process.argv[2];
  if (!executablePath) throw new Error('usage: probes.ts <firefox>');
  process.stdout.write(JSON.stringify(await main(executablePath), null, 2) + '\n');
}
