import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { randomBytes } from 'node:crypto';
import {gzipSync} from 'node:zlib';
import type { AddressInfo } from 'node:net';

export type RecordItem = { id: string; price: number };
export type Verdict = { status: 'verified' | 'failed'; reasons: string[] };
export type Scenario = 'http-catalog' | 'js-catalog' | 'form' | 'clicks' | 'shadow' | 'iframe' | 'download' | 'actionability' | 'primitives' | 'navigation';
export type RunSpec = { scenario: Scenario; seed: number; pages?: number; actions?: number; delayMs?: number; variant?: 'normal' | 'overlay' | 'rerender' | 'delayed' | 'hidden' | 'moving' | 'never-enabled' | 'never-uncovered' | 'ambiguous' | 'hover' | 'dblclick' | 'contextmenu' | 'drag' | 'wheel' | 'keyboard' | 'contenteditable' | 'unicode' | 'moved' | 'retarget' | 'spa' | 'history' | 'popup' | 'frame' | 'cross-frame' | 'upload' | 'download' | 'http-429' | 'http-503-once' | 'http-truncated' | 'http-reset' | 'http-redirect-loop' | 'http-slow' | 'http-redirect' | 'http-gzip' | 'http-chunked' | 'http-large' };
type Run = RunSpec & { id: string; records: RecordItem[]; effects: unknown[]; requests: number; errors: number; incidents: { at: number; status: number }[] };
// HTTP incidents that a client must surface as an explicit failure rather than a success.
const httpRefusals = ['http-truncated', 'http-reset', 'http-redirect-loop', 'http-slow'];

function send(res: ServerResponse, status: number, body: string, type = 'text/html') {
  res.writeHead(status, { 'content-type': `${type}; charset=utf-8`, 'cache-control': 'no-store' }); res.end(body);
}
async function body(req: IncomingMessage): Promise<unknown> {
  let text = ''; for await (const chunk of req) { text += chunk; if (text.length > 1_000_000) throw new Error('body too large'); }
  return JSON.parse(text || '{}');
}
function pageHtml(run: Run, page: number) {
  const records = run.records.slice(page * 2, page * 2 + 2);
  const items = records.map(record => `<article data-id="${record.id}" data-price="${record.price}"><h2>${record.id}</h2><span>${record.price}</span></article>`).join('');
  const next = page + 1 < (run.pages ?? 3) ? `<a class="next" href="/runs/${run.id}/catalog?p=${page + 1}">Next</a>` : '';
  return `<main id="catalog" data-ready="true">${items}${next}</main>`;
}
function ui(run: Run, crossOrigin = '') {
  if(run.scenario==='actionability')return actionUi(run);
  if(run.scenario==='primitives')return primitivesUi(run);
  if(run.scenario==='navigation')return navigationUi(run, crossOrigin);
  const nested = run.scenario === 'shadow' ? '<div id="shadow-host"></div>' : run.scenario === 'iframe' ? `<iframe src="/runs/${run.id}/frame"></iframe>` : '';
  const controls = `<label>Name <input id="name" name="name"></label><select id="region"><option value="us">US</option><option value="eu">EU</option></select><input type="checkbox" id="consent"><button id="submit">Save</button>`;
  return `<!doctype html><html><body>${nested || controls}<button id="counter">Increment</button><output id="count">0</output><output id="saved"></output><a id="download" href="/runs/${run.id}/file">Download</a>
  <script>
  const events=[]; let count=0;
  function setup(root){
    for(const type of ['input','change']) root.addEventListener(type,e=>events.push({type,id:e.target.id,trusted:e.isTrusted}));
    root.querySelector('#submit').addEventListener('click',async e=>{
      const data={name:root.querySelector('#name').value,region:root.querySelector('#region').value,consent:root.querySelector('#consent').checked,events:[...events],trusted:e.isTrusted};
      await fetch('/runs/${run.id}/effect',{method:'POST',body:JSON.stringify(data)}); document.querySelector('#saved').textContent='saved';
    });
  }
  ${run.scenario === 'shadow' ? `const root=document.querySelector('#shadow-host').attachShadow({mode:'open'}); root.innerHTML=${JSON.stringify(controls)}; setup(root);` : run.scenario === 'iframe' ? '' : 'setup(document);'}
  const formRoot=document.querySelector('#shadow-host')?.shadowRoot ?? document;
  document.querySelector('#counter').addEventListener('click',async e=>{const n=++count;await fetch('/runs/${run.id}/effect',{method:'POST',body:JSON.stringify({count:n,trusted:e.isTrusted})});document.querySelector('#count').textContent=String(n)});
  ${run.variant === 'delayed' ? "formRoot.querySelector('#submit').disabled=true;setTimeout(()=>formRoot.querySelector('#submit').disabled=false,90);" : ''}
  ${run.variant === 'overlay' ? "const overlay=document.createElement('div');overlay.id='overlay';overlay.style='position:fixed;inset:0;z-index:100;background:#fff8';document.body.append(overlay);setTimeout(()=>overlay.remove(),120);" : ''}
  ${run.variant === 'rerender' ? "const original=formRoot.querySelector('#submit');original.replaceWith(original.cloneNode(true));setup(formRoot);" : ''}
  document.body.dataset.ready='true';
  </script></body></html>`;
}
function actionUi(run:Run){
  const variant=run.variant??'normal';const negative=['never-enabled','never-uncovered','ambiguous'].includes(variant);
  return `<!doctype html><html><body><button id="target">Target</button>${variant==='ambiguous'?'<button id="target">Other target</button>':''}<output id="action-saved"></output><script>
    let ready=${variant==='normal'||variant==='ambiguous'?'true':'false'};
    const attach=button=>button.addEventListener('click',async event=>{await fetch('/runs/${run.id}/effect',{method:'POST',body:JSON.stringify({target:'target',phaseReady:ready,trusted:event.isTrusted})});document.querySelector('#action-saved').textContent='saved'});
    document.querySelectorAll('#target').forEach(attach);let target=document.querySelector('#target');
    ${variant==='delayed'||variant==='never-enabled'||variant==='rerender'?"target.disabled=true;":''}
    ${variant==='hidden'?"target.style.display='none';":''}
    ${variant==='moving'?"target.style.position='relative';let x=0;var motion=setInterval(()=>{x=(x+30)%300;target.style.left=x+'px'},16);":''}
    ${variant==='overlay'||variant==='never-uncovered'?"const overlay=document.createElement('div');overlay.id='overlay';overlay.style='position:fixed;inset:0;z-index:100;background:#fff8';overlay.onclick=async event=>{await fetch('/runs/"+run.id+"/effect',{method:'POST',body:JSON.stringify({target:'overlay',phaseReady:ready,trusted:event.isTrusted})})};document.body.append(overlay);":''}
    ${negative?'':`setTimeout(()=>{${variant==='rerender'?"const replacement=target.cloneNode(true);target.replaceWith(replacement);target=replacement;attach(target);":''}target.disabled=false;target.style.display='';document.querySelector('#overlay')?.remove();${variant==='moving'?"clearInterval(motion);target.style.left='0px';":''}ready=true},180);`}
    document.body.dataset.ready='true';
  </script></body></html>`;
}

// Expected text of the unicode variant; drivers carry their own copy so the oracle stays independent.
export const unicodeText = 'Ünïcødé — 漢字 😀';
const primitiveNegatives = ['retarget'];
// One primitive per variant. Every effect is posted with its event kind, target and isTrusted; #done is set only by the expected effect.
function primitivesUi(run: Run) {
  const variant = run.variant ?? 'hover';
  const box = (id: string, left: number, top: number, width: number, height: number, extra = '') => `<div id="${id}" style="position:absolute;left:${left}px;top:${top}px;width:${width}px;height:${height}px;${extra}"></div>`;
  const button = (id: string, label: string, left = 100, top = 100, z = 2) => `<button id="${id}" style="position:absolute;left:${left}px;top:${top}px;width:160px;height:40px;z-index:${z}">${label}</button>`;
  const markup: Record<string, string> = {
    hover: button('target', 'Target'), dblclick: button('target', 'Target'), contextmenu: button('target', 'Target'),
    drag: box('source', 100, 100, 80, 80, 'background:#68a;touch-action:none') + box('dropzone', 400, 100, 120, 120, 'border:2px dashed #333'),
    wheel: `<div id="scroller" style="position:absolute;left:100px;top:100px;width:300px;height:200px;overflow:auto"><div style="height:2000px"></div></div>`,
    keyboard: '<input id="field" style="position:absolute;left:100px;top:100px;width:200px">', unicode: '<input id="field" style="position:absolute;left:100px;top:100px;width:300px">',
    contenteditable: '<div id="editor" contenteditable="true" style="position:absolute;left:100px;top:100px;width:300px;height:60px;border:1px solid #333"></div>',
    moved: button('decoy', 'Decoy', 100, 100, 1) + button('target', 'Target'), retarget: button('target', 'Save')
  };
  const script: Record<string, string> = {
    hover: "target.addEventListener('pointerenter',async e=>{await post({kind:'hover',target:'target',trusted:e.isTrusted});done()},{once:true});",
    dblclick: "target.addEventListener('dblclick',async e=>{await post({kind:'dblclick',target:'target',trusted:e.isTrusted});done()});",
    contextmenu: "target.addEventListener('contextmenu',async e=>{e.preventDefault();await post({kind:'contextmenu',target:'target',trusted:e.isTrusted});done()});",
    drag: "let dragging=false,moves=0;source.addEventListener('pointerdown',()=>{dragging=true;moves=0});document.addEventListener('pointermove',()=>{if(dragging)moves++});document.addEventListener('pointerup',async e=>{if(!dragging)return;dragging=false;if(dropzone.contains(document.elementFromPoint(e.clientX,e.clientY))){await post({kind:'drop',target:'dropzone',trusted:e.isTrusted,moves});done()}});",
    wheel: "let wheeled=false,sent=false;scroller.addEventListener('wheel',e=>{if(e.isTrusted)wheeled=true},{passive:true});scroller.addEventListener('scroll',async()=>{if(!sent&&wheeled&&scroller.scrollTop>=300){sent=true;await post({kind:'scrolled',target:'scroller',trusted:true,scrollTop:scroller.scrollTop});done()}});",
    keyboard: "field.addEventListener('keydown',async e=>{if(e.key==='Enter'){await post({kind:'enter',target:'field',trusted:e.isTrusted});done()}});",
    contenteditable: "let sent=false;editor.addEventListener('input',async e=>{if(!sent&&editor.textContent==='Automan'){sent=true;await post({kind:'edited',target:'editor',trusted:e.isTrusted,text:editor.textContent});done()}});",
    unicode: `let sent=false;field.addEventListener('input',async e=>{if(!sent&&field.value===${JSON.stringify(unicodeText)}){sent=true;await post({kind:'typed',target:'field',trusted:e.isTrusted,value:field.value});done()}});`,
    // The target slips away as the pointer arrives, uncovering a decoy at the point it was verified.
    moved: "target.addEventListener('pointerover',()=>{target.style.left='500px'},{once:true});target.addEventListener('click',async e=>{await post({kind:'click',target:'target',trusted:e.isTrusted});done()});decoy.addEventListener('click',async e=>{await post({kind:'click',target:'decoy',trusted:e.isTrusted})});",
    // The target is swapped for a destructive control at the same place as the pointer arrives.
    retarget: "target.addEventListener('pointerover',()=>{const danger=document.createElement('button');danger.id='danger';danger.textContent='Delete';danger.style.cssText=target.style.cssText;danger.addEventListener('click',async e=>{await post({kind:'click',target:'danger',trusted:e.isTrusted})});target.replaceWith(danger)},{once:true});target.addEventListener('click',async e=>{await post({kind:'click',target:'target',trusted:e.isTrusted})});"
  };
  return `<!doctype html><html><body style="margin:0;font:16px sans-serif"><output id="done"></output>${markup[variant] ?? ''}<script>
    const post=effect=>fetch('/runs/${run.id}/effect',{method:'POST',body:JSON.stringify(effect)});const done=()=>{document.querySelector('#done').textContent='done'};
    ${script[variant] ?? ''}
    document.body.dataset.ready='true';
  </script></body></html>`;
}
// Navigation, frames and files. Each page posts its effect with isTrusted; #done marks the expected end state.
export const uploadText = 'automan-upload\n';
function navigationUi(run: Run, crossOrigin: string) {
  const variant = run.variant ?? 'spa';
  const post = `const post=effect=>fetch('/runs/${run.id}/effect',{method:'POST',body:JSON.stringify(effect)});const done=()=>{document.querySelector('#done').textContent='done'};`;
  const body: Record<string, string> = {
    spa: `<a id="go" href="/runs/${run.id}/ui?view=detail">Detail</a><h1 id="view">list</h1><script>${post}document.querySelector('#go').addEventListener('click',async e=>{e.preventDefault();history.pushState({},'',e.currentTarget.href);document.querySelector('#view').textContent='detail';await post({kind:'route',path:location.pathname+location.search,trusted:e.isTrusted});done()});</script>`,
    history: `<h1>page A</h1><script>${post}addEventListener('pageshow',async e=>{const type=performance.getEntriesByType('navigation')[0]?.type;if(e.persisted||type==='back_forward'){await post({kind:'page',page:'a',back:true});done()}});</script>`,
    popup: `<button id="open" style="width:160px;height:40px">Open</button><script>${post}document.querySelector('#open').addEventListener('click',()=>{window.open('/runs/${run.id}/popup','_blank','width=600,height=400')});</script>`,
    frame: `<iframe id="frame" onload="this.dataset.loaded='true'" src="/runs/${run.id}/inner" style="border:0;width:400px;height:200px;position:absolute;left:50px;top:80px"></iframe>`,
    'cross-frame': `<iframe id="frame" onload="this.dataset.loaded='true'" src="${crossOrigin}/runs/${run.id}/inner" style="border:0;width:400px;height:200px;position:absolute;left:50px;top:80px"></iframe>`,
    upload: `<input id="file" type="file"><script>${post}document.querySelector('#file').addEventListener('change',async e=>{const file=e.target.files[0];await post({kind:'upload',name:file.name,content:await file.text(),trusted:e.isTrusted});done()});</script>`,
    download: `<a id="file" href="/runs/${run.id}/file" download="catalog.txt">Download</a>`
  };
  return `<!doctype html><html><body style="margin:0;font:16px sans-serif"><output id="done"></output>${body[variant] ?? ''}<script>document.body.dataset.ready='true'</script></body></html>`;
}
// Pages opened from navigation runs: the second history page, the popup and the frame content, each with one trusted control.
function navigationChild(run: Run, kind: 'page-b' | 'popup' | 'inner') {
  const control = kind === 'page-b' ? '<h1>page B</h1>' : `<button id="confirm" style="position:absolute;left:20px;top:20px;width:160px;height:40px">Confirm</button>`;
  const effect = kind === 'popup' ? 'popup-click' : 'frame-click';
  return `<!doctype html><html><body style="margin:0">${control}<output id="done"></output><script>
    ${kind === 'page-b' ? '' : `document.querySelector('#confirm').addEventListener('click',async e=>{await fetch('/runs/${run.id}/effect',{method:'POST',body:JSON.stringify({kind:'${effect}',trusted:e.isTrusted,origin:location.origin})});document.querySelector('#done').textContent='done'});`}
    document.body.dataset.ready='true';
  </script></body></html>`;
}

const expectedPrimitive: Record<string, { kind: string; target: string }> = {
  hover: { kind: 'hover', target: 'target' }, dblclick: { kind: 'dblclick', target: 'target' }, contextmenu: { kind: 'contextmenu', target: 'target' }, drag: { kind: 'drop', target: 'dropzone' },
  wheel: { kind: 'scrolled', target: 'scroller' }, keyboard: { kind: 'enter', target: 'field' }, contenteditable: { kind: 'edited', target: 'editor' }, unicode: { kind: 'typed', target: 'field' }, moved: { kind: 'click', target: 'target' }
};

export async function startFixtures() {
  const runs = new Map<string, Run>();
  const server = createServer(async (req, res) => {
    try {
      const url = new URL(req.url ?? '/', 'http://fixture');
      if (url.pathname === '/robots.txt') return send(res, 200, 'User-agent: *\nAllow: /\n', 'text/plain');
      const [, , id, action] = url.pathname.split('/'); const run = id ? runs.get(id) : undefined;
      if (!run) return send(res, 404, 'unknown fixture');
      run.requests++;
      if (run.delayMs) await new Promise(resolve => setTimeout(resolve, run.delayMs));
      if (action === 'effect' && req.method === 'POST') { run.effects.push(await body(req)); return send(res, 200, '{}', 'application/json'); }
      if (action === 'catalog'||action==='redirect') {
        const p = Number(url.searchParams.get('p') ?? 0);
        if (!Number.isInteger(p) || p < 0 || p >= (run.pages ?? 3)) return send(res, 404, 'bad page');
        if(run.variant==='http-redirect'&&action==='catalog'){res.writeHead(302,{location:`/runs/${id}/redirect?p=${p}`});res.end();return}
        // Incidents hit the second page only, so a client that mishandles them can still look plausible.
        if (p === 1 && run.scenario === 'http-catalog' && run.variant?.startsWith('http-')) {
          const first = !run.incidents.length; const at = performance.now();
          if (run.variant === 'http-429' && first) { run.incidents.push({ at, status: 429 }); res.writeHead(429, { 'retry-after': '1', 'content-type': 'text/plain' }); res.end('slow down'); return; }
          if (run.variant === 'http-503-once' && first) { run.incidents.push({ at, status: 503 }); return send(res, 503, 'unavailable', 'text/plain'); }
          if (run.variant === 'http-redirect-loop') { run.incidents.push({ at, status: 302 }); res.writeHead(302, { location: `/runs/${id}/catalog?p=1` }); res.end(); return; }
          if (run.variant === 'http-slow') { run.incidents.push({ at, status: 0 }); await new Promise(resolve => setTimeout(resolve, 8000)); if (!res.destroyed) send(res, 200, `<!doctype html><html><body>${pageHtml(run, p)}</body></html>`); return; }
          if (run.variant === 'http-truncated' || run.variant === 'http-reset') {
            run.incidents.push({ at, status: 200 }); const page = `<!doctype html><html><body>${pageHtml(run, p)}</body></html>`; const cut = page.slice(0, Math.floor(page.indexOf('<a class="next"') > 0 ? page.indexOf('<a class="next"') : page.length * 0.4));
            res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', ...(run.variant === 'http-truncated' ? { 'content-length': String(Buffer.byteLength(page)) } : {}) });
            res.write(cut); await new Promise(resolve => setTimeout(resolve, 10)); res.socket?.destroy(); return;
          }
          if (!first) run.incidents.push({ at, status: 200 });
        }
        if (run.scenario === 'js-catalog') return send(res, 200, `<!doctype html><div id="loading">Loading</div><script>fetch('/runs/${id}/data?p=${p}').then(r=>r.text()).then(html=>{document.body.innerHTML=html})</script>`);
        const html=`<!doctype html><html><body>${pageHtml(run, p)}${run.variant==='http-large'?'<!--'+'x'.repeat(131072)+'-->':''}</body></html>`;
        if(run.variant==='http-gzip'){res.writeHead(200,{'content-type':'text/html; charset=utf-8','content-encoding':'gzip','cache-control':'no-store'});res.end(gzipSync(html));return}
        if(run.variant==='http-chunked'){res.writeHead(200,{'content-type':'text/html; charset=utf-8','cache-control':'no-store'});const middle=Math.floor(html.length/2);res.write(html.slice(0,middle));await new Promise(resolve=>setTimeout(resolve,12));if(!res.destroyed)res.end(html.slice(middle));return}
        return send(res, 200,html);
      }
      if (action === 'data') return send(res, 200, pageHtml(run, Number(url.searchParams.get('p') ?? 0)));
      if (action === 'ui') return send(res, 200, ui(run, crossOrigin));
      if (action === 'page-b' || action === 'popup' || action === 'inner') return send(res, 200, navigationChild(run, action));
      if (action === 'frame') return send(res, 200, `<input id="name"><select id="region"><option value="eu">EU</option></select><input id="consent" type="checkbox"><button id="submit">Save</button><script>document.querySelector('#submit').onclick=async e=>{await fetch('/runs/${id}/effect',{method:'POST',body:JSON.stringify({name:document.querySelector('#name').value,region:document.querySelector('#region').value,consent:document.querySelector('#consent').checked,trusted:e.isTrusted})});document.body.dataset.saved='true'};document.body.dataset.ready='true'</script>`);
      if (action === 'file') { res.setHeader('content-disposition', 'attachment; filename="catalog.txt"'); return send(res, 200, `automan-fixture:${run.seed}\n`, 'text/plain'); }
      return send(res, 404, 'unknown route');
    } catch { send(res, 500, 'fixture error'); }
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  // The same server under another host name is another site, so Chromium isolates its frames in their own process.
  const crossOrigin = `http://localhost:${(server.address() as AddressInfo).port}`;
  return {
    origin,
    async createRun(spec: RunSpec) {
      const id = randomBytes(12).toString('hex');
      const records = Array.from({ length: (spec.pages ?? 3) * 2 }, (_, i) => ({ id: `P-${String(i + 1).padStart(3, '0')}`, price: 1201 + i * 37 + (spec.seed === 42 ? 0 : spec.seed % 97) }));
      runs.set(id, { ...spec, id, records, effects: [], requests: 0, errors: 0, incidents: [] });
      return { id, url: `${origin}/runs/${id}/${spec.scenario.endsWith('catalog') ? 'catalog?p=0' : 'ui'}` };
    },
    async verify(id: string, output: { records?: RecordItem[]; file?: string; error?: string; [key: string]: unknown }): Promise<Verdict> {
      const run = runs.get(id); if (!run) throw new Error('unknown run'); const reasons: string[] = [];
      if (httpRefusals.includes(run.variant ?? '')) {
        if (output.error === undefined) {
          // Complete data after the deadline breaks the time contract; incomplete data reported as success is a false success.
          const got = new Set((output.records ?? []).map(record => record.id)); const missing = run.records.filter(record => !got.has(record.id)).length;
          reasons.push(missing ? `false success: ${missing} of ${run.records.length} records missing after ${run.variant}` : `complete data but the task deadline was ignored (${run.variant})`);
        }
      } else if (run.scenario.endsWith('catalog')) {
        if (output.error !== undefined) reasons.push(`failed: ${String(output.error).slice(0, 120)}`);
        if (run.variant === 'http-429') { const [limited, retried] = run.incidents; if (!retried) reasons.push('page not retried after 429'); else if (retried.at - limited!.at < 950) reasons.push(`retried ${Math.round(retried.at - limited!.at)} ms after a 429 asking for 1 s`); }
        const received = output.records ?? []; const byId = new Map(received.map(r => [r.id, r.price]));
        if (byId.size !== received.length) reasons.push('duplicate records');
        for (const expected of run.records) { if (!byId.has(expected.id)) reasons.push(`missing ${expected.id}`); else if (byId.get(expected.id) !== expected.price) reasons.push(`wrong price ${expected.id}`); }
        for (const record of received) if (!run.records.some(r => r.id === record.id)) reasons.push(`unexpected ${record.id}`);
      } else if (run.scenario === 'clicks') {
        if (run.effects.length !== (run.actions ?? 50)) reasons.push('wrong effect count');
        run.effects.forEach((effect, index) => { const e = effect as { count?: number; trusted?: boolean }; if (e.count !== index + 1 || !e.trusted) reasons.push('invalid click event'); });
      } else if(run.scenario==='actionability'){
        const negative=['never-enabled','never-uncovered','ambiguous'].includes(run.variant??'normal');
        const metrics=output.metrics as Record<string,number>|undefined;
        if(negative){if(run.effects.length)reasons.push('blocked or ambiguous target produced an effect');if(metrics?.nativeRejected!==1)reasons.push('operation reported success for a blocked or ambiguous target')}
        else {if(run.effects.length!==1)reasons.push('expected one target effect');const effect=run.effects[0] as {target?:string;phaseReady?:boolean;trusted?:boolean}|undefined;if(effect?.target!=='target'||effect.phaseReady!==true||effect.trusted!==true)reasons.push('wrong, premature or untrusted action effect');if(metrics?.nativeRejected!==0)reasons.push('actionable target was rejected')}
      } else if (run.scenario === 'primitives') {
        const variant = run.variant ?? 'hover'; const metrics = output.metrics as Record<string, number> | undefined;
        const effects = run.effects as { kind?: string; target?: string; trusted?: boolean; text?: string; value?: string; moves?: number }[];
        if (primitiveNegatives.includes(variant)) {
          if (effects.length) reasons.push(`retargeted control received an effect: ${effects.map(e => e.target).join(', ')}`);
          if (metrics?.nativeRejected !== 1) reasons.push('operation reported success after its target was replaced');
        } else {
          const expected = expectedPrimitive[variant]!;
          if (effects.some(e => e.target === 'decoy')) reasons.push('a decoy under the verified point received the action');
          const own = effects.filter(e => e.kind === expected.kind && e.target === expected.target);
          if (own.length !== 1 || effects.length !== 1) reasons.push(`expected exactly one ${expected.kind} effect on ${expected.target}, got ${effects.length}`);
          if (own[0] && own[0].trusted !== true) reasons.push('untrusted event');
          if (variant === 'contenteditable' && own[0]?.text !== 'Automan') reasons.push('wrong edited text');
          if (variant === 'unicode' && own[0]?.value !== unicodeText) reasons.push('wrong unicode value');
          if (variant === 'drag' && !(Number(own[0]?.moves) >= 1)) reasons.push('drop without pointer movement');
          if (metrics?.nativeRejected !== 0) reasons.push('actionable target was rejected');
        }
      } else if (run.scenario === 'navigation') {
        const variant = run.variant ?? 'spa'; const effects = run.effects as { kind?: string; trusted?: boolean; path?: string; page?: string; back?: boolean; origin?: string; name?: string; content?: string }[];
        const one = (kind: string) => { const own = effects.filter(e => e.kind === kind); if (own.length !== 1 || effects.length !== 1) reasons.push(`expected exactly one ${kind} effect, got ${effects.length}`); return own[0]; };
        if (variant === 'spa') { const e = one('route'); if (e && (!e.trusted || !e.path?.endsWith('/ui?view=detail'))) reasons.push('wrong or untrusted route change'); }
        else if (variant === 'history') { const e = one('page'); if (e && (e.page !== 'a' || e.back !== true)) reasons.push('page A was not restored by a history navigation'); }
        else if (variant === 'popup') { const e = one('popup-click'); if (e && !e.trusted) reasons.push('untrusted popup click'); }
        else if (variant === 'frame' || variant === 'cross-frame') {
          const e = one('frame-click'); const host = variant === 'cross-frame' ? 'localhost' : '127.0.0.1';
          if (e && (!e.trusted || !e.origin?.includes(host))) reasons.push(`wrong or untrusted click in the ${variant === 'cross-frame' ? 'cross-origin' : 'same-origin'} frame`);
        }
        else if (variant === 'upload') { const e = one('upload'); if (e && (e.name !== 'upload.txt' || e.content !== uploadText)) reasons.push('wrong uploaded file'); }
        else if (variant === 'download') { if (output.file !== `automan-fixture:${run.seed}\n`) reasons.push('wrong downloaded bytes'); if (effects.length) reasons.push('unexpected effect'); }
      } else if (run.scenario === 'download') {
        if (output.file !== `automan-fixture:${run.seed}\n`) reasons.push('wrong downloaded bytes');
      } else {
        if (run.effects.length !== 1) reasons.push('expected one form effect');
        const effect = run.effects[0] as { name?: string; region?: string; consent?: boolean; trusted?: boolean; events?: { type: string; id: string; trusted: boolean }[] } | undefined;
        if (effect?.name !== 'Automan' || effect.region !== 'eu' || effect.consent !== true || effect.trusted !== true) reasons.push('wrong form effect');
        if (run.scenario !== 'iframe' && !effect?.events?.some(e => e.id === 'name' && e.type === 'input' && e.trusted)) reasons.push('missing trusted input event');
      }
      return { status: reasons.length ? 'failed' : 'verified', reasons };
    },
    // An adapter error is the expected outcome for these runs; the lane hands it to verify instead of failing the cell.
    expectsRefusal(id: string) { return httpRefusals.includes(runs.get(id)?.variant ?? ''); },
    snapshot(id: string) { const run = runs.get(id); if (!run) throw new Error('unknown run'); return { requests: run.requests, effects: run.effects.length,...(run.scenario==='actionability'||run.scenario==='primitives'?{effectDetails:structuredClone(run.effects)}:{}) }; },
    async close() { server.closeAllConnections(); await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
  };
}
