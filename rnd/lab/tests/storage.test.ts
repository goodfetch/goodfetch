import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { verifyStorage } from '../src/storage.ts';
import { DatabaseSync } from 'node:sqlite';

test('storage oracle accepts exact bytes and rejects duplicate or stale persisted records', async () => {
  const path = join(await mkdtemp(join(tmpdir(), 'automan-lab-sink-')), 'records.jsonl');
  const batch = { batchId: 'b0', seed: 7, rows: 2, payloadBytes: 3 };
  const records = [{ id: 'b0:0', value: 7, payload: 'xxx' }, { id: 'b0:1', value: 8, payload: 'xxx' }];
  await writeFile(path, records.map(record => JSON.stringify(record)).join('\n') + '\n');
  assert.equal((await verifyStorage(path, 'jsonl-buffered', [batch])).status, 'verified');
  await writeFile(path, [...records, records[0]].map(record => JSON.stringify(record)).join('\n') + '\n');
  assert.equal((await verifyStorage(path, 'jsonl-buffered', [batch])).status, 'failed');
  await writeFile(path, JSON.stringify({ id: 'b0:0', value: 6, payload: 'xxx' }) + '\n');
  assert.equal((await verifyStorage(path, 'jsonl-buffered', [batch])).status, 'failed');
});

test('SQLite oracle checks persisted values after reopen and detects a changed row', async () => {
  const path = join(await mkdtemp(join(tmpdir(), 'automan-lab-sqlite-')), 'records.sqlite');
  const batch = { batchId: 'b0', seed: 7, rows: 1, payloadBytes: 3 };
  const db = new DatabaseSync(path);
  db.exec("CREATE TABLE records(id TEXT PRIMARY KEY,value INTEGER,payload TEXT); INSERT INTO records VALUES('b0:0',7,'xxx');"); db.close();
  assert.equal((await verifyStorage(path, 'sqlite-wal-full', [batch])).status, 'verified');
  const changed = new DatabaseSync(path); changed.exec("UPDATE records SET value=99 WHERE id='b0:0'"); changed.close();
  assert.equal((await verifyStorage(path, 'sqlite-wal-full', [batch])).status, 'failed');
});
