import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { randomBytes } from 'node:crypto';
import type { AddressInfo } from 'node:net';

export type RecordItem = { id: string; price: number };
export type Verdict = { status: 'verified' | 'failed'; reasons: string[] };
export type Scenario = 'http-catalog' | 'js-catalog' | 'form' | 'clicks' | 'shadow' | 'iframe' | 'download';
export type RunSpec = { scenario: Scenario; seed: number; pages?: number; actions?: number; delayMs?: number; variant?: 'normal' | 'overlay' | 'rerender' | 'delayed' };
type Run = RunSpec & { id: string; records: RecordItem[]; effects: unknown[]; requests: number; errors: number };

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
function ui(run: Run) {
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
      if (action === 'catalog') {
        const p = Number(url.searchParams.get('p') ?? 0);
        if (!Number.isInteger(p) || p < 0 || p >= (run.pages ?? 3)) return send(res, 404, 'bad page');
        if (run.scenario === 'js-catalog') return send(res, 200, `<!doctype html><div id="loading">Loading</div><script>fetch('/runs/${id}/data?p=${p}').then(r=>r.text()).then(html=>{document.body.innerHTML=html})</script>`);
        return send(res, 200, `<!doctype html><html><body>${pageHtml(run, p)}</body></html>`);
      }
      if (action === 'data') return send(res, 200, pageHtml(run, Number(url.searchParams.get('p') ?? 0)));
      if (action === 'ui') return send(res, 200, ui(run));
      if (action === 'frame') return send(res, 200, `<input id="name"><select id="region"><option value="eu">EU</option></select><input id="consent" type="checkbox"><button id="submit">Save</button><script>document.querySelector('#submit').onclick=async e=>{await fetch('/runs/${id}/effect',{method:'POST',body:JSON.stringify({name:document.querySelector('#name').value,region:document.querySelector('#region').value,consent:document.querySelector('#consent').checked,trusted:e.isTrusted})});document.body.dataset.saved='true'};document.body.dataset.ready='true'</script>`);
      if (action === 'file') { res.setHeader('content-disposition', 'attachment; filename="catalog.txt"'); return send(res, 200, `automan-fixture:${run.seed}\n`, 'text/plain'); }
      return send(res, 404, 'unknown route');
    } catch { send(res, 500, 'fixture error'); }
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  return {
    origin,
    async createRun(spec: RunSpec) {
      const id = randomBytes(12).toString('hex');
      const records = Array.from({ length: (spec.pages ?? 3) * 2 }, (_, i) => ({ id: `P-${String(i + 1).padStart(3, '0')}`, price: 1201 + i * 37 + (spec.seed === 42 ? 0 : spec.seed % 97) }));
      runs.set(id, { ...spec, id, records, effects: [], requests: 0, errors: 0 });
      return { id, url: `${origin}/runs/${id}/${spec.scenario.endsWith('catalog') ? 'catalog?p=0' : 'ui'}` };
    },
    async verify(id: string, output: { records?: RecordItem[]; file?: string; [key: string]: unknown }): Promise<Verdict> {
      const run = runs.get(id); if (!run) throw new Error('unknown run'); const reasons: string[] = [];
      if (run.scenario.endsWith('catalog')) {
        const received = output.records ?? []; const byId = new Map(received.map(r => [r.id, r.price]));
        if (byId.size !== received.length) reasons.push('duplicate records');
        for (const expected of run.records) { if (!byId.has(expected.id)) reasons.push(`missing ${expected.id}`); else if (byId.get(expected.id) !== expected.price) reasons.push(`wrong price ${expected.id}`); }
        for (const record of received) if (!run.records.some(r => r.id === record.id)) reasons.push(`unexpected ${record.id}`);
      } else if (run.scenario === 'clicks') {
        if (run.effects.length !== (run.actions ?? 50)) reasons.push('wrong effect count');
        run.effects.forEach((effect, index) => { const e = effect as { count?: number; trusted?: boolean }; if (e.count !== index + 1 || !e.trusted) reasons.push('invalid click event'); });
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
    snapshot(id: string) { const run = runs.get(id); if (!run) throw new Error('unknown run'); return { requests: run.requests, effects: run.effects.length }; },
    async close() { server.closeAllConnections(); await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
  };
}
