// Lane drivers and the generic lane engine: each lane keeps its own cell executor and oracle,
// the engine adds provenance, host gates, randomized blocks, typed statuses and a run.v2 document.
import { appendFile } from 'node:fs/promises';
import { join } from 'node:path';
import { labRoot, backendNames } from '../registry.ts';
import { shuffle } from '../statistics.ts';
import { browserPath, hashFile } from '../environment.ts';
import { validateDocument } from './schema.ts';
import { openRun, type RunOptions } from './run-context.ts';
import { cellsCell, type Json, type Lane, type ImportedCell } from './legacy-cells.ts';
import type { HostClass } from './host.ts';
import { validateStorageConfig, storageCombinations, runStorageCell, type StorageConfig, type StorageCell, type StorageCellShared } from '../storage-campaign.ts';
import { validateWorkflowCampaign, workflowCombinations, runWorkflowCell, type WorkflowCampaign, type WorkflowCell } from '../workflow-campaign.ts';
import { runRuntimeCell, type RuntimeCampaign } from '../runtime-campaign.ts';
import { actionabilityCombinations, runActionabilityCell, type ActionabilityConfig, type ActionabilityCell } from '../actionability-campaign.ts';
import { readersMatrix, runReadersCell, type ReadersConfig } from '../sqlite-readers-campaign.ts';
import { isSharedEngine, sharedBuildMetadata } from '../storage-engines.ts';

export type LaneName = Exclude<Lane, 'bench' | 'suite'>;
export type LaneCampaign = { schemaVersion: 2; id: string; lane: LaneName; decisionIds: string[]; hostClass: { requested: HostClass; allowDowngrade: boolean }; config: Record<string, unknown> };
// Cells and contexts are lane-specific; the engine treats them as opaque records.
export type LaneDriver = {
  validate(config: unknown): void; blocks(config: unknown): number; seed(config: unknown): number;
  combinations(config: unknown): Json[]; label(cell: Json): string;
  prepare?(config: unknown): Promise<unknown>;
  runCell(config: unknown, cell: Json, block: number, directory: string, context: unknown): Promise<Json>;
  binaryChanges?(config: unknown, context: unknown): Promise<number>;
};

const integer = (value: unknown, min: number, max: number) => Number.isInteger(value) && (value as number) >= min && (value as number) <= max;
const nativeHashes = async (files: string[]) => Object.fromEntries(await Promise.all(files.map(async file => [file, await hashFile(join(labRoot, '.native', file))] as const)));

export const drivers: Record<LaneName, LaneDriver> = {
  storage: {
    validate: config => validateStorageConfig(config as StorageConfig), blocks: config => (config as StorageConfig).blocks, seed: config => (config as StorageConfig).seed,
    combinations: config => storageCombinations(config as StorageConfig), label: cell => `${cell.engine}-${cell.format}-b${cell.batchSize}-w${cell.writers}`,
    prepare: async config => ({ sharedBuild: (config as StorageConfig).engines.some(isSharedEngine) ? await sharedBuildMetadata() : null } satisfies StorageCellShared),
    runCell: (config, cell, block, directory, context) => runStorageCell(config as StorageConfig, cell as StorageCell, block, directory, context as StorageCellShared),
    binaryChanges: async (_config, context) => { const before = (context as StorageCellShared).sharedBuild; if (!before) return 0; const after = await sharedBuildMetadata(); return Object.keys(before.hashes).filter(file => after.hashes[file] !== before.hashes[file]).length; }
  },
  workflow: {
    validate: config => validateWorkflowCampaign(config as WorkflowCampaign), blocks: config => (config as WorkflowCampaign).blocks, seed: config => (config as WorkflowCampaign).seed,
    combinations: config => workflowCombinations(config as WorkflowCampaign), label: cell => `${cell.backend}-c${cell.concurrency}-b${cell.batchJobs}-${cell.fault}${cell.poolWake ? '-' + cell.poolWake : ''}`,
    prepare: async config => (config as WorkflowCampaign).backends.includes('spider-http') ? await hashFile(join(labRoot, 'adapters/rust/target/http/release/automan-lab-spider')) : null,
    runCell: (config, cell, block, directory) => runWorkflowCell(config as WorkflowCampaign, cell as WorkflowCell, block, directory),
    binaryChanges: async (_config, spider) => spider && spider !== await hashFile(join(labRoot, 'adapters/rust/target/http/release/automan-lab-spider')) ? 1 : 0
  },
  runtime: {
    validate: config => { const c = config as RuntimeCampaign; if (!c || !/^[a-z0-9-]{1,80}$/.test(c.name) || !integer(c.blocks, 1, 20) || !integer(c.seed, 0, 2 ** 32 - 1) || !Array.isArray(c.cells) || !c.cells.length) throw new Error('invalid runtime campaign'); },
    blocks: config => (config as RuntimeCampaign).blocks, seed: config => (config as RuntimeCampaign).seed,
    combinations: config => (config as RuntimeCampaign).cells.map((cell, index) => ({ ...cell, index }) as Json), label: cell => `cell-${cell.index}`,
    prepare: () => nativeHashes(['libautoman_sqlite.dylib', 'sqlite.node', 'automan-workflow-rust', 'process-probe']),
    runCell: (config, cell, block, directory) => runRuntimeCell(config as RuntimeCampaign, Number(cell.index), block, directory),
    binaryChanges: async (_config, before) => { const after = await nativeHashes(Object.keys(before as Record<string, string>)); return Object.entries(before as Record<string, string>).filter(([file, hash]) => after[file] !== hash).length; }
  },
  actionability: {
    validate: config => { const c = config as ActionabilityConfig; if (!c || !integer(c.blocks, 1, 20) || !integer(c.seed, 0, 2 ** 32 - 1) || !Array.isArray(c.backends) || !c.backends.length || !c.backends.every(b => (backendNames as readonly string[]).includes(b)) || !Array.isArray(c.variants) || !c.variants.length || !Array.isArray(c.policies) || !c.policies.length || !c.policies.every(p => p === 'native' || p === 'guarded') || !integer(c.timeoutMs, 50, 120_000) || !integer(c.outerTimeoutMs, 50, 120_000)) throw new Error('invalid actionability campaign'); },
    blocks: config => (config as ActionabilityConfig).blocks, seed: config => (config as ActionabilityConfig).seed,
    combinations: config => actionabilityCombinations(config as ActionabilityConfig) as unknown as Json[], label: cell => `${cell.backend}/${cell.policy}/${cell.variant}`,
    prepare: async () => browserPath(),
    runCell: (config, cell, block, _directory, executablePath) => runActionabilityCell(config as ActionabilityConfig, cell as unknown as ActionabilityCell, block, executablePath as string)
  },
  'sqlite-readers': {
    validate: config => { const c = config as ReadersConfig; if (!c || !integer(c.blocks, 1, 20) || !integer(c.seed, 0, 2 ** 32 - 1) || !Array.isArray(c.engines) || !c.engines.length || !Array.isArray(c.writers) || !Array.isArray(c.readerModes) || !Array.isArray(c.batchRows) || !c.batchRows.every(b => Number.isInteger(c.totalRows / b))) throw new Error('invalid readers campaign'); },
    blocks: config => (config as ReadersConfig).blocks, seed: config => (config as ReadersConfig).seed,
    combinations: config => readersMatrix(config as ReadersConfig).map((cell, index) => ({ ...cell, index }) as Json), label: cell => `cell-${cell.index}`,
    prepare: () => sharedBuildMetadata(),
    runCell: (config, cell, block, directory) => runReadersCell(config as ReadersConfig, Number(cell.index), block, directory),
    binaryChanges: async (_config, before) => (await sharedBuildMetadata()).hashes['libautoman_sqlite.dylib'] !== (before as Awaited<ReturnType<typeof sharedBuildMetadata>>).hashes['libautoman_sqlite.dylib'] ? 1 : 0
  }
};

export function validateLaneCampaign(campaign: unknown): asserts campaign is LaneCampaign {
  const problems = validateDocument('campaign-lane.v2', campaign);
  if (problems.length) throw new Error(`invalid lane campaign: ${problems.join('; ')}`);
}

export async function runLaneCampaign(campaign: LaneCampaign, options: RunOptions & { driver?: LaneDriver } = {}) {
  validateLaneCampaign(campaign);
  const driver = options.driver ?? drivers[campaign.lane];
  driver.validate(campaign.config);
  const context = await openRun(campaign, options);
  const prepared = await driver.prepare?.(campaign.config);
  const combinations = driver.combinations(campaign.config);
  const cells: ImportedCell[] = [];
  for (let block = 0; block < driver.blocks(campaign.config); block++) {
    const order = shuffle(combinations.map((_, index) => index), driver.seed(campaign.config) + block);
    for (const index of order) {
      const cell = combinations[index]!;
      let raw: Json;
      try { raw = await driver.runCell(campaign.config, cell, block, context.directory, prepared); }
      catch (error) { raw = { ...cell, block, status: 'failed', errorCode: 'E_LANE_EXCEPTION', error: error instanceof Error ? error.message : String(error) }; }
      await appendFile(join(context.directory, 'cells.raw.jsonl'), JSON.stringify(raw) + '\n');
      const mapped = cellsCell(campaign.lane, raw, cells.length).cell;
      if (typeof raw.errorCode === 'string') mapped.errorCode = raw.errorCode;
      for (const [id, measure] of Object.entries(mapped.metrics)) if (!context.availability[id]?.available) mapped.metrics[id] = { value: null, unit: measure.unit, source: `unavailable: ${context.availability[id]?.reason ?? 'not resolved'}` };
      cells.push(mapped);
      process.stderr.write(`${campaign.id}/${driver.label(cell)}/b${block}: ${mapped.status}\n`);
    }
    await context.sampleBlock(block, order.map(index => driver.label(combinations[index]!)));
  }
  const binaryChanges = driver.binaryChanges ? await driver.binaryChanges(campaign.config, prepared) : null;
  const run = await context.close({ lane: campaign.lane, campaign, decisionIds: campaign.decisionIds, countUnit: 'cell', cells, binaryChanges });
  return { directory: context.directory, run };
}
