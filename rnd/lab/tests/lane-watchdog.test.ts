import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runLaneCampaign, type LaneDriver, type LaneCampaign } from '../src/core/lanes.ts';

test('a cell that never settles is recorded as a timeout and the campaign goes on', async () => {
  const seen: string[] = [];
  const driver: LaneDriver = {
    validate() {}, blocks: () => 1, seed: () => 1, combinations: () => [{ variant: 'hangs' }, { variant: 'works' }], label: cell => String(cell.variant),
    runCell: async (_config, cell, block) => { seen.push(String(cell.variant)); return cell.variant === 'hangs' ? new Promise(() => {}) : { ...cell, block, status: 'verified' }; }
  };
  const campaign = { schemaVersion: 2, id: 'watchdog-test', lane: 'ai', decisionIds: [], hostClass: { requested: 'local-dev', allowDowngrade: true }, config: { cellTimeoutMs: 200 } } as unknown as LaneCampaign;
  const { run } = await runLaneCampaign(campaign, { driver, outputRoot: await mkdtemp(join(tmpdir(), 'goodfetch-watchdog-')), skipProvenance: true });
  assert.deepEqual(seen.sort(), ['hangs', 'works']);
  assert.deepEqual(run.cells.map(cell => [cell.factors.variant, cell.status]).sort(), [['hangs', 'timeout'], ['works', 'verified']]);
});
