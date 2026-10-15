import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { startFixtures } from '../../fixtures/server.ts';
import { WorkerClient } from '../../src/worker-client.ts';
import { commandFor, labRoot } from '../../src/registry.ts';
import { browserPath } from '../../src/environment.ts';

const variants = ['hover', 'dblclick', 'contextmenu', 'drag', 'wheel', 'keyboard', 'contenteditable', 'unicode', 'moved', 'retarget'];
const rustBuilt = existsSync(join(labRoot, 'adapters/cdp-rust/target/release/goodfetch-lab-cdp'));

async function primitives(backend: 'node-cdp' | 'rust-cdp' | 'playwright', actionPolicy: 'native' | 'guarded') {
  const fixtures = await startFixtures(); const worker = new WorkerClient(commandFor(backend, 20_000, browserPath()));
  const rows: { variant: string; status: string; reasons: string[]; cdpCommands?: number }[] = [];
  try {
    await worker.call('hello', {}); await worker.call('prepare', { executablePath: browserPath(), headless: true }, 45_000);
    for (const variant of variants) {
      const fixture = await fixtures.createRun({ scenario: 'primitives', seed: 3, variant: variant as never });
      try {
        const output = await worker.call('run', { scenario: 'primitives', seed: 3, pages: 1, actions: 1, timeoutMs: 1500, variant, actionPolicy, url: fixture.url }) as { metrics?: Record<string, number> };
        rows.push({ variant, ...(await fixtures.verify(fixture.id, output as never)), cdpCommands: output.metrics?.cdpCommands });
      } catch (error) { rows.push({ variant, status: 'failed', reasons: [String(error)] }); }
    }
  } finally { await worker.close(); await fixtures.close(); }
  return rows;
}

test('guarded CDP drivers perform every primitive and refuse a target swapped as the pointer arrives, with equal command counts', { skip: !rustBuilt && 'run pnpm bootstrap to build adapters/cdp-rust' }, async () => {
  const [node, rust] = [await primitives('node-cdp', 'guarded'), await primitives('rust-cdp', 'guarded')];
  for (const rows of [node, rust]) assert.deepEqual(rows.filter(row => row.status !== 'verified').map(row => [row.variant, row.reasons]), []);
  assert.deepEqual(rust.map(row => [row.variant, row.cdpCommands]), node.map(row => [row.variant, row.cdpCommands]));
});

test('Playwright performs every primitive with its native guarantees', async () => {
  const rows = await primitives('playwright', 'native');
  assert.deepEqual(rows.filter(row => row.status !== 'verified').map(row => [row.variant, row.reasons]), []);
});

const navigationVariants = ['spa', 'history', 'popup', 'frame', 'cross-frame', 'upload', 'download'];
async function navigation(backend: 'node-cdp' | 'rust-cdp') {
  const fixtures = await startFixtures(); const worker = new WorkerClient(commandFor(backend, 20_000, browserPath()));
  const rows: { variant: string; status: string; reasons: string[]; cdpCommands?: number }[] = [];
  try {
    await worker.call('hello', {}); await worker.call('prepare', { executablePath: browserPath(), headless: true }, 45_000);
    for (const variant of navigationVariants) {
      const fixture = await fixtures.createRun({ scenario: 'navigation', seed: 3, variant: variant as never });
      try {
        const output = await worker.call('run', { scenario: 'navigation', seed: 3, pages: 1, actions: 1, timeoutMs: 3000, variant, actionPolicy: 'guarded', url: fixture.url }) as { metrics?: Record<string, number> };
        rows.push({ variant, ...(await fixtures.verify(fixture.id, output as never)), cdpCommands: output.metrics?.cdpCommands });
      } catch (error) { rows.push({ variant, status: 'failed', reasons: [String(error)] }); }
    }
  } finally { await worker.close(); await fixtures.close(); }
  return rows;
}

test('guarded CDP drivers handle SPA routes, history, popups, same- and cross-origin frames, uploads and downloads with equal command counts', { skip: !rustBuilt && 'run pnpm bootstrap to build adapters/cdp-rust' }, async () => {
  const [node, rust] = [await navigation('node-cdp'), await navigation('rust-cdp')];
  for (const rows of [node, rust]) assert.deepEqual(rows.filter(row => row.status !== 'verified').map(row => [row.variant, row.reasons]), []);
  assert.deepEqual(rust.map(row => [row.variant, row.cdpCommands]), node.map(row => [row.variant, row.cdpCommands]));
});
