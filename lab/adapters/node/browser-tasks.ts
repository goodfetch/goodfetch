import type { AdapterOutput, Task } from '../../src/worker-server.ts';
import { waitScript, guardScript, condition, recordsScript, nextScript, snapshotScript } from '../browser-scripts/scripts.ts';

export interface BrowserPage {
  goto(url: string): Promise<unknown>;
  evaluate<T>(script: string): Promise<T>;
  click(selector: string): Promise<void>;
  fill(selector: string, value: string): Promise<void>;
  select(selector: string, value: string): Promise<void>;
  close(): Promise<void>;
}
export { waitScript } from '../browser-scripts/scripts.ts';
// The Automan guard: bounded strictness, visibility, enabled, stability and hit-test checks, shadow DOM included.
export const actionReadinessScript = (timeoutMs: number, selector = '#target') => guardScript(selector, timeoutMs);
export async function runBrowserTask(page: BrowserPage, task: Task, alreadyNavigated = false): Promise<AdapterOutput> {
  const start = performance.now(); let operations = 0; const metrics: Record<string, number> = {};
  // Guarded drivers check every action target first; native drivers rely on their own guarantees.
  const guarded = async (selector: string) => { if (task.actionPolicy === 'guarded') { await page.evaluate(guardScript(selector, task.timeoutMs)); operations++; } };
  if (!alreadyNavigated) { await page.goto(task.url); operations++; }
  if (task.scenario.endsWith('catalog')) {
    const records: { id: string; price: number }[] = []; let url: string | null = task.url; let visited = 0;
    while (url) {
      if (visited++ > task.pages) throw new Error('pagination exceeded contract');
      await page.evaluate(waitScript(condition('catalog-ready'), task.timeoutMs)); operations++;
      records.push(...await page.evaluate<{ id: string; price: number }[]>(recordsScript)); operations++;
      url = await page.evaluate<string | null>(nextScript); operations++;
      if (url) { await page.goto(url); operations++; }
    }
    return { status: 'ok', records, metrics: { ...metrics, operationCount: operations, taskMs: performance.now() - start } };
  }
  if (task.scenario === 'download') return { status: 'unsupported', reason: 'download events not yet qualified on this adapter' };
  if (task.scenario === 'iframe') return { status: 'unsupported', reason: 'frame-specific adapter contract not qualified' };
  if(task.scenario==='actionability'){
    await page.evaluate(waitScript(condition('body-ready'),task.timeoutMs));
    const negative=['never-enabled','never-uncovered','ambiguous'].includes(task.variant);let nativeRejected=0;const began=performance.now();
    try{if(task.actionPolicy==='guarded')await page.evaluate(guardScript('#target',task.timeoutMs));await page.click('#target')}catch(error){nativeRejected=1;if(!negative)throw error}
    const nativeClickMs=performance.now()-began;
    if(!negative)await page.evaluate(waitScript(condition('action-saved'),task.timeoutMs));
    return {status:'ok',metrics:{nativeRejected,nativeClickMs,taskMs:performance.now()-start}};
  }
  await page.evaluate(waitScript(condition('ui-ready'), task.timeoutMs)); operations++;
  const snapshotStart = performance.now();
  const snapshot = await page.evaluate<{nodes:number;width:number;height:number}>(snapshotScript);
  if(snapshot.width!==1280||snapshot.height!==720) throw new Error(`viewport contract: ${snapshot.width}x${snapshot.height}`);
  metrics.domNodes = snapshot.nodes; metrics.viewportWidth=snapshot.width; metrics.viewportHeight=snapshot.height;
  metrics.domCountMs = performance.now() - snapshotStart; operations++;
  if (task.scenario === 'clicks') {
    for (let i = 1; i <= task.actions; i++) {
      const clickStart = performance.now(); await guarded('#counter'); await page.click('#counter');
      await page.evaluate(waitScript(condition('count', { N: String(i) }), task.timeoutMs));
      metrics.clickTotalMs = (metrics.clickTotalMs ?? 0) + performance.now() - clickStart; operations += 2;
    }
  } else {
    const shadow = task.scenario === 'shadow';
    const root = shadow ? 'document.querySelector("#shadow-host").shadowRoot' : 'document';
    // Each backend resolves fresh. Explicit common readiness is timed, not a forced click.
    await page.evaluate(waitScript(condition('submit-enabled', { ROOT: root }), task.timeoutMs)); operations++;
    await guarded('#name'); await page.fill('#name', 'Automan'); await guarded('#region'); await page.select('#region', 'eu');
    await guarded('#consent'); await page.click('#consent'); await guarded('#submit'); await page.click('#submit'); operations += 4;
    await page.evaluate(waitScript(condition('saved'), task.timeoutMs)); operations++;
  }
  return { status: 'ok', metrics: { ...metrics, operationCount: operations, taskMs: performance.now() - start } };
}
