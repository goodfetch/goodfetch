// Main-content extraction quality on the WCXB corpus (CC-BY-4.0): word-level F1 against the ground truth, share of
// required snippets found and of boilerplate snippets leaked, as defined by the corpus's own evaluate.py.
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { basename, join } from 'node:path';
import { scoreMarkdown, elementTypes, f1Of, type Counts } from './extraction-markdown.ts';
import { WorkerClient } from './worker-client.ts';
import { commandFor, labRoot, type BackendName } from './registry.ts';
import { quantiles } from './statistics.ts';
import { gzipSync } from 'node:zlib';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { startReplayProxy, replayUrl } from '../fixtures/replay-proxy.ts';
import { launchBrowser, launchLightpanda, lightpandaPath, type HostedBrowser } from './core/browser-host.ts';
import { browserPath } from './environment.ts';
import { CdpConnection, CdpPage } from '../adapters/cdp-node/cdp.ts';

export type Renderer = 'none' | 'chrome' | 'lightpanda';
// shards: the pages of the selected types are spread over that many shards (stratified by type) and a cell is one extractor
// over one shard, so that the verdict pairs enough blocks; pageType is then 'all'. markdown: the cell also scores the
// output as Markdown against ground_truth.main_markdown (src/extraction-markdown.ts). task 'fields': the output is a JSON
// record of structural fields, compared afterwards between parsers (tools/extraction-fields.ts); no F1 is computed.
export type ExtractionConfig = { split: 'dev' | 'test'; extractors: BackendName[]; pageTypes: string[]; blocks: number; seed: number; corpus?: string; renderers?: Renderer[]; renderTimeoutMs?: number; shards?: number; markdown?: boolean; task?: 'main-content' | 'fields' };
export type ExtractionCell = { extractor: BackendName; pageType: string; renderer: Renderer; corpus: string; shard?: number };
export const corpusRoot = (config: ExtractionConfig) => join(labRoot, config.corpus ?? 'corpora/wcxb');
export const corpusName = (config: ExtractionConfig) => basename(config.corpus ?? 'corpora/wcxb');
// A corpus page is stored gzipped (WCXB, WebMainBench) or plain (the curated fixtures, kept readable in git).
export const pagePath = (config: ExtractionConfig, fileId: string) => { const plain = join(corpusRoot(config), config.split, 'html', `${fileId}.html`); return existsSync(plain) ? plain : `${plain}.gz`; };

// Python's \w on str is Unicode letters, digits and underscore; this is its JavaScript equivalent.
export const tokenize = (text: string) => (text.toLowerCase().match(/[\p{L}\p{N}_]+/gu) ?? []);
export function wordF1(predicted: string, reference: string) {
  const prediction = tokenize(predicted), truth = tokenize(reference);
  if (!truth.length) return prediction.length ? { precision: 0, recall: 0, f1: 0 } : { precision: 1, recall: 1, f1: 1 };
  if (!prediction.length) return { precision: 0, recall: 0, f1: 0 };
  const counts = new Map<string, number>(); for (const token of truth) counts.set(token, (counts.get(token) ?? 0) + 1);
  let overlap = 0; for (const token of prediction) { const left = counts.get(token) ?? 0; if (left > 0) { overlap++; counts.set(token, left - 1); } }
  const precision = overlap / prediction.length, recall = overlap / truth.length;
  return { precision, recall, f1: precision + recall > 0 ? 2 * precision * recall / (precision + recall) : 0 };
}
export const snippetRate = (text: string, snippets: string[]) => snippets.length ? snippets.filter(snippet => text.toLowerCase().includes(snippet.toLowerCase())).length / snippets.length : 1;

type Truth = { fileId: string; url: string; pageType: string; mainContent: string; mainMarkdown: string | null; with: string[]; without: string[] };
export async function loadTruth(config: ExtractionConfig): Promise<Truth[]> {
  const directory = join(corpusRoot(config), config.split, 'ground-truth');
  const files = (await readdir(directory)).filter(name => name.endsWith('.json')).sort();
  return Promise.all(files.map(async name => {
    const data = JSON.parse(await readFile(join(directory, name), 'utf8')) as { file_id: string; url: string; _internal?: { page_type?: { primary?: string } | string }; ground_truth: { main_content: string; main_markdown?: string; with?: string[]; without?: string[] } };
    const raw = typeof data._internal?.page_type === 'string' ? data._internal.page_type : data._internal?.page_type?.primary ?? 'article';
    return { fileId: data.file_id, url: data.url, pageType: raw === 'category' ? 'collection' : raw, mainContent: data.ground_truth.main_content ?? '', mainMarkdown: data.ground_truth.main_markdown ?? null, with: data.ground_truth.with ?? [], without: data.ground_truth.without ?? [] };
  }));
}

export function extractionCombinations(config: ExtractionConfig): ExtractionCell[] {
  const corpus = corpusName(config);
  if (config.shards) { const shards = config.shards; return (config.renderers ?? ['none']).flatMap(renderer => config.extractors.flatMap(extractor => Array.from({ length: shards }, (_, shard) => ({ extractor, pageType: 'all', renderer, corpus, shard })))); }
  return (config.renderers ?? ['none']).flatMap(renderer => config.extractors.flatMap(extractor => config.pageTypes.map(pageType => ({ extractor, pageType, renderer, corpus }))));
}
// Pages of a cell: one page type, or one shard. Shards take the selected pages grouped by type, in file order, round-robin.
export async function cellPages(config: ExtractionConfig, cell: Pick<ExtractionCell, 'pageType' | 'shard'>) {
  const truth = await loadTruth(config);
  if (cell.shard === undefined) return truth.filter(page => page.pageType === cell.pageType);
  const selected = config.pageTypes.flatMap(type => truth.filter(page => page.pageType === type));
  return selected.filter((_, index) => index % config.shards! === cell.shard);
}

// A browser whose every request goes to the replay proxy: Chrome cannot resolve or reach any other host, Lightpanda blocks every
// address but loopback, so a frozen page loads only what the archive holds.
async function renderingEngine(renderer: Exclude<Renderer, 'none'>, proxy: string): Promise<HostedBrowser> {
  if (renderer === 'chrome') return launchBrowser(browserPath(), 30_000, [`--proxy-server=${proxy}`, '--proxy-bypass-list=<-loopback>', '--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1', '--disable-features=HttpsUpgrades,HttpsFirstBalancedModeAutoEnable,HttpsFirstModeV2ForEngagedSites']);
  return launchLightpanda(lightpandaPath(labRoot), 15_000, ['--http-proxy', proxy, '--block-cidrs', '0.0.0.0/0', '--block-cidrs', '-127.0.0.1/32']);
}
type Rendered = { path: string | null; status: 'ok' | 'timeout' | 'error'; loadMs: number; misses: number; error?: string };
async function renderPages(renderer: Exclude<Renderer, 'none'>, pages: Truth[], config: ExtractionConfig, scratch: string): Promise<{ rendered: Map<string, Rendered>; hits: number; misses: number; tunnels: number; renderedBytes: number }> {
  const proxy = await startReplayProxy(pages.map(page => ({ url: page.url, fileId: page.fileId, path: join(corpusRoot(config), config.split, 'html', `${page.fileId}.html.gz`) })));
  const rendered = new Map<string, Rendered>(); const timeoutMs = config.renderTimeoutMs ?? 10_000; let renderedBytes = 0;
  // Chrome accepts the lab's local certificate for the archived hosts it must load over HTTPS.
  const connect = async (hosted: HostedBrowser) => { const c = await CdpConnection.connect(hosted.wsEndpoint); if (renderer === 'chrome') await c.send('Security.setIgnoreCertificateErrors', { ignore: true }); return c; };
  let engine = await renderingEngine(renderer, proxy.url); let connection = await connect(engine);
  try {
    for (const page of pages) {
      proxy.loading(page.url); const began = performance.now(); let tab: CdpPage | null = null;
      try {
        tab = await CdpPage.open(connection, timeoutMs);
        await tab.goto(replayUrl(page.url, page.fileId));
        // Inline scripts have run by DOMContentLoaded; external ones are refused at once, so a short settle is enough.
        await new Promise(resolve => setTimeout(resolve, 200));
        const html = await Promise.race([tab.evaluate<string>('document.documentElement.outerHTML'), new Promise<never>((_, reject) => setTimeout(() => reject(new Error('timeout')), timeoutMs))]);
        const path = join(scratch, `${renderer}-${page.fileId}.html.gz`); await writeFile(path, gzipSync(html)); renderedBytes += html.length;
        rendered.set(page.fileId, { path, status: 'ok', loadMs: performance.now() - began, misses: proxy.stats.missesByPage.get(page.url) ?? 0 });
        await tab.close().catch(() => {});
      } catch (error) {
        rendered.set(page.fileId, { path: null, status: /timeout/i.test(String(error)) ? 'timeout' : 'error', loadMs: performance.now() - began, misses: proxy.stats.missesByPage.get(page.url) ?? 0, error: `${page.fileId}: ${String(error).slice(0, 120)}` });
        // A failed load can leave the engine unusable; the next page starts on a fresh one.
        connection.close(); await engine.close(); engine = await renderingEngine(renderer, proxy.url); connection = await connect(engine);
      }
    }
  } finally { connection.close(); await engine.close(); await proxy.close(); }
  return { rendered, hits: proxy.stats.hits, misses: proxy.stats.misses, tunnels: proxy.stats.tunnels, renderedBytes };
}

// One cell: one extractor over every page of one type in the split, through a single adapter process.
export async function runExtractionCell(config: ExtractionConfig, cell: ExtractionCell, block: number, directory: string): Promise<Record<string, unknown>> {
  const pages = await cellPages(config, cell);
  const scratch = await mkdtemp(join(tmpdir(), 'automan-render-'));
  const render = cell.renderer === 'none' ? null : await renderPages(cell.renderer, pages, config, scratch);
  const worker = new WorkerClient(commandFor(cell.extractor, 60_000)); const predictions: Record<string, string> = {};
  const scores: { f1: number; precision: number; recall: number; withRate: number; leak: number; ms: number }[] = []; let errors = 0;
  const timings: Record<string, number> = {}; const markdown: Record<string, ReturnType<typeof scoreMarkdown>> = {}; const fields = config.task === 'fields';
  try {
    await worker.call('hello', {}); await worker.call('prepare', {});
    for (const page of pages) {
      let text = '', ms = 0;
      // A page the engine failed to render scores zero: the extractor never sees it.
      const rendered = render?.rendered.get(page.fileId);
      if (rendered && !rendered.path) { predictions[page.fileId] = ''; scores.push({ ...wordF1('', page.mainContent), withRate: snippetRate('', page.with), leak: 0, ms: 0 }); continue; }
      try { const output = await worker.call('run', { path: rendered?.path ?? pagePath(config, page.fileId), url: page.url }) as { text?: string; metrics?: { extractMs?: number } }; text = output.text ?? ''; ms = output.metrics?.extractMs ?? 0; }
      catch { errors++; }
      predictions[page.fileId] = text; timings[page.fileId] = ms;
      if (config.markdown) { if (page.mainMarkdown === null) throw new Error(`${page.fileId}: the ground truth has no main_markdown`); markdown[page.fileId] = scoreMarkdown(text, page.mainMarkdown); }
      scores.push(fields ? { f1: 0, precision: 0, recall: 0, withRate: 0, leak: 0, ms } : { ...wordF1(text, page.mainContent), withRate: snippetRate(text, page.with), leak: page.without.length ? snippetRate(text, page.without) : 0, ms });
    }
  } finally { await worker.close(); await rm(scratch, { recursive: true, force: true }); }
  const suffix = `${cell.renderer === 'none' ? '' : `${cell.renderer}-`}${cell.extractor}-${cell.pageType}${cell.shard === undefined ? '' : `-s${String(cell.shard).padStart(2, '0')}`}-b${block}.json`;
  await writeFile(join(directory, `predictions-${suffix}`), JSON.stringify(predictions));
  await writeFile(join(directory, `timings-${suffix}`), JSON.stringify(timings));
  if (config.markdown) await writeFile(join(directory, `markdown-${suffix}`), JSON.stringify(markdown));
  const mean = (key: keyof (typeof scores)[number]) => scores.length && !(fields && key !== 'ms') ? scores.reduce((sum, score) => sum + score[key], 0) / scores.length : null;
  // Markdown: per-page structure F1 averaged over the pages whose ground truth has structure; per-type F1 pooled over the cell.
  const pageScores = Object.values(markdown);
  const structured = pageScores.map(score => score.structureF1).filter((value): value is number => value !== null);
  const pooled = (type: (typeof elementTypes)[number]) => { const sum = pageScores.reduce<Counts>((acc, score) => ({ matched: acc.matched + score.counts[type].matched, predicted: acc.predicted + score.counts[type].predicted, expected: acc.expected + score.counts[type].expected }), { matched: 0, predicted: 0, expected: 0 }); return sum.expected ? f1Of(sum) : null; };
  const markdownStats = config.markdown ? { structuredPages: structured.length, mdStructureF1: structured.length ? structured.reduce((a, b) => a + b, 0) / structured.length : null,
    mdHeadingF1: pooled('heading'), mdListF1: pooled('list'), mdTableF1: pooled('table'), mdCodeF1: pooled('code'), mdLinkF1: pooled('link') } : {};
  const renderStats = render ? { renderOk: [...render.rendered.values()].filter(r => r.status === 'ok').length, renderTimeouts: [...render.rendered.values()].filter(r => r.status === 'timeout').length, renderErrors: [...render.rendered.values()].filter(r => r.status === 'error').length,
    renderMsP50: quantiles([...render.rendered.values()].filter(r => r.status === 'ok').map(r => r.loadMs)).p50, archiveHits: render.hits, refusedRequests: render.misses, refusedTunnels: render.tunnels, renderedBytes: render.renderedBytes, renderFailures: [...render.rendered.values()].filter(r => r.error).map(r => r.error).slice(0, 10) } : {};
  return { ...cell, block, status: 'verified', pages: pages.length, ...renderStats, extractErrors: errors, ...markdownStats, meanF1: mean('f1'), meanPrecision: mean('precision'), meanRecall: mean('recall'), snippetRecall: mean('withRate'), boilerplateLeak: mean('leak'), extractMsP50: quantiles(scores.map(score => score.ms)).p50 };
}
