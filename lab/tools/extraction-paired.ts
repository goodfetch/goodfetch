// Per-page paired analysis of an extraction run (D-extraction). The lane aggregates F1 by page type, which leaves seven blocks; the
// page is the natural unit. F1 is recomputed per page from the saved predictions with the lane's own definition, and each extractor
// is compared with the baseline on the difference of F1 (not a log ratio, which would drop the pages where one side scores 0).
// Usage: node tools/extraction-paired.ts <run directory> <baseline extractor> <out.json>
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { loadTruth, wordF1, type ExtractionConfig } from '../src/extraction-campaign.ts';
import { wilcoxonSignedRank, hodgesLehmann } from '../src/stats/rank.ts';
import { blockBootstrapCI } from '../src/stats/bootstrap.ts';
import { labRoot } from '../src/registry.ts';

const [directory, baseline = 'scraper-text', out = 'extraction-paired.json'] = process.argv.slice(2);
if (!directory) throw new Error('usage: node tools/extraction-paired.ts <run directory> <baseline> <out.json>');
const run = JSON.parse(await readFile(join(directory, 'run.v2.json'), 'utf8')) as { runId: string; campaign: { source: string } };
// The run names the campaign file it ran; the corpus and split come from it.
const { config } = JSON.parse(await readFile(join(labRoot, run.campaign.source), 'utf8')) as { config: ExtractionConfig };
const truth = new Map((await loadTruth(config)).map(page => [page.fileId, page]));

// predictions-<extractor>-<pageType>-b<block>.json, renderer none only.
const scores = new Map<string, Map<string, number>>();
for (const file of await readdir(directory)) {
  const m = file.match(/^predictions-(.+)-([a-z]+)-b(\d+)\.json$/); if (!m || /^(chrome|lightpanda)-/.test(m[1]!)) continue;
  const predictions = JSON.parse(await readFile(join(directory, file), 'utf8')) as Record<string, string>;
  const byPage = scores.get(m[1]!) ?? new Map<string, number>();
  for (const [fileId, text] of Object.entries(predictions)) { const page = truth.get(fileId); if (page) byPage.set(fileId, wordF1(text, page.mainContent).f1); }
  scores.set(m[1]!, byPage);
}
const base = scores.get(baseline); if (!base) throw new Error(`no predictions for ${baseline}`);
const comparisons = [...scores].filter(([name]) => name !== baseline).map(([extractor, byPage]) => {
  const pages = [...byPage.keys()].filter(id => base.has(id));
  const differences = pages.map(id => byPage.get(id)! - base.get(id)!);
  const ci = blockBootstrapCI(differences, { seed: 20261009, alpha: 0.05 });
  const mean = (values: number[]) => values.reduce((a, b) => a + b, 0) / values.length;
  return { extractor, baseline, pages: pages.length, meanF1: mean(pages.map(id => byPage.get(id)!)), baselineMeanF1: mean(pages.map(id => base.get(id)!)),
    meanDifference: ci.estimate, ci95: ci.ci, hodgesLehmann: hodgesLehmann(differences), wilcoxon: wilcoxonSignedRank(differences),
    wins: differences.filter(d => d > 0).length, losses: differences.filter(d => d < 0).length, ties: differences.filter(d => d === 0).length };
});
await writeFile(out, JSON.stringify({ schemaVersion: 1, runId: run.runId, unit: 'page', metric: 'word F1 (lab definition)', baseline, comparisons }, null, 1) + '\n');
for (const c of comparisons) process.stdout.write(`${c.extractor} vs ${baseline}: ${c.pages} pages, F1 ${c.meanF1.toFixed(3)} vs ${c.baselineMeanF1.toFixed(3)}, Δ ${c.meanDifference.toFixed(3)} [${c.ci95.map(x => x.toFixed(3)).join(', ')}], HL ${c.hodgesLehmann.toFixed(3)}, Wilcoxon p=${c.wilcoxon.pValue.toExponential(2)}, ${c.wins}/${c.losses}/${c.ties}\n`);
