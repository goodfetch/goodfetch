// Fixture server and private oracle in their own process: ground truth never lives in the measured coordinator.
import { createInterface } from 'node:readline';
import { startFixtures } from './server.ts';

const fixtures = await startFixtures();
const lines = createInterface({ input: process.stdin });
lines.on('line', async line => {
  let id: number | null = null;
  try {
    const message = JSON.parse(line) as { version: number; id: number; method: string; params: Record<string, unknown> };
    id = message.id;
    if (message.version !== 1 || !Number.isInteger(id)) throw new Error('invalid request');
    let result: unknown;
    if (message.method === 'hello') result = { origin: fixtures.origin };
    else if (message.method === 'createRun') result = await fixtures.createRun(message.params as never);
    else if (message.method === 'verify') result = await fixtures.verify(message.params.id as string, message.params.output as never);
    else if (message.method === 'snapshot') result = fixtures.snapshot(message.params.id as string);
    else if (message.method === 'close') { await fixtures.close(); process.stdout.write(JSON.stringify({ version: 1, id, result: { closed: true } }) + '\n'); process.exit(0); }
    else throw new Error('unknown method');
    process.stdout.write(JSON.stringify({ version: 1, id, result }) + '\n');
  } catch (error) { process.stdout.write(JSON.stringify({ version: 1, id, error: error instanceof Error ? error.message : String(error) }) + '\n'); }
});
lines.on('close', () => { void fixtures.close().finally(() => process.exit(0)); });
