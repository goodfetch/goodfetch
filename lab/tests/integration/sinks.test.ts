import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runStorageCell } from '../../src/storage-campaign.ts';

const config = { name: 'sinks', engines: ['node'], formats: [], blocks: 1, batches: 8, batchSizes: [64], payloadBytes: 32, writers: [1], producers: 2, timeoutMs: 15_000, maxDurationMs: 30_000, busyTimeoutMs: 5000, seed: 9, killAfterAck: true } as never;
const cell = async (format: string, writers = 1) => runStorageCell(config, { engine: 'node', format, batchSize: 64, writers } as never, 0, await mkdtemp(join(tmpdir(), 'sinks-')), { sharedBuild: null }) as Promise<{ status: string; reason?: string; verdict?: { reasons: string[] } }>;

test('DuckDB and Parquet parts keep every acknowledged batch after the writer is killed', async () => {
  for (const format of ['duckdb-table', 'parquet-batch']) { const result = await cell(format); assert.equal(result.status, 'verified', `${format}: ${result.verdict?.reasons.join('; ')}`); }
  assert.equal((await cell('duckdb-table', 2)).status, 'unsupported', 'a DuckDB file has one writing process');
});

test('PostgreSQL COPY keeps every acknowledged batch after the writer is killed', { skip: process.env.AUTOMAN_PG_URL ? false : 'AUTOMAN_PG_URL is not set' }, async () => {
  const result = await cell('postgres-copy', 2); assert.equal(result.status, 'verified', result.verdict?.reasons.join('; ') ?? '');
});
