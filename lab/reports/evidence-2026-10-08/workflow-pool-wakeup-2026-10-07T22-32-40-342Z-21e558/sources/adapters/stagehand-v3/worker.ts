import { StagehandCrawler, Configuration } from '@crawlee/stagehand';
import { LLMClient } from '@browserbasehq/stagehand';
import { chromium } from 'playwright';
import { serve, type Adapter, type AdapterOutput } from '../../src/worker-server.ts';
import { runBrowserTask, type BrowserPage } from '../node/browser-tasks.ts';

// Model boundary is disabled explicitly; any accidental inference fails the run.
class NoModel extends LLMClient {
  constructor() { super('openai/gpt-4.1-mini'); }
  async createChatCompletion<T>(): Promise<T> { throw new Error('model calls disabled in deterministic lab'); }
}
console.log = console.info = console.warn = (...args: unknown[]) => console.error(...args);
let options: { executablePath?: string; headless?: boolean; viewport?:{width:number;height:number} } = {};
const adapter: Adapter = {
  info() { return { backend: 'crawlee-stagehand', protocol: 1, capabilities: ['http-catalog','js-catalog','form','clicks','shadow'], versions: { '@crawlee/stagehand': '3.18.2', stagehand: '3.7.3', playwright: '1.63.0' }, inference: 'disabled at model boundary' }; },
  async prepare(input) { options = {executablePath:input.executablePath,headless:input.headless??true,viewport:{width:1280,height:720}}; },
  async run(task): Promise<AdapterOutput> {
    if (!['http-catalog','js-catalog','form','clicks','shadow'].includes(task.scenario)) return { status: 'unsupported' };
    const records: {id:string;price:number}[] = [], failures: string[] = []; const start = performance.now(); const taskMetrics:Record<string,number>={};
    const crawler = new StagehandCrawler({ maxConcurrency: 1, maxRequestRetries: 0, maxRequestsPerCrawl: task.pages + 1,
      stagehandOptions: { env: 'LOCAL', verbose: 0, selfHeal: false, llmClient: new NoModel() },
      launchContext: { launcher: chromium, launchOptions: options }, browserPoolOptions: { useFingerprints: false },
      navigationTimeoutSecs: task.timeoutMs / 1000, requestHandlerTimeoutSecs: task.timeoutMs / 1000,
      preNavigationHooks: [async (_, gotoOptions) => { gotoOptions.waitUntil = 'domcontentloaded'; }],
      requestHandler: async ({ page, enqueueLinks }) => {
        if (task.scenario.endsWith('catalog')) {
          await page.waitForSelector('#catalog[data-ready="true"]');
          records.push(...await page.locator('article[data-id]').evaluateAll(els => els.map(el=>({id:el.getAttribute('data-id')!,price:Number(el.getAttribute('data-price'))}))));
          await enqueueLinks({ selector:'a.next' });
        } else {
          const wrapped: BrowserPage = { goto: url=>page.goto(url, { waitUntil: 'domcontentloaded' }), evaluate: async <T>(script:string)=>await page.evaluate(script) as T,
            click: selector=>page.locator(selector).click(), fill:(selector,value)=>page.locator(selector).fill(value), select:async(selector,value)=>{await page.locator(selector).selectOption(value)}, close:()=>page.close() };
          Object.assign(taskMetrics,(await runBrowserTask(wrapped, task, true)).metrics);
        }
      }, failedRequestHandler: async({request})=>{failures.push(request.url)}
    }, new Configuration({ persistStorage:false }));
    await crawler.run([task.url]);
    if(failures.length) throw new Error(`crawler failed ${failures.length} requests`);
    return { status:'ok', records, metrics:{...taskMetrics,taskMs:performance.now()-start} };
  },
  async close() {}
};
serve(adapter);
