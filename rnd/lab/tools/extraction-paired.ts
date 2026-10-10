// Per-page paired analysis of an extraction run (D-extraction, D-markdown). The lane aggregates by page type or by shard;
// the page is the natural unit. The metric is recomputed per page from the saved outputs with the lane's own definition,
// and each extractor is compared with the baseline on the difference (not a log ratio, which would drop the pages where
// one side scores 0).
// Usage: node tools/extraction-paired.ts <run directory> <baseline extractor> <out.json> [f1|markdown]
//   f1        word F1 recomputed from predictions-*.json (the lane's wordF1)
//   markdown  per-page structure F1 read from markdown-*.json (src/extraction-markdown.ts); pages without structure skipped
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { loadTruth, wordF1, type ExtractionConfig } from '../src/extraction-campaign.ts';
import { wilcoxonSignedRank, hodgesLehmann } from '../src/stats/rank.ts';
import { blockBootstrapCI } from '../src/stats/bootstrap.ts';
import { quantiles } from '../src/statistics.ts';
import { labRoot } from '../src/registry.ts';

const [directory, baseline = 'scraper-text', out = 'extraction-paired.json', metric = 'f1'] = process.argv.slice(2);
if (!directory || !['f1', 'markdown'].includes(metric)) throw new Error('usage: node tools/extraction-paired.ts <run directory> <baseline> <out.json> [f1|markdown]');
const run = JSON.parse(await readFile(join(directory, 'run.v2.json'), 'utf8')) as { runId: string; campaign: { source: string } };
// The run names the campaign file it ran; the corpus and split come from it.
const { config } = JSON.parse(await readFile(join(labRoot, run.campaign.source), 'utf8')) as { config: ExtractionConfig };
const truth = new Map((await loadTruth(config)).map(page => [page.fileId, page]));

// <kind>-<extractor>-<pageType>[-s<shard>]-b<block>.json, renderer none only.
const pattern = (kind: string) => new RegExp(`^${kind}-(.+)-([a-z]+)(?:-s\\d+)?-b(\\d+)\\.json$`);
const scores = new Map<string, Map<string, number>>(); const times = new Map<string, Map<string, number>>();
for (const file of await readdir(directory)) {
  const kind = metric === 'f1' ? 'predictions' : 'markdown';
  const m = file.match(pattern(kind)); const t = file.match(pattern('timings'));
  if (t && !/^(chrome|lightpanda)-/.test(t[1]!)) { const byPage = times.get(t[1]!) ?? new Map<string, number>(); for (const [id, ms] of Object.entries(JSON.parse(await readFile(join(directory, file), 'utf8')) as Record<string, number>)) byPage.set(id, ms); times.set(t[1]!, byPage); }
  if (!m || /^(chrome|lightpanda)-/.test(m[1]!)) continue;
  const byPage = scores.get(m[1]!) ?? new Map<string, number>();
  if (metric === 'f1') {
    for (const [fileId, text] of Object.entries(JSON.parse(await readFile(join(directory, file), 'utf8')) as Record<string, string>)) { const page = truth.get(fileId); if (page) byPage.set(fileId, wordF1(text, page.mainContent).f1); }
  } else {
    for (const [fileId, score] of Object.entries(JSON.parse(await readFile(join(directory, file), 'utf8')) as Record<string, { structureF1: number | null }>)) if (score.structureF1 !== null) byPage.set(fileId, score.structureF1);
  }
  scores.set(m[1]!, byPage);
}
const base = scores.get(baseline); if (!base) throw new Error(`no ${metric} scores for ${baseline}`);
const mean = (values: number[]) => values.length ? values.reduce((a, b) => a + b, 0) / values.length : NaN;
const types = [...new Set([...truth.values()].map(page => page.pageType))].sort();
const perType = (byPage: Map<string, number>) => Object.fromEntries(types.map(type => { const values = [...byPage].filter(([id]) => truth.get(id)?.pageType === type).map(([, v]) => v); return [type, { pages: values.length, mean: values.length ? mean(values) : null }]; }));
const timing = (extractor: string) => { const own = times.get(extractor); if (!own) return null; const values = [...own.values()]; const q = quantiles(values); return { pages: values.length, p50Ms: q.p50, p95Ms: q.p95, totalMs: values.reduce((a, b) => a + b, 0) }; };
const comparisons = [...scores].filter(([name]) => name !== baseline).map(([extractor, byPage]) => {
  const pages = [...byPage.keys()].filter(id => base.has(id));
  const differences = pages.map(id => byPage.get(id)! - base.get(id)!);
  const ci = blockBootstrapCI(differences, { seed: 20261009, alpha: 0.05 });
  return { extractor, baseline, pages: pages.length, meanScore: mean(pages.map(id => byPage.get(id)!)), baselineMeanScore: mean(pages.map(id => base.get(id)!)),
    meanDifference: ci.estimate, ci95: ci.ci, hodgesLehmann: hodgesLehmann(differences), wilcoxon: wilcoxonSignedRank(differences),
    wins: differences.filter(d => d > 0).length, losses: differences.filter(d => d < 0).length, ties: differences.filter(d => d === 0).length };
});
const extractors = Object.fromEntries([...scores].map(([name, byPage]) => [name, { pages: byPage.size, meanScore: mean([...byPage.values()]), byPageType: perType(byPage), timing: timing(name) }]));
await writeFile(out, JSON.stringify({ schemaVersion: 2, runId: run.runId, unit: 'page', metric: metric === 'f1' ? 'word F1 (lab definition)' : 'Markdown structure F1 (src/extraction-markdown.ts)', baseline, extractors, comparisons }, null, 1) + '\n');
for (const c of comparisons) process.stdout.write(`${c.extractor} vs ${baseline}: ${c.pages} pages, ${metric} ${c.meanScore.toFixed(3)} vs ${c.baselineMeanScore.toFixed(3)}, Δ ${c.meanDifference.toFixed(3)} [${c.ci95.map(x => x.toFixed(3)).join(', ')}], HL ${c.hodgesLehmann.toFixed(3)}, Wilcoxon p=${c.wilcoxon.pValue.toExponential(2)}, ${c.wins}/${c.losses}/${c.ties}\n`);
