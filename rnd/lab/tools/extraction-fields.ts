// Field agreement and speed of a streaming field reader against the tree-building reference (D-extraction, field-precision
// and field-recall). A fields run (config.task 'fields') saves, per page, a JSON record of title, meta description, canonical
// link, og:title, h1 texts, a[href] and img[src]. The reference's values are the oracle: the decision's baseline is
// html5ever/scraper, a spec-conformant tree builder.
//   field-precision  candidate items equal to a reference item / candidate items
//   field-recall     reference items found by the candidate / reference items
// A scalar field is one item when present; list fields are compared as multisets. Time is the adapter's per-page extractMs,
// parsing included, file reading excluded, paired per page.
// Usage: node tools/extraction-fields.ts <run directory> <reference> <candidate> <out.json>
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { matchCounts } from '../src/extraction-markdown.ts';
import { blockBootstrapCI } from '../src/stats/bootstrap.ts';
import { quantiles } from '../src/statistics.ts';

const [directory = '', reference = 'scraper-fields', candidate = 'lol-html-fields', out = 'extraction-fields.json'] = process.argv.slice(2);
if (!directory) throw new Error('usage: node tools/extraction-fields.ts <run directory> <reference> <candidate> <out.json>');
const run = JSON.parse(await readFile(join(directory, 'run.v2.json'), 'utf8')) as { runId: string };
type Fields = Record<string, string | string[] | null>;
async function load(kind: string, extractor: string) {
  const merged: Record<string, unknown> = {};
  for (const file of await readdir(directory)) if (file.startsWith(`${kind}-${extractor}-`) && /-[a-z]+(?:-s\d+)?-b\d+\.json$/.test(file.slice(`${kind}-${extractor}`.length))) Object.assign(merged, JSON.parse(await readFile(join(directory, file), 'utf8')));
  return merged;
}
const parse = (text: unknown): Fields | null => { try { return JSON.parse(String(text)) as Fields; } catch { return null; } };
const [refOut, candOut, refMs, candMs] = await Promise.all([load('predictions', reference), load('predictions', candidate), load('timings', reference), load('timings', candidate)]);
const names = ['title', 'description', 'canonical', 'ogTitle', 'h1', 'links', 'images'];
const perField = Object.fromEntries(names.map(name => [name, { matched: 0, predicted: 0, expected: 0, pagesEqual: 0 }]));
let pages = 0, unreadable = 0, pagesAllEqual = 0; const ratios: number[] = []; const disagreements: { fileId: string; field: string; reference: unknown; candidate: unknown }[] = [];
for (const [fileId, text] of Object.entries(refOut)) {
  if (!(fileId in candOut)) continue; pages++;
  const ref = parse(text), cand = parse(candOut[fileId]); if (!ref || !cand) { unreadable++; continue; }
  let all = true;
  for (const name of names) {
    const items = (value: string | string[] | null | undefined) => value === null || value === undefined ? [] : Array.isArray(value) ? value : [value];
    const counts = matchCounts(items(cand[name]), items(ref[name])); const field = perField[name]!;
    field.matched += counts.matched; field.predicted += counts.predicted; field.expected += counts.expected;
    const equal = JSON.stringify(ref[name] ?? null) === JSON.stringify(cand[name] ?? null); if (equal) field.pagesEqual++; else { all = false; if (disagreements.length < 40) disagreements.push({ fileId, field: name, reference: Array.isArray(ref[name]) ? (ref[name] as string[]).length + ' items' : ref[name], candidate: Array.isArray(cand[name]) ? (cand[name] as string[]).length + ' items' : cand[name] }); }
  }
  if (all) pagesAllEqual++;
  const a = Number(candMs[fileId]), b = Number(refMs[fileId]); if (a > 0 && b > 0) ratios.push(Math.log(a / b));
}
const total = Object.values(perField).reduce((sum, f) => ({ matched: sum.matched + f.matched, predicted: sum.predicted + f.predicted, expected: sum.expected + f.expected }), { matched: 0, predicted: 0, expected: 0 });
const ci = ratios.length ? blockBootstrapCI(ratios, { seed: 20261010, alpha: 0.05 }) : null;
const ms = (values: unknown[]) => { const q = quantiles(values.map(Number)); return { p50: q.p50, p95: q.p95, total: values.reduce<number>((a, b) => a + Number(b), 0) }; };
const result = { schemaVersion: 1, runId: run.runId, reference, candidate, pages, unreadable, pagesAllFieldsEqual: pagesAllEqual,
  fieldPrecision: total.predicted ? total.matched / total.predicted : null, fieldRecall: total.expected ? total.matched / total.expected : null,
  perField: Object.fromEntries(Object.entries(perField).map(([name, f]) => [name, { ...f, precision: f.predicted ? f.matched / f.predicted : null, recall: f.expected ? f.matched / f.expected : null }])),
  time: { reference: ms(Object.values(refMs)), candidate: ms(Object.values(candMs)), pairedPages: ratios.length, geometricMeanRatio: ci ? Math.exp(ci.estimate) : null, ratioCi95: ci ? ci.ci.map(Math.exp) : null },
  disagreements };
await writeFile(out, JSON.stringify(result, null, 1) + '\n');
process.stdout.write(`${candidate} vs ${reference}: ${pages} pages, all fields equal on ${pagesAllEqual}, precision ${result.fieldPrecision?.toFixed(4)}, recall ${result.fieldRecall?.toFixed(4)}, time ratio ${result.time.geometricMeanRatio?.toFixed(3)} [${result.time.ratioCi95?.map(x => x.toFixed(3)).join(', ')}]\n`);
