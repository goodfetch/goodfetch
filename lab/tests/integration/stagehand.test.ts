import { test } from 'node:test';
import assert from 'node:assert/strict';
import { startFixtures } from '../../fixtures/server.ts';
import { WorkerClient } from '../../src/worker-client.ts';
import { commandFor } from '../../src/registry.ts';
import { browserPath } from '../../src/environment.ts';

test('Stagehand accepts scheduler configuration at the lab boundary and completes trusted clicks', async () => {
  const fixtures = await startFixtures();
  const worker = new WorkerClient(commandFor('stagehand', 30000));
  try {
    await worker.call('hello', {});
    await worker.call('prepare', { executablePath: browserPath(), headless: true, concurrency: 1 });
    const fixture = await fixtures.createRun({ scenario: 'clicks', seed: 7, actions: 2, variant: 'overlay' });
    const out = await worker.call('run', { scenario: 'clicks', url: fixture.url, seed: 7, actions: 2, pages: 1, variant: 'overlay', timeoutMs: 30000 }) as {status:string;metrics:Record<string,number>};
    assert.equal(out.status, 'ok'); assert.equal((await fixtures.verify(fixture.id, out)).status, 'verified');
    assert.equal(out.metrics.viewportWidth, 1280); assert.equal(out.metrics.viewportHeight, 720);
  } finally { await worker.close(); await fixtures.close(); }
});
