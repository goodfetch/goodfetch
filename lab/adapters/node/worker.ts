import { readFileSync } from 'node:fs';
import { mkdtemp, readFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import type { Browser } from 'playwright';
import type { Browser as PuppeteerBrowser, Page as PuppeteerPage, Frame as PuppeteerFrame, ElementHandle } from 'puppeteer-core';
import type { Stagehand, StagehandBrowser } from '@browserbasehq/stagehand';
import { serve, type Adapter, type AdapterOutput, type Task } from '../../src/worker-server.ts';
import { runBrowserTask, waitScript, actionReadinessScript,type BrowserPage } from './browser-tasks.ts';

const backend = process.argv[2] ?? 'node-http';
const require = createRequire(import.meta.url);
let browser: Browser | undefined, puppeteerBrowser: PuppeteerBrowser | undefined;
let stagehandBrowser: StagehandBrowser | undefined, stagehand: Stagehand | undefined;
let crawlee: typeof import('crawlee') | undefined;
let options: { executablePath?: string; headless?: boolean } = {}; let connected = false;
const capabilities = backend.includes('http') ? ['http-catalog'] : ['http-catalog', 'js-catalog', 'form', 'clicks','actionability', ...(backend === 'puppeteer' ? [] : ['shadow']), ...(['playwright', 'puppeteer'].includes(backend) ? ['primitives', 'navigation'] : [])];
function version(packageName: string) { return JSON.parse(readFileSync(require.resolve(`${packageName}/package.json`), 'utf8')).version as string; }
function config() { return new crawlee!.Configuration({ persistStorage: false, storageClient: new crawlee!.MemoryStorage({ persistStorage: false }) }); }
const adapter: Adapter = {
  info() { return { backend, protocol: 1, capabilities, runtime: process.version, maxConcurrency: backend === 'stagehand-batch' ? 1 : 128,
    lifecycle: backend === 'node-http' ? 'native fetch; shared process connection pool' : backend === 'crawlee-http' ? 'crawler per task' : backend === 'crawlee-playwright' ? 'crawler and browser per task' : 'browser per block; page per task',
    versions: { ...(backend.startsWith('crawlee') ? {crawlee:version('crawlee')} : {}), ...(['playwright','crawlee-playwright'].includes(backend) ? {playwright:version('playwright')} : {}), ...(backend.startsWith('stagehand') ? {stagehand:'4.1.0'} : {}), ...(backend==='puppeteer' ? {puppeteer:version('puppeteer-core')} : {}) } }; },
  async prepare(input) {
    // Lab scheduler options belong to the adapter, not to native launch schemas.
    // Chromium refuses its sandbox as root, the usual user in a Linux container; Playwright already launches without it.
    options = {executablePath:input.executablePath,headless:input.headless??true,...(process.platform==='linux'&&process.getuid?.()===0?{args:['--no-sandbox']}:{})};
    if (backend.startsWith('crawlee')) { crawlee = await import('crawlee'); crawlee.log.setLevel(crawlee.log.LEVELS.ERROR); }
    // A harness browser is shared by every driver of the campaign: connect to it instead of launching one.
    if (input.cdpEndpoint) {
      if (backend === 'playwright') { const {chromium}=await import('playwright'); browser = await chromium.connectOverCDP(input.cdpEndpoint); }
      else if (backend === 'puppeteer') { const {default:puppeteer}=await import('puppeteer-core'); puppeteerBrowser = await puppeteer.connect({ browserWSEndpoint: input.cdpEndpoint, defaultViewport: null }); }
      else throw new Error(`${backend} cannot drive a harness browser`);
      connected = true; return;
    }
    if (backend === 'playwright') { const {chromium}=await import('playwright'); browser = await chromium.launch(options); }
    if (backend === 'puppeteer') { const {default:puppeteer}=await import('puppeteer-core'); puppeteerBrowser = await puppeteer.launch(options); }
    if (backend.startsWith('stagehand')) {
      const {localBrowser,Stagehand}=await import('@browserbasehq/stagehand');
      stagehandBrowser = await localBrowser.launch({ ...options, viewport: { width: 1280, height: 720 } });
      stagehand = await Stagehand.create({ browser: stagehandBrowser, cache: false, selfHeal: false, logging: { level: 'error' } });
    }
  },
  async run(task): Promise<AdapterOutput> {
    if (!capabilities.includes(task.scenario)) return { status: 'unsupported', reason: `unsupported ${task.scenario}` };
    if (backend === 'node-http') {
      // The lab's TLS fixture is self-signed; fetch has no per-request switch, so the process trusts it for this task.
      if (task.insecureTls) process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0';
      const start = performance.now(); const records: { id: string; price: number }[] = []; let url: string | null = task.url; let visited = 0;
      while (url) {
        if (visited++ > task.pages) throw new Error('pagination exceeded contract');
        const response: globalThis.Response = await fetch(url, { signal: AbortSignal.timeout(task.timeoutMs) }); if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const html: string = await response.text();
        for (const match of html.matchAll(/<article data-id="([^"]+)" data-price="(\d+)"/g)) records.push({ id: match[1]!, price: Number(match[2]) });
        const next: RegExpMatchArray | null = html.match(/class="next" href="([^"]+)"/); url = next ? new URL(next[1]!, url).href : null;
      }
      return { status: 'ok', records, metrics: { taskMs: performance.now() - start, pages: visited } };
    }
    if (backend.startsWith('crawlee')) {
      const {CheerioCrawler,PlaywrightCrawler}=crawlee!;
      const start = performance.now(); const records: { id: string; price: number }[] = []; const failures: string[] = []; const taskMetrics:Record<string,number>={};
      if (backend === 'crawlee-http') {
        const crawler = new CheerioCrawler({ maxConcurrency: 1, maxRequestRetries: 0, maxRequestsPerCrawl: task.pages + 1, respectRobotsTxtFile: task.respectRobots === true, ignoreSslErrors: task.insecureTls === true,
          requestHandler: async ({ $, enqueueLinks }) => { $('article[data-id]').each((_, el) => { records.push({ id: $(el).attr('data-id')!, price: Number($(el).attr('data-price')) }); }); await enqueueLinks({ selector: 'a.next' }); },
          failedRequestHandler: async ({ request }) => { failures.push(request.url); }
        }, config());
        await crawler.run([task.url]);
      } else {
        const crawler = new PlaywrightCrawler({ maxConcurrency: 1, maxRequestRetries: 0, maxRequestsPerCrawl: task.pages + 1,
          launchContext: { launchOptions: { ...options, headless: true } }, browserPoolOptions: { useFingerprints: false },
          navigationTimeoutSecs: task.timeoutMs / 1000, requestHandlerTimeoutSecs: (task.timeoutMs+(task.scenario==='actionability'?3000:0)) / 1000,
          preNavigationHooks: [async (_, gotoOptions) => { gotoOptions.waitUntil = 'domcontentloaded'; }],
          requestHandler: async ({ page, enqueueLinks }) => {
            page.setDefaultTimeout(task.timeoutMs);
            if (task.scenario.endsWith('catalog')) { await page.waitForSelector('#catalog[data-ready="true"]'); records.push(...await page.locator('article[data-id]').evaluateAll(els => els.map(el => ({ id: el.getAttribute('data-id')!, price: Number(el.getAttribute('data-price')) })))); await enqueueLinks({ selector: 'a.next' }); }
            else Object.assign(taskMetrics,(await runBrowserTask(wrapPlaywright(page), task, true)).metrics);
          }, failedRequestHandler: async ({ request }) => { failures.push(request.url); }
        }, config());
        await crawler.run([task.url]);
      }
      if (failures.length) throw new Error(`crawler failed ${failures.length} requests`);
      return { status: 'ok', records, metrics: { ...taskMetrics, taskMs: performance.now() - start } };
    }
    if (browser) { const page = await browser.newPage({ viewport: { width: 1280, height: 720 } }); page.setDefaultTimeout(task.timeoutMs); try { return await runBrowserTask(wrapPlaywright(page), task); } finally { await page.close(); } }
    if (puppeteerBrowser) {
      const page = await puppeteerBrowser.newPage(); await page.setViewport({ width: 1280, height: 720 }); page.setDefaultTimeout(task.timeoutMs);
      const wrapped = wrapPuppeteer(page);
      try { return await runBrowserTask(wrapped, task); } finally { await page.close(); }
    }
    if (stagehand && stagehandBrowser) {
      const page = await stagehandBrowser.context.newPage();
      try { await page.setViewportSize(1280,720); } catch(error) { await page.close().catch(()=>{}); throw error; }
      if(backend==='stagehand-batch'&&task.scenario==='actionability'){
        try{
          await page.goto(task.url,{waitUntil:'domcontentloaded'});await page.evaluate(waitScript(`document.body.dataset.ready==='true'`,task.timeoutMs));await stagehandBrowser.context.setActivePage(page);
          const result=await stagehand.experimentalBatch(async(batch,input)=>{
            let nativeRejected=0;const began=performance.now();try{if(input.guarded)await batch.page.evaluate(input.guard);await batch.page.locator('#target').click()}catch(error){nativeRejected=1;if(!input.negative)throw error}
            const nativeClickMs=performance.now()-began;if(!input.negative)await batch.page.evaluate(input.postcondition);return {nativeRejected,nativeClickMs};
          },{guarded:task.actionPolicy==='guarded',negative:['never-enabled','never-uncovered','ambiguous'].includes(task.variant),guard:actionReadinessScript(task.timeoutMs),postcondition:waitScript(`document.querySelector('#action-saved').textContent==='saved'`,task.timeoutMs)});
          return {status:'ok',metrics:result};
        }finally{await page.close()}
      }
      if (backend === 'stagehand-batch' && task.scenario === 'clicks') {
        try {
          await page.goto(task.url, { waitUntil: 'domcontentloaded' });
          await page.evaluate(waitScript(`document.body.dataset.ready==='true' && !document.querySelector('#overlay')`, task.timeoutMs));
          await stagehandBrowser.context.setActivePage(page);
          const result = await stagehand.experimentalBatch(async (batch, input) => {
            const snapshot = await batch.page.evaluate<{nodes:number;width:number;height:number}>('({nodes:document.querySelectorAll("*").length,width:innerWidth,height:innerHeight})');
            if(snapshot.width!==1280||snapshot.height!==720) throw new Error('viewport contract');
            for (let i = 1; i <= input.actions; i++) {
              await batch.page.locator('#counter').click();
              await batch.page.evaluate(`(async()=>{const end=performance.now()+${input.timeoutMs};while(document.querySelector('#count').textContent!=='${i}'){if(performance.now()>end)throw new Error('postcondition timeout');await new Promise(r=>setTimeout(r,8))}})()`);
            }
            return snapshot;
          }, { actions: task.actions, timeoutMs: task.timeoutMs });
          return { status: 'ok', metrics: { operationCount: task.actions * 2 + 3, domNodes:result.nodes, viewportWidth:result.width, viewportHeight:result.height } };
        } finally { await page.close(); }
      }
      const wrapped: BrowserPage = { goto: url => page.goto(url, { waitUntil: 'domcontentloaded' }), evaluate: async <T>(script: string) => await page.evaluate(script) as T,
        click: async selector => { await page.locator(selector).click(); },
        fill: (selector, value) => page.locator(selector).fill(value), select: async (selector, value) => { await page.locator(selector).selectOption(value); }, close: () => page.close() };
      try { return await runBrowserTask(wrapped, task); } finally { await page.close(); }
    }
    throw new Error('adapter not prepared');
  },
  // Playwright's close on a CDP connection only disconnects; Puppeteer needs an explicit disconnect.
  async close() { await stagehand?.close(); await stagehandBrowser?.close(); await browser?.close(); if (connected) await puppeteerBrowser?.disconnect(); else await puppeteerBrowser?.close(); }
};
function wrapPlaywright(page: import('playwright').Page): BrowserPage {
  return { goto: url => page.goto(url, { waitUntil: 'domcontentloaded' }), evaluate: async <T>(script: string) => await page.evaluate(script) as T,
    click: selector => page.locator(selector).click(), fill: (selector, value) => page.locator(selector).fill(value),
    select: async (selector, value) => { await page.locator(selector).selectOption(value); }, close: () => page.close(),
    hover: selector => page.locator(selector).hover(), dblclick: selector => page.locator(selector).dblclick(), rightClick: selector => page.locator(selector).click({ button: 'right' }),
    drag: (from, to) => page.locator(from).dragTo(page.locator(to)), wheel: async (selector, deltaY) => { await page.locator(selector).hover(); await page.mouse.wheel(0, deltaY); },
    press: (selector, key) => page.locator(selector).press(key),
    goBack: async () => { await page.goBack({ waitUntil: 'domcontentloaded' }); },
    withPopup: async (trigger, act) => {
      const [popup] = await Promise.all([page.waitForEvent('popup'), page.locator(trigger).click()]);
      try { await popup.waitForLoadState('domcontentloaded'); await act(wrapPlaywright(popup)); } finally { await popup.close(); }
    },
    withFrame: async (selector, act) => { const frame = await (await page.locator(selector).elementHandle())?.contentFrame(); if (!frame) throw new Error(`no frame for ${selector}`); await act(wrapPlaywrightFrame(frame)); },
    upload: (selector, path) => page.locator(selector).setInputFiles(path),
    download: async selector => { const [download] = await Promise.all([page.waitForEvent('download'), page.locator(selector).click()]); return readFile((await download.path())!, 'utf8'); } };
}
function wrapPlaywrightFrame(frame: import('playwright').Frame): BrowserPage {
  return { goto: url => frame.goto(url), evaluate: async <T>(script: string) => await frame.evaluate(script) as T, click: selector => frame.locator(selector).click(),
    fill: (selector, value) => frame.locator(selector).fill(value), select: async (selector, value) => { await frame.locator(selector).selectOption(value); }, close: async () => {} };
}
function wrapPuppeteer(page: PuppeteerPage): BrowserPage {
  return { goto: url => page.goto(url, { waitUntil: 'domcontentloaded' }), evaluate: async <T>(script: string) => await page.evaluate(script) as T,
    click: async selector => { await page.locator(selector).click(); }, fill: async (selector, value) => { await page.locator(selector).fill(value); },
    select: async (selector, value) => { await page.select(selector, value); }, close: () => page.close(),
    // Puppeteer's locator API for what it offers; drag and wheel fall back to its raw mouse, without actionability checks.
    hover: async selector => { await page.locator(selector).hover(); }, dblclick: async selector => { await page.locator(selector).click({ count: 2 }); },
    rightClick: async selector => { await page.locator(selector).click({ button: 'right' }); },
    drag: async (from, to) => {
      const center = async (selector: string) => { const box = await (await page.$(selector))!.boundingBox(); if (!box) throw new Error(`no box for ${selector}`); return { x: box.x + box.width / 2, y: box.y + box.height / 2 }; };
      const start = await center(from), end = await center(to); await page.mouse.move(start.x, start.y); await page.mouse.down(); await page.mouse.move(end.x, end.y); await page.mouse.up();
    },
    wheel: async (selector, deltaY) => { await page.locator(selector).hover(); await page.mouse.wheel({ deltaY }); },
    press: async (selector, key) => { const handle = await page.$(selector); if (!handle) throw new Error(`no element for ${selector}`); await handle.press(key); },
    goBack: async () => { await page.goBack({ waitUntil: 'domcontentloaded' }); },
    withPopup: async (trigger, act) => {
      const opened = new Promise<PuppeteerPage | null>(resolve => page.once('popup', resolve)); await page.locator(trigger).click();
      const popup = await opened; if (!popup) throw new Error('popup did not open');
      try { await popup.waitForSelector('body'); await act(wrapPuppeteerFrame(popup.mainFrame())); } finally { await popup.close(); }
    },
    withFrame: async (selector, act) => { const frame = await (await page.$(selector))?.contentFrame(); if (!frame) throw new Error(`no frame for ${selector}`); await act(wrapPuppeteerFrame(frame)); },
    upload: async (selector, path) => { const handle = await page.$(selector) as ElementHandle<HTMLInputElement> | null; if (!handle) throw new Error(`no element for ${selector}`); await handle.uploadFile(path); },
    // Puppeteer has no download API: downloads go through the CDP Browser domain, as its users do.
    download: async selector => {
      const directory = await mkdtemp(join(tmpdir(), 'automan-download-')); const session = await page.createCDPSession();
      await session.send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: directory, eventsEnabled: true });
      const completed = new Promise<void>((resolve, reject) => session.on('Browser.downloadProgress', event => { if (event.state === 'completed') resolve(); if (event.state === 'canceled') reject(new Error('download canceled')); }));
      await page.locator(selector).click(); await completed;
      const [name] = await readdir(directory); await session.detach(); return readFile(join(directory, name!), 'utf8');
    } };
}
function wrapPuppeteerFrame(frame: PuppeteerFrame): BrowserPage {
  return { goto: async url => { await frame.goto(url); }, evaluate: async <T>(script: string) => await frame.evaluate(script) as T, click: async selector => { await frame.locator(selector).click(); },
    fill: async (selector, value) => { await frame.locator(selector).fill(value); }, select: async (selector, value) => { await frame.select(selector, value); }, close: async () => {} };
}
serve(adapter);
