import type { AdapterOutput, Task } from '../../src/worker-server.ts';

export interface BrowserPage {
  goto(url: string): Promise<unknown>;
  evaluate<T>(script: string): Promise<T>;
  click(selector: string): Promise<void>;
  fill(selector: string, value: string): Promise<void>;
  select(selector: string, value: string): Promise<void>;
  close(): Promise<void>;
}
export function waitScript(condition: string, timeoutMs: number) {
  return `(async()=>{const end=performance.now()+${timeoutMs};while(!(${condition})){if(performance.now()>end)throw new Error('postcondition timeout');await new Promise(resolve=>setTimeout(resolve,8))}return true})()`;
}
const recordsScript = `Array.from(document.querySelectorAll('article[data-id]'),el=>({id:el.getAttribute('data-id'),price:Number(el.getAttribute('data-price'))}))`;
export async function runBrowserTask(page: BrowserPage, task: Task, alreadyNavigated = false): Promise<AdapterOutput> {
  const start = performance.now(); let operations = 0; const metrics: Record<string, number> = {};
  if (!alreadyNavigated) { await page.goto(task.url); operations++; }
  if (task.scenario.endsWith('catalog')) {
    const records: { id: string; price: number }[] = []; let url: string | null = task.url; let visited = 0;
    while (url) {
      if (visited++ > task.pages) throw new Error('pagination exceeded contract');
      await page.evaluate(waitScript(`document.querySelector('#catalog')?.getAttribute('data-ready')==='true'`, task.timeoutMs)); operations++;
      records.push(...await page.evaluate<{ id: string; price: number }[]>(recordsScript)); operations++;
      url = await page.evaluate<string | null>(`document.querySelector('a.next')?.href ?? null`); operations++;
      if (url) { await page.goto(url); operations++; }
    }
    return { status: 'ok', records, metrics: { ...metrics, operationCount: operations, taskMs: performance.now() - start } };
  }
  if (task.scenario === 'download') return { status: 'unsupported', reason: 'download events not yet qualified on this adapter' };
  if (task.scenario === 'iframe') return { status: 'unsupported', reason: 'frame-specific adapter contract not qualified' };
  await page.evaluate(waitScript(`document.body.dataset.ready==='true' && !document.querySelector('#overlay')`, task.timeoutMs)); operations++;
  const snapshotStart = performance.now();
  const snapshot = await page.evaluate<{nodes:number;width:number;height:number}>('({nodes:document.querySelectorAll("*").length,width:innerWidth,height:innerHeight})');
  if(snapshot.width!==1280||snapshot.height!==720) throw new Error(`viewport contract: ${snapshot.width}x${snapshot.height}`);
  metrics.domNodes = snapshot.nodes; metrics.viewportWidth=snapshot.width; metrics.viewportHeight=snapshot.height;
  metrics.domCountMs = performance.now() - snapshotStart; operations++;
  if (task.scenario === 'clicks') {
    for (let i = 1; i <= task.actions; i++) {
      const clickStart = performance.now(); await page.click('#counter');
      await page.evaluate(waitScript(`document.querySelector('#count').textContent==='${i}'`, task.timeoutMs));
      metrics.clickTotalMs = (metrics.clickTotalMs ?? 0) + performance.now() - clickStart; operations += 2;
    }
  } else {
    const shadow = task.scenario === 'shadow';
    const root = shadow ? 'document.querySelector("#shadow-host").shadowRoot' : 'document';
    // Each backend resolves fresh. Explicit common readiness is timed, not a forced click.
    await page.evaluate(waitScript(`!${root}.querySelector('#submit').disabled`, task.timeoutMs)); operations++;
    await page.fill('#name', 'Automan'); await page.select('#region', 'eu'); await page.click('#consent'); await page.click('#submit'); operations += 4;
    await page.evaluate(waitScript(`document.querySelector('#saved').textContent==='saved'`, task.timeoutMs)); operations++;
  }
  return { status: 'ok', metrics: { ...metrics, operationCount: operations, taskMs: performance.now() - start } };
}
