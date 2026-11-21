import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { AdapterOutput, Task } from '../../src/worker-server.ts';
import { waitScript, guardScript, armScript, disarmScript, gestureEvents, condition, recordsScript, nextScript, snapshotScript } from '../browser-scripts/scripts.ts';

export interface BrowserPage {
  goto(url: string): Promise<unknown>;
  evaluate<T>(script: string): Promise<T>;
  click(selector: string): Promise<void>;
  fill(selector: string, value: string): Promise<void>;
  select(selector: string, value: string): Promise<void>;
  close(): Promise<void>;
  // Primitives; a driver without one reports the primitives scenario as unsupported.
  hover?(selector: string): Promise<void>;
  dblclick?(selector: string): Promise<void>;
  rightClick?(selector: string): Promise<void>;
  drag?(from: string, to: string): Promise<void>;
  wheel?(selector: string, deltaY: number): Promise<void>;
  press?(selector: string, key: 'Enter'): Promise<void>;
  // Navigation, frames and files; popups and frames are handed to the task as pages of their own.
  goBack?(): Promise<void>;
  withPopup?(trigger: string, act: (popup: BrowserPage) => Promise<void>): Promise<void>;
  withFrame?(frame: string, act: (frame: BrowserPage) => Promise<void>): Promise<void>;
  upload?(selector: string, path: string): Promise<void>;
  download?(selector: string): Promise<string>;
  // A driver that runs the Goodfetch Guard itself on every action (Firefox over WebDriver BiDi) takes no lab guard on top.
  ownGuard?: true;
}
// The drivers' own copy of the unicode variant text (the fixture keeps the oracle's copy).
export const unicodeText = 'Ünïcødé — 漢字 😀';
export const uploadText = 'goodfetch-upload\n';
const primitiveNegatives = ['retarget'];
export { waitScript } from '../browser-scripts/scripts.ts';
// The Collie guard: bounded strictness, visibility, enabled, stability and hit-test checks, shadow DOM included.
export const actionReadinessScript = (timeoutMs: number, selector = '#target') => guardScript(selector, timeoutMs);
export async function runBrowserTask(page: BrowserPage, task: Task, alreadyNavigated = false): Promise<AdapterOutput> {
  const start = performance.now(); let operations = 0; const metrics: Record<string, number> = {};
  // Guarded drivers check every action target first; native drivers rely on their own guarantees.
  const labGuard = task.actionPolicy === 'guarded' && page.ownGuard !== true;
  const guarded = async (selector: string) => { if (labGuard) { await page.evaluate(guardScript(selector, task.timeoutMs)); operations++; } };
  // Guarded gestures also arm a capture listener on the verified element: an event reaching anything else is cancelled
  // and the gesture retried, so a page that moves or swaps the target as the pointer arrives cannot redirect it.
  const gesture = (selector: string, types: readonly string[], perform: () => Promise<void>) => gestureOn(page, selector, types, perform);
  const gestureOn = async (target: BrowserPage, selector: string, types: readonly string[], perform: () => Promise<void>) => {
    if (!labGuard) return perform();
    const deadline = performance.now() + task.timeoutMs;
    for (;;) {
      await target.evaluate(guardScript(selector, Math.max(50, Math.round(deadline - performance.now())))); operations++;
      await target.evaluate(armScript(selector, types)); await perform();
      const intercepted = await target.evaluate<string | null>(disarmScript); operations += 2;
      if (!intercepted) return;
      if (performance.now() >= deadline) throw new Error(`gesture intercepted: ${intercepted}`);
    }
  };
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
  if (task.scenario === 'primitives') {
    const { hover, dblclick, rightClick, drag, wheel, press } = page;
    if (!hover || !dblclick || !rightClick || !drag || !wheel || !press) return { status: 'unsupported', reason: 'primitive actions not implemented by this driver' };
    await page.evaluate(waitScript(condition('body-ready'), task.timeoutMs));
    const act: Record<string, () => Promise<void>> = {
      hover: async () => { await guarded('#target'); await hover.call(page, '#target'); },
      dblclick: () => gesture('#target', gestureEvents.dblclick, () => dblclick.call(page, '#target')),
      contextmenu: () => gesture('#target', gestureEvents.contextmenu, () => rightClick.call(page, '#target')),
      drag: () => gesture('#source', gestureEvents.drag, () => drag.call(page, '#source', '#dropzone')),
      wheel: () => gesture('#scroller', gestureEvents.wheel, () => wheel.call(page, '#scroller', 600)),
      keyboard: async () => { await guarded('#field'); await press.call(page, '#field', 'Enter'); },
      contenteditable: async () => { await guarded('#editor'); await page.fill('#editor', 'Goodfetch'); },
      unicode: async () => { await guarded('#field'); await page.fill('#field', unicodeText); },
      moved: () => gesture('#target', gestureEvents.click, () => page.click('#target')),
      retarget: () => gesture('#target', gestureEvents.click, () => page.click('#target'))
    };
    const action = act[task.variant]; if (!action) throw new Error(`unknown primitive ${task.variant}`);
    const negative = primitiveNegatives.includes(task.variant); let nativeRejected = 0; const began = performance.now();
    try { await action(); } catch (error) { nativeRejected = 1; if (!negative) throw error; }
    const actionMs = performance.now() - began;
    if (!negative) await page.evaluate(waitScript(condition('done'), task.timeoutMs));
    return { status: 'ok', metrics: { nativeRejected, actionMs, taskMs: performance.now() - start } };
  }
  if (task.scenario === 'navigation') {
    const required: Record<string, unknown> = { history: page.goBack, popup: page.withPopup, frame: page.withFrame, 'cross-frame': page.withFrame, upload: page.upload, download: page.download };
    if (task.variant in required && !required[task.variant]) return { status: 'unsupported', reason: `${task.variant} not implemented by this driver` };
    await page.evaluate(waitScript(condition('body-ready'), task.timeoutMs));
    // Inside a popup or a frame: same readiness, guarded gesture and postcondition as on the main page.
    const confirm = async (target: BrowserPage) => {
      await target.evaluate(waitScript(condition('body-ready'), task.timeoutMs));
      await gestureOn(target, '#confirm', gestureEvents.click, () => target.click('#confirm'));
      await target.evaluate(waitScript(condition('done'), task.timeoutMs));
    };
    let file: string | undefined;
    if (task.variant === 'spa') { await gesture('#go', gestureEvents.click, () => page.click('#go')); await page.evaluate(waitScript(condition('done'), task.timeoutMs)); }
    else if (task.variant === 'history') { await page.goto(task.url.replace('/ui', '/page-b')); await page.evaluate(waitScript(condition('page-b'), task.timeoutMs)); await page.goBack!(); await page.evaluate(waitScript(condition('done'), task.timeoutMs)); }
    else if (task.variant === 'popup') { await guarded('#open'); await page.withPopup!('#open', confirm); }
    // A cross-origin frame first commits in-process and then swaps to its own process: resolve it once loaded.
    else if (task.variant === 'frame' || task.variant === 'cross-frame') { await page.evaluate(waitScript(condition('frame-loaded'), task.timeoutMs)); await page.withFrame!('#frame', confirm); }
    else if (task.variant === 'upload') {
      const path = join(await mkdtemp(join(tmpdir(), 'goodfetch-upload-')), 'upload.txt'); await writeFile(path, uploadText);
      await guarded('#file'); await page.upload!('#file', path); await page.evaluate(waitScript(condition('done'), task.timeoutMs));
    } else if (task.variant === 'download') { await guarded('#file'); file = await page.download!('#file'); }
    else throw new Error(`unknown navigation variant ${task.variant}`);
    return { status: 'ok', ...(file !== undefined ? { file } : {}), metrics: { taskMs: performance.now() - start } };
  }
  if (task.scenario === 'download') return { status: 'unsupported', reason: 'download events not yet qualified on this adapter' };
  if (task.scenario === 'iframe') return { status: 'unsupported', reason: 'frame-specific adapter contract not qualified' };
  if(task.scenario==='actionability'){
    await page.evaluate(waitScript(condition('body-ready'),task.timeoutMs));
    const negative=['never-enabled','never-uncovered','ambiguous'].includes(task.variant);let nativeRejected=0;const began=performance.now();
    try{await gesture('#target',gestureEvents.click,()=>page.click('#target'))}catch(error){nativeRejected=1;if(!negative)throw error}
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
      const clickStart = performance.now(); await gesture('#counter', gestureEvents.click, () => page.click('#counter'));
      await page.evaluate(waitScript(condition('count', { N: String(i) }), task.timeoutMs));
      metrics.clickTotalMs = (metrics.clickTotalMs ?? 0) + performance.now() - clickStart; operations += 2;
    }
  } else {
    const shadow = task.scenario === 'shadow';
    const root = shadow ? 'document.querySelector("#shadow-host").shadowRoot' : 'document';
    // Each backend resolves fresh. Explicit common readiness is timed, not a forced click.
    await page.evaluate(waitScript(condition('submit-enabled', { ROOT: root }), task.timeoutMs)); operations++;
    await guarded('#name'); await page.fill('#name', 'Goodfetch'); await guarded('#region'); await page.select('#region', 'eu');
    await gesture('#consent', gestureEvents.click, () => page.click('#consent')); await gesture('#submit', gestureEvents.click, () => page.click('#submit')); operations += 4;
    await page.evaluate(waitScript(condition('saved'), task.timeoutMs)); operations++;
  }
  return { status: 'ok', metrics: { ...metrics, operationCount: operations, taskMs: performance.now() - start } };
}
