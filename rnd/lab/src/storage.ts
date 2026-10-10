import { readFile, stat } from 'node:fs/promises';
import { DatabaseSync } from 'node:sqlite';
import { pg, pgUrl, pgTable, duckdb } from './sinks.ts';

export const storageFormats = ['jsonl-buffered', 'jsonl-durable', 'json-snapshot-durable', 'sqlite-wal-full', 'sqlite-wal-normal', 'sqlite-delete-full', 'postgres-copy', 'duckdb-table', 'parquet-batch'] as const;
export type StorageFormat = typeof storageFormats[number];
export type StorageBatch = { batchId: string; seed: number; rows: number; payloadBytes: number };
type Record = { id: string; value: number; payload: string };

// This reads committed files independently of worker acknowledgements and metrics.
export async function verifyStorage(path: string, format: StorageFormat, batches: StorageBatch[]) {
  const reasons: string[] = []; let records: Record[] = [];
  try {
    if (format === 'postgres-copy') {
      const client = new (pg().Client)({ connectionString: pgUrl() }); await client.connect();
      try { records = (await client.query(`SELECT id, value::int AS value, payload FROM ${pgTable(path)}`)).rows as unknown as Record[]; } finally { await client.end(); }
    } else if (format === 'duckdb-table' || format === 'parquet-batch') {
      // DuckDB reopens its own file read-only; Parquet parts are read by an in-memory instance.
      const instance = await duckdb().DuckDBInstance.create(format === 'duckdb-table' ? path : ':memory:', format === 'duckdb-table' ? { access_mode: 'READ_ONLY' } : {}); const connection = await instance.connect();
      try { const source = format === 'duckdb-table' ? 'records' : `read_parquet('${path.replaceAll("'", "''")}/*.parquet')`; records = (await (await connection.runAndReadAll(`SELECT id, CAST(value AS INTEGER) AS value, payload FROM ${source}`)).getRowObjectsJS()) as unknown as Record[]; }
      catch (error) { if (!(format === 'parquet-batch' && /No files found/.test(String(error)))) throw error; }
      finally { connection.closeSync(); instance.closeSync(); }
    } else if (format.startsWith('sqlite')) {
      const db = new DatabaseSync(path, { readOnly: true });
      try {
        const integrity = db.prepare('PRAGMA integrity_check').all();
        if (integrity.length !== 1 || Object.values(integrity[0]!)[0] !== 'ok') reasons.push('SQLite integrity check failed');
        records = db.prepare('SELECT id, value, payload FROM records').all() as unknown as Record[];
      } finally { db.close(); }
    } else {
      const text = await readFile(path, 'utf8');
      records = format.startsWith('jsonl') ? text.trim().split('\n').filter(Boolean).map(line => JSON.parse(line)) : JSON.parse(text);
    }
    if (!Array.isArray(records)) throw new Error('output is not a record collection');
    const byId = new Map(records.map(record => [record.id, record]));
    const expectedCount = batches.reduce((count, batch) => count + batch.rows, 0);
    if (records.length !== expectedCount) reasons.push(`row count ${records.length}, expected ${expectedCount}`);
    if (byId.size !== records.length) reasons.push('duplicate persisted IDs');
    for (const batch of batches) for (let i = 0; i < batch.rows; i++) {
      const record = byId.get(`${batch.batchId}:${i}`);
      if (!record || record.value !== batch.seed + i || record.payload !== 'x'.repeat(batch.payloadBytes)) reasons.push(`wrong persisted record ${batch.batchId}:${i}`);
    }
  } catch (error) { reasons.push(error instanceof Error ? error.message : String(error)); }
  const fileBytes = (await Promise.all([path, `${path}-wal`, `${path}-shm`].map(file => stat(file).then(s => s.size).catch(() => 0)))).reduce((sum, bytes) => sum + bytes, 0);
  return { status: reasons.length ? 'failed' as const : 'verified' as const, reasons: reasons.slice(0, 20), persistedRows: records.length, fileBytes };
}
