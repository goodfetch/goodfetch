import { readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { runCampaign, validateCampaign } from './campaign.ts';
import { writeReport } from './report.ts';
import { labRoot } from './registry.ts';
const args = process.argv.slice(2);
try {
  if (args[0] === 'report') {
    if (!args[1]) throw new Error('usage: pnpm report <results-directory>');
    const directory = resolve(args[1]); await writeReport(directory, JSON.parse(await readFile(join(directory, 'summary.json'), 'utf8'))); console.log(join(directory, 'report.html'));
  } else {
    const configPath = resolve(args[0] ?? join(labRoot, 'campaigns/smoke.json'));
    const config: unknown = JSON.parse(await readFile(configPath, 'utf8')); validateCampaign(config);
    if (args.includes('--backends')) { const input = args[args.indexOf('--backends') + 1]; if (!input) throw new Error('missing backend list'); config.backends = input.split(',') as typeof config.backends; validateCampaign(config); }
    const result = await runCampaign(config); console.log(result.directory);
    if (result.rows.some(row => ['failed','timeout','crashed'].includes(row.status)) || result.summary.sourceChanges.length) process.exitCode = 1;
  }
} catch (error) { console.error(error instanceof Error ? error.message : error); process.exitCode = 1; }
