// Main-content extraction quality on the WCXB corpus (CC-BY-4.0): word-level F1 against the ground truth, share of
// required snippets found and of boilerplate snippets leaked, as defined by the corpus's own evaluate.py.
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { WorkerClient } from './worker-client.ts';
import { commandFor, labRoot, type BackendName } from './registry.ts';
import { quantiles } from './statistics.ts';

export type ExtractionConfig = { split: 'dev' | 'test'; extractors: BackendName[]; pageTypes: string[]; blocks: number; seed: number; corpus?: string };
export type ExtractionCell = { extractor: BackendName; pageType: string };
export const corpusRoot = (config: ExtractionConfig) => join(labRoot, config.corpus ?? 'corpora/wcxb');

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

type Truth = { fileId: string; url: string; pageType: string; mainContent: string; with: string[]; without: string[] };
export async function loadTruth(config: ExtractionConfig): Promise<Truth[]> {
  const directory = join(corpusRoot(config), config.split, 'ground-truth');
  const files = (await readdir(directory)).filter(name => name.endsWith('.json')).sort();
  return Promise.all(files.map(async name => {
    const data = JSON.parse(await readFile(join(directory, name), 'utf8')) as { file_id: string; url: string; _internal?: { page_type?: { primary?: string } | string }; ground_truth: { main_content: string; with?: string[]; without?: string[] } };
    const raw = typeof data._internal?.page_type === 'string' ? data._internal.page_type : data._internal?.page_type?.primary ?? 'article';
    return { fileId: data.file_id, url: data.url, pageType: raw === 'category' ? 'collection' : raw, mainContent: data.ground_truth.main_content ?? '', with: data.ground_truth.with ?? [], without: data.ground_truth.without ?? [] };
  }));
}

export function extractionCombinations(config: ExtractionConfig): ExtractionCell[] { return config.extractors.flatMap(extractor => config.pageTypes.map(pageType => ({ extractor, pageType }))); }

// One cell: one extractor over every page of one type in the split, through a single adapter process.
export async function runExtractionCell(config: ExtractionConfig, cell: ExtractionCell, block: number, directory: string): Promise<Record<string, unknown>> {
  const pages = (await loadTruth(config)).filter(page => page.pageType === cell.pageType);
  const worker = new WorkerClient(commandFor(cell.extractor, 60_000)); const predictions: Record<string, string> = {};
  const scores: { f1: number; precision: number; recall: number; withRate: number; leak: number; ms: number }[] = []; let errors = 0;
  try {
    await worker.call('hello', {}); await worker.call('prepare', {});
    for (const page of pages) {
      let text = '', ms = 0;
      try { const output = await worker.call('run', { path: join(corpusRoot(config), config.split, 'html', `${page.fileId}.html.gz`), url: page.url }) as { text?: string; metrics?: { extractMs?: number } }; text = output.text ?? ''; ms = output.metrics?.extractMs ?? 0; }
      catch { errors++; }
      predictions[page.fileId] = text;
      scores.push({ ...wordF1(text, page.mainContent), withRate: snippetRate(text, page.with), leak: page.without.length ? snippetRate(text, page.without) : 0, ms });
    }
  } finally { await worker.close(); }
  await writeFile(join(directory, `predictions-${cell.extractor}-${cell.pageType}-b${block}.json`), JSON.stringify(predictions));
  const mean = (key: keyof (typeof scores)[number]) => scores.length ? scores.reduce((sum, score) => sum + score[key], 0) / scores.length : null;
  return { ...cell, block, status: 'verified', pages: pages.length, extractErrors: errors, meanF1: mean('f1'), meanPrecision: mean('precision'), meanRecall: mean('recall'), snippetRecall: mean('withRate'), boilerplateLeak: mean('leak'), extractMsP50: quantiles(scores.map(score => score.ms)).p50 };
}
