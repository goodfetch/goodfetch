// Converts the WebMainBench 545-page subset (corpora/webmainbench.manifest.json) into the corpus layout the extraction lane
// reads, after checking the file against the manifest's SHA-256. Usage: node tools/prepare-webmainbench.ts <jsonl>
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { gzipSync } from 'node:zlib';
import { labRoot } from '../src/registry.ts';

const source = process.argv[2];
if (!source) throw new Error('usage: node tools/prepare-webmainbench.ts <WebMainBench_545.jsonl>');
const manifest = JSON.parse(await readFile(join(labRoot, 'corpora/webmainbench.manifest.json'), 'utf8')) as { sha256: string };
const bytes = await readFile(source);
const sha = createHash('sha256').update(bytes).digest('hex');
if (sha !== manifest.sha256) throw new Error(`SHA-256 ${sha} differs from the manifest`);

type Row = { track_id: string; html: string; url: string; groundtruth_content: string; meta: { language: string | null; level: string; table: string[]; code: string[]; equation: string[] } };
// Word F1 reads text: link and image targets are dropped, their visible text kept.
const plainOfMarkdown = (markdown: string) => markdown.replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1').replace(/\[([^\]]*)\]\([^)]*\)/g, '$1');

const root = join(labRoot, 'corpora/webmainbench/test');
await mkdir(join(root, 'html'), { recursive: true }); await mkdir(join(root, 'ground-truth'), { recursive: true });
const rows = bytes.toString('utf8').split('\n').filter(line => line.trim()).map(line => JSON.parse(line) as Row);
for (const [index, row] of rows.entries()) {
  const id = String(index + 1).padStart(4, '0');
  await writeFile(join(root, 'html', `${id}.html.gz`), gzipSync(row.html));
  await writeFile(join(root, 'ground-truth', `${id}.json`), JSON.stringify({ file_id: id, url: row.url, track_id: row.track_id,
    _internal: { page_type: { primary: row.meta.level }, language: row.meta.language, table: row.meta.table, code: row.meta.code, equation: row.meta.equation, source: 'webmainbench-545' },
    ground_truth: { main_content: plainOfMarkdown(row.groundtruth_content), main_markdown: row.groundtruth_content, with: [], without: [] } }));
}
process.stdout.write(`${rows.length} pages written to ${root}\n`);
