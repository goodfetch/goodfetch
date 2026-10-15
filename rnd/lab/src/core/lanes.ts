import { sqliteLibrary } from '../registry.ts';
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
import { aiCombinations, runAiCell, priceSnapshot, type AiConfig, type AiCell } from '../ai-campaign.ts';
import { identityCombinations, runIdentityCell, httpTiers, identityVariants, type IdentityConfig, type IdentityCell } from '../identity-campaign.ts';
import { admissionCombinations, runAdmissionCell, admissionBinary, type AdmissionConfig, type AdmissionCell } from '../admission-campaign.ts';
import { topologyCombinations, runTopologyCell, topologyPolicies, type TopologyConfig, type TopologyCell } from '../topology-campaign.ts';
import { portalCombinations, runPortalCell, portalPolicies, portalVariants, type PortalConfig, type PortalCell } from '../portal-campaign.ts';
import { extractionCombinations, runExtractionCell, type ExtractionConfig, type ExtractionCell } from '../extraction-campaign.ts';
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
    prepare: async config => (config as WorkflowCampaign).backends.includes('spider-http') ? await hashFile(join(labRoot, 'adapters/rust/target/http/release/goodfetch-lab-spider')) : null,
    runCell: (config, cell, block, directory) => runWorkflowCell(config as WorkflowCampaign, cell as WorkflowCell, block, directory),
    binaryChanges: async (_config, spider) => spider && spider !== await hashFile(join(labRoot, 'adapters/rust/target/http/release/goodfetch-lab-spider')) ? 1 : 0
  },
  runtime: {
    validate: config => { const c = config as RuntimeCampaign; if (!c || !/^[a-z0-9-]{1,80}$/.test(c.name) || !integer(c.blocks, 1, 20) || !integer(c.seed, 0, 2 ** 32 - 1) || !Array.isArray(c.cells) || !c.cells.length) throw new Error('invalid runtime campaign'); },
    blocks: config => (config as RuntimeCampaign).blocks, seed: config => (config as RuntimeCampaign).seed,
    combinations: config => (config as RuntimeCampaign).cells.map((cell, index) => ({ ...cell, index }) as Json), label: cell => `cell-${cell.index}`,
    prepare: () => nativeHashes([sqliteLibrary, 'sqlite.node', 'goodfetch-workflow-rust', ...(process.platform === 'darwin' ? ['process-probe'] : [])]),
    runCell: (config, cell, block, directory) => runRuntimeCell(config as RuntimeCampaign, Number(cell.index), block, directory),
    binaryChanges: async (_config, before) => { const after = await nativeHashes(Object.keys(before as Record<string, string>)); return Object.entries(before as Record<string, string>).filter(([file, hash]) => after[file] !== hash).length; }
  },
  actionability: {
    validate: config => { const c = config as ActionabilityConfig; if (!c || !integer(c.blocks, 1, 20) || !integer(c.seed, 0, 2 ** 32 - 1) || !Array.isArray(c.backends) || !c.backends.length || !c.backends.every(b => (backendNames as readonly string[]).includes(b)) || !Array.isArray(c.variants) || !c.variants.length || !Array.isArray(c.policies) || !c.policies.length || !c.policies.every(p => p === 'native' || p === 'guarded') || !integer(c.timeoutMs, 50, 120_000) || !integer(c.outerTimeoutMs, 50, 120_000) || (c.scenario !== undefined && !['actionability', 'primitives', 'navigation', 'http-catalog'].includes(c.scenario))) throw new Error('invalid actionability campaign'); },
    blocks: config => (config as ActionabilityConfig).blocks, seed: config => (config as ActionabilityConfig).seed,
    combinations: config => actionabilityCombinations(config as ActionabilityConfig) as unknown as Json[], label: cell => `${cell.scenario}/${cell.backend}/${cell.policy}/${cell.variant}`,
    prepare: async () => browserPath(),
    runCell: (config, cell, block, _directory, executablePath) => runActionabilityCell(config as ActionabilityConfig, cell as unknown as ActionabilityCell, block, executablePath as string)
  },
  ai: {
    validate: config => { const c = config as AiConfig; if (!c || !Array.isArray(c.modes) || !c.modes.length || !c.modes.every(m => m === 'agent-each-run' || m === 'compiled-replay') || !Array.isArray(c.variants) || !c.variants.length || !integer(c.warmRuns, 1, 50) || !integer(c.blocks, 1, 10) || !integer(c.timeoutMs, 100, 60_000) || (c.models !== undefined && (!Array.isArray(c.models) || !c.models.length || !c.models.every(m => /^[a-z0-9-]+\/[a-z0-9.-]+$/.test(m)) || typeof c.gateway !== 'string' || !/^https:\/\//.test(c.gateway) || !(typeof c.maxUsd === 'number' && c.maxUsd > 0 && c.maxUsd <= 20))) || (c.interfaces !== undefined && (!c.models || !Array.isArray(c.interfaces) || !c.interfaces.every(i => ['sdk', 'mcp', 'cli', 'code-mode', 'code-mode-journal'].includes(i))))) throw new Error('invalid ai campaign'); },
    blocks: config => (config as AiConfig).blocks, seed: config => (config as AiConfig).seed,
    combinations: config => aiCombinations(config as AiConfig) as unknown as Json[], label: cell => `${cell.model}/${cell.interface ?? cell.mode}/${cell.variant}`,
    prepare: config => priceSnapshot(config as AiConfig),
    runCell: (config, cell, block, directory, prices) => runAiCell(config as AiConfig, cell as unknown as AiCell, block, directory, prices as Awaited<ReturnType<typeof priceSnapshot>>) as Promise<Json>
  },
  identity: {
    validate: config => { const c = config as IdentityConfig; if (!c || !Array.isArray(c.httpTiers) || !c.httpTiers.length || !c.httpTiers.every(t => httpTiers.includes(t)) || !Array.isArray(c.profiles) || !c.profiles.length || !c.profiles.every(p => p === 'identified' || p === 'standard') || !Array.isArray(c.purposes) || !c.purposes.length || !c.purposes.every(p => ['search', 'ai-input', 'ai-train', 'agent', 'data-collection', 'transact'].includes(p)) || !Array.isArray(c.variants) || !c.variants.length || !c.variants.every(v => (identityVariants as readonly string[]).includes(v)) || !integer(c.repeats, 1, 50) || !integer(c.blocks, 1, 20) || !integer(c.seed, 0, 2 ** 32 - 1) || !integer(c.timeoutMs, 1000, 120_000)) throw new Error('invalid identity campaign'); },
    blocks: config => (config as IdentityConfig).blocks, seed: config => (config as IdentityConfig).seed,
    combinations: config => identityCombinations(config as IdentityConfig) as unknown as Json[], label: cell => `${cell.httpTier}/${cell.profile}/${cell.purpose}/${cell.variant}`,
    runCell: (config, cell, block) => runIdentityCell(config as IdentityConfig, cell as unknown as IdentityCell, block) as Promise<Json>
  },
  admission: {
    validate: config => { const c = config as AdmissionConfig; if (!c || !Array.isArray(c.policies) || !c.policies.length || !c.policies.every(p => p === 'fifo-global' || p === 'fair-domain') || typeof c.scenario !== 'string' || !Array.isArray(c.domains) || !c.domains.length || !c.domains.every(d => /^[a-z0-9-]{1,40}$/.test(d.name) && d.ratePerSecond > 0 && d.ratePerSecond <= 2000) || !integer(c.durationMs, 100, 120_000) || !integer(c.serviceMs, 0, 10_000) || !integer(c.workers, 1, 512) || !integer(c.domainLimit, 1, 512) || !integer(c.capacity, 1, 100_000) || !integer(c.domainCapacity, 1, 100_000) || !integer(c.batchMax, 1, 4096) || !integer(c.blocks, 1, 20) || !integer(c.seed, 0, 2 ** 32 - 1) || !integer(c.timeoutMs, 100, 120_000) || (c.crashAtMs !== undefined && !integer(c.crashAtMs, 1, c.durationMs))) throw new Error('invalid admission campaign'); },
    blocks: config => (config as AdmissionConfig).blocks, seed: config => (config as AdmissionConfig).seed,
    combinations: config => admissionCombinations(config as AdmissionConfig) as unknown as Json[], label: cell => `${cell.policy}/${cell.scenario}/${cell.fault}`,
    prepare: () => hashFile(admissionBinary()),
    runCell: (config, cell, block, directory) => runAdmissionCell(config as AdmissionConfig, cell as unknown as AdmissionCell, block, directory) as Promise<Json>,
    binaryChanges: async (_config, before) => before !== await hashFile(admissionBinary()) ? 1 : 0
  },
  extraction: {
    validate: config => { const c = config as ExtractionConfig; if (!c || !['dev', 'test'].includes(c.split) || !Array.isArray(c.extractors) || !c.extractors.length || !c.extractors.every(e => (backendNames as readonly string[]).includes(e)) || !Array.isArray(c.pageTypes) || !c.pageTypes.length || !integer(c.blocks, 1, 5) || !integer(c.seed, 0, 2 ** 32 - 1) || (c.renderers !== undefined && (!Array.isArray(c.renderers) || !c.renderers.every(r => ['none', 'chrome', 'lightpanda'].includes(r)))) || (c.shards !== undefined && !integer(c.shards, 1, 200)) || (c.markdown !== undefined && typeof c.markdown !== 'boolean') || (c.task !== undefined && !['main-content', 'fields'].includes(c.task)) || (c.corpus !== undefined && !/^(corpora|fixtures)\/[a-z0-9-]+$/.test(c.corpus))) throw new Error('invalid extraction campaign'); },
    blocks: config => (config as ExtractionConfig).blocks, seed: config => (config as ExtractionConfig).seed,
    combinations: config => extractionCombinations(config as ExtractionConfig) as unknown as Json[], label: cell => `${cell.corpus ?? 'wcxb'}/${cell.renderer ?? 'none'}/${cell.extractor}/${cell.pageType}${cell.shard === undefined ? '' : `/s${cell.shard}`}`,
    runCell: (config, cell, block, directory) => runExtractionCell(config as ExtractionConfig, cell as unknown as ExtractionCell, block, directory) as Promise<Json>
  },
  portal: {
    validate: config => { const c = config as PortalConfig; const group = c?.group; if (!c || !(group in portalPolicies) || !Array.isArray(c.variants) || !c.variants.length || !c.variants.every(v => (portalVariants[group] as readonly string[]).includes(v)) || !Array.isArray(c.policies) || !c.policies.length || !c.policies.every(p => (portalPolicies[group] as readonly string[]).includes(p)) || (c.tiers !== undefined && (!Array.isArray(c.tiers) || !c.tiers.length || !c.tiers.every(t => t === 'http' || t === 'browser'))) || !integer(c.repeats, 1, 50) || !integer(c.blocks, 1, 20) || !integer(c.seed, 0, 2 ** 32 - 1) || !integer(c.timeoutMs, 1000, 120_000) || (c.maxRequests !== undefined && !integer(c.maxRequests, 10, 100_000))) throw new Error('invalid portal campaign'); },
    blocks: config => (config as PortalConfig).blocks, seed: config => (config as PortalConfig).seed,
    combinations: config => portalCombinations(config as PortalConfig) as unknown as Json[], label: cell => `${cell.scenario}/${cell.variant}/${cell.policy}${cell.tier ? '/' + cell.tier : ''}`,
    runCell: (config, cell, block, directory) => runPortalCell(config as PortalConfig, cell as unknown as PortalCell, block, directory) as Promise<Json>
  },
  topology: {
    validate: config => { const c = config as TopologyConfig; if (!c || !Array.isArray(c.policies) || !c.policies.length || !c.policies.every(p => (topologyPolicies as readonly string[]).includes(p)) || typeof c.image !== 'string' || !/^[a-z0-9./-]+:[0-9A-Za-z.-]+$/.test(c.image) || !integer(c.sessions, 1, 100) || !integer(c.poolSize, 1, 16) || !integer(c.blocks, 1, 20) || !integer(c.seed, 0, 2 ** 32 - 1) || !integer(c.timeoutMs, 1000, 120_000)) throw new Error('invalid topology campaign'); },
    blocks: config => (config as TopologyConfig).blocks, seed: config => (config as TopologyConfig).seed,
    combinations: config => topologyCombinations(config as TopologyConfig) as unknown as Json[], label: cell => `${cell.policy}`,
    runCell: (config, cell, block) => runTopologyCell(config as TopologyConfig, cell as unknown as TopologyCell, block) as Promise<Json>
  },
  'sqlite-readers': {
    validate: config => { const c = config as ReadersConfig; if (!c || !integer(c.blocks, 1, 20) || !integer(c.seed, 0, 2 ** 32 - 1) || !Array.isArray(c.engines) || !c.engines.length || !Array.isArray(c.writers) || !Array.isArray(c.readerModes) || !Array.isArray(c.batchRows) || !c.batchRows.every(b => Number.isInteger(c.totalRows / b))) throw new Error('invalid readers campaign'); },
    blocks: config => (config as ReadersConfig).blocks, seed: config => (config as ReadersConfig).seed,
    combinations: config => readersMatrix(config as ReadersConfig).map((cell, index) => ({ ...cell, index }) as Json), label: cell => `cell-${cell.index}`,
    prepare: () => sharedBuildMetadata(),
    runCell: (config, cell, block, directory) => runReadersCell(config as ReadersConfig, Number(cell.index), block, directory),
    binaryChanges: async (_config, before) => (await sharedBuildMetadata()).hashes[sqliteLibrary] !== (before as Awaited<ReturnType<typeof sharedBuildMetadata>>).hashes[sqliteLibrary] ? 1 : 0
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
      // A cell has a budget (config.cellTimeoutMs, 15 min by default); past it, the cell is a timeout and the campaign goes on.
      const budgetMs = typeof campaign.config.cellTimeoutMs === 'number' ? campaign.config.cellTimeoutMs : 15 * 60_000; let watchdog: NodeJS.Timeout | undefined;
      try { raw = await Promise.race([driver.runCell(campaign.config, cell, block, context.directory, prepared), new Promise<Json>(resolve => { watchdog = setTimeout(() => resolve({ ...cell, block, status: 'timeout', errorCode: 'E_CELL_TIMEOUT', error: `cell exceeded its ${budgetMs} ms budget` }), budgetMs); })]); }
      catch (error) { raw = { ...cell, block, status: 'failed', errorCode: 'E_LANE_EXCEPTION', error: error instanceof Error ? error.message : String(error) }; }
      finally { clearTimeout(watchdog); }
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
