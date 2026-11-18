// The Firefox cells of the Guard's conformance matrix over WebDriver BiDi (issue 2460): the three cross-engine
// campaigns (actionability, primitives, navigation) with the same variants, seeds, timeouts and blocks, on the
// installed stock Firefox, guarded, through the same fixtures and private oracle as every other driver.
// Usage: node src/firefox-bidi-campaign.ts [blocks]; the Firefox is $FIREFOX_BIN, else `firefox` on the PATH.
import { execFileSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { randomBytes } from 'node:crypto';
import { runActionabilityCell, actionabilityCombinations, type ActionabilityConfig } from './actionability-campaign.ts';
import { labRoot } from './registry.ts';
import { environment } from './environment.ts';
import { shuffle } from './statistics.ts';

const campaigns = ['cross-engine-actionability', 'cross-engine-primitives', 'cross-engine-navigation'];

function firefoxPath(): string {
  if (process.env.FIREFOX_BIN) return process.env.FIREFOX_BIN;
  return execFileSync('sh', ['-c', 'command -v firefox'], { encoding: 'utf8' }).trim();
}

export async function firefoxBidiCampaign(blocks?: number) {
  const executablePath = firefoxPath();
  const version = execFileSync(executablePath, ['--version'], { encoding: 'utf8' }).trim();
  const id = `firefox-bidi-guard-${new Date().toISOString().replace(/[:.]/g, '-')}-${randomBytes(3).toString('hex')}`;
  const directory = join(labRoot, 'results', id); await mkdir(directory, { recursive: true });
  const env = await environment().catch(error => ({ unavailable: String(error) }));
  const cells: Record<string, unknown>[] = [];
  const configs: ActionabilityConfig[] = [];
  for (const name of campaigns) {
    const campaign = JSON.parse(await readFile(join(labRoot, 'campaigns/v2/actionability', `${name}.json`), 'utf8')) as { config: ActionabilityConfig };
    configs.push({ ...campaign.config, blocks: blocks ?? campaign.config.blocks, backends: ['firefox-bidi'], policies: ['guarded'] });
  }
  await writeFile(join(directory, 'manifest.json'), JSON.stringify({ id, firefox: { executablePath, version }, campaigns, configs, environment: env, methodology: 'the cross-engine campaigns with backend firefox-bidi and the guarded policy: fresh worker and Firefox per cell, the Guard\'s check.js and disarm.js from crates/goodfetch-page in a WebDriver BiDi sandbox, every effect checked by the fixtures\' private oracle' }, null, 2) + '\n');
  for (const config of configs) for (let block = 0; block < config.blocks; block++) for (const cell of shuffle(actionabilityCombinations(config), config.seed + block)) {
    cells.push(await runActionabilityCell(config, cell, block, executablePath));
    await writeFile(join(directory, 'summary.json'), JSON.stringify({ cells }, null, 2) + '\n');
    const last = cells.at(-1)!;
    process.stderr.write(`${cell.scenario}/${cell.variant}/b${block}: ${String(last.status)}${last.status === 'verified' ? '' : ` ${JSON.stringify(last.reasons)}`}\n`);
  }
  return { directory, cells };
}

if (process.argv[1]?.endsWith('firefox-bidi-campaign.ts')) {
  const { directory, cells } = await firefoxBidiCampaign(process.argv[2] === undefined ? undefined : Number(process.argv[2]));
  const held = cells.filter(cell => cell.status === 'verified').length;
  process.stdout.write(JSON.stringify({ directory, held, cells: cells.length, failed: cells.filter(cell => cell.status !== 'verified').map(cell => `${String(cell.scenario)}/${String(cell.variant)}/b${String(cell.block)}`) }) + '\n');
}
