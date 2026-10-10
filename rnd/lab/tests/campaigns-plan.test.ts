import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { labRoot } from '../src/registry.ts';
import { drivers, type LaneCampaign } from '../src/core/lanes.ts';
import type { BenchCampaign } from '../src/core/campaign.ts';

const load = (path: string) => JSON.parse(readFileSync(join(labRoot, path), 'utf8')) as BenchCampaign | LaneCampaign;
// Planned units of a campaign: cells × blocks for a lane, attempts for the bench engine.
function planned(campaign: BenchCampaign | LaneCampaign) {
  if (campaign.lane === 'bench') return campaign.blocks * campaign.backends.length * campaign.scenarios.length * ((campaign.fixtures.variants?.length ?? 1) + campaign.warmup + campaign.iterations);
  const driver = drivers[campaign.lane];
  return driver.blocks(campaign.config) * driver.combinations(campaign.config).length;
}

test('the data-driven campaigns plan exactly the units published in the 8 October qualification', () => {
  const suite = JSON.parse(readFileSync(join(labRoot, 'campaigns/v2/suites/qualification-current.json'), 'utf8')) as { steps: { campaign: string; expected: { verified: number } }[] };
  // Published counts: lab/reports/integrated-2026-10-08.md.
  assert.deepEqual(suite.steps.map(step => step.expected.verified), [30, 42, 162, 108, 60, 24]);
  for (const step of suite.steps) assert.equal(planned(load(step.campaign)), step.expected.verified, step.campaign);
});
