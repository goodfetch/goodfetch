import { validateCampaign, type Campaign } from '../campaign.ts';
import { validateDocument } from './schema.ts';
import type { HostClass } from './host.ts';

export type BenchCampaign = {
  schemaVersion: 2; id: string; lane: 'bench'; decisionIds: string[]; hostClass: { requested: HostClass; allowDowngrade: boolean };
  seed: number; blocks: number; warmup: number; iterations: number; concurrency: number; mode: 'closed' | 'open'; ratePerSecond?: number;
  timeoutMs: number; maxDurationMs: number; maxRestarts: number; backends: string[]; scenarios: string[];
  fixtures: { pages: number; actions: number; delayMs?: number; variants?: string[] }; poolWake?: 'poll' | 'notify';
  network?: { link: 'browser-origin' | 'driver-browser'; rttMs: number };
  // 'harness' launches one Chromium per backend and block and hands drivers its DevTools endpoint.
  browserHost?: 'adapter' | 'harness' | 'lightpanda'; browserHosts?: ('harness' | 'lightpanda')[]; actionPolicies?: Record<string, 'native' | 'guarded'>;
};

export function toLegacyBench(campaign: BenchCampaign): Campaign {
  return { name: campaign.id, backends: campaign.backends as Campaign['backends'], scenarios: campaign.scenarios as Campaign['scenarios'], seed: campaign.seed, blocks: campaign.blocks,
    iterations: campaign.iterations, warmup: campaign.warmup, concurrency: campaign.concurrency, mode: campaign.mode, ...(campaign.ratePerSecond !== undefined ? { ratePerSecond: campaign.ratePerSecond } : {}),
    timeoutMs: campaign.timeoutMs, maxDurationMs: campaign.maxDurationMs, pages: campaign.fixtures.pages, actions: campaign.fixtures.actions,
    ...(campaign.fixtures.delayMs !== undefined ? { delayMs: campaign.fixtures.delayMs } : {}), ...(campaign.fixtures.variants ? { variants: campaign.fixtures.variants as Campaign['variants'] } : {}),
    ...(campaign.poolWake ? { poolWake: campaign.poolWake } : {}) };
}

export function fromLegacyBench(legacy: Campaign, overrides: Partial<Pick<BenchCampaign, 'decisionIds' | 'hostClass' | 'maxRestarts'>> = {}): BenchCampaign {
  return { schemaVersion: 2, id: legacy.name, lane: 'bench', decisionIds: overrides.decisionIds ?? [], hostClass: overrides.hostClass ?? { requested: 'local-dev', allowDowngrade: true },
    seed: legacy.seed, blocks: legacy.blocks, warmup: legacy.warmup, iterations: legacy.iterations, concurrency: legacy.concurrency, mode: legacy.mode,
    ...(legacy.ratePerSecond !== undefined ? { ratePerSecond: legacy.ratePerSecond } : {}), timeoutMs: legacy.timeoutMs, maxDurationMs: legacy.maxDurationMs,
    maxRestarts: overrides.maxRestarts ?? 2, backends: [...legacy.backends], scenarios: [...legacy.scenarios],
    fixtures: { pages: legacy.pages, actions: legacy.actions, ...(legacy.delayMs !== undefined ? { delayMs: legacy.delayMs } : {}), ...(legacy.variants ? { variants: [...legacy.variants] } : {}) },
    ...(legacy.poolWake ? { poolWake: legacy.poolWake } : {}) };
}

// Schema first, then the legacy semantic checks (known backends and scenarios, retained-corpus budget).
export function validateBenchCampaign(campaign: unknown): asserts campaign is BenchCampaign {
  const problems = validateDocument('campaign-bench.v2', campaign);
  if (problems.length) throw new Error(`invalid bench campaign: ${problems.join('; ')}`);
  validateCampaign(toLegacyBench(campaign as BenchCampaign));
}
