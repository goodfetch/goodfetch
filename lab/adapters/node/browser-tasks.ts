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
export function actionReadinessScript(timeoutMs:number){
  return `(async()=>{
    const end=performance.now()+${timeoutMs};let previous=null,previousAt=0,stable=0;
    while(performance.now()<end){
      await new Promise(resolve=>requestAnimationFrame(resolve));
      const now=performance.now();if(now-previousAt<16)continue;
      const elements=document.querySelectorAll('#target');if(elements.length>1)throw new Error('ambiguous target');const el=elements[0];
      if(el){const box=el.getBoundingClientRect(),style=getComputedStyle(el),current=[box.x,box.y,box.width,box.height].join(',');
        stable=current===previous?stable+1:0;previous=current;previousAt=now;
        const hit=document.elementFromPoint(box.x+box.width/2,box.y+box.height/2);
        if(!el.disabled&&box.width>0&&box.height>0&&style.visibility!=='hidden'&&style.display!=='none'&&(hit===el||el.contains(hit))&&stable>=2)return true;
      }else{previous=null;stable=0;previousAt=now;}
    }throw new Error('actionability timeout');
  })()`;
}
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
  if(task.scenario==='actionability'){
    await page.evaluate(waitScript(`document.body.dataset.ready==='true'`,task.timeoutMs));
    const negative=['never-enabled','never-uncovered','ambiguous'].includes(task.variant);let nativeRejected=0;const began=performance.now();
    try{if(task.actionPolicy==='guarded')await page.evaluate(actionReadinessScript(task.timeoutMs));await page.click('#target')}catch(error){nativeRejected=1;if(!negative)throw error}
    const nativeClickMs=performance.now()-began;
    if(!negative)await page.evaluate(waitScript(`document.querySelector('#action-saved').textContent==='saved'`,task.timeoutMs));
    return {status:'ok',metrics:{nativeRejected,nativeClickMs,taskMs:performance.now()-start}};
  }
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
