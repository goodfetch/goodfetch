import { readFile, stat } from 'node:fs/promises';
import { DatabaseSync } from 'node:sqlite';

export const storageFormats = ['jsonl-buffered', 'jsonl-durable', 'json-snapshot-durable', 'sqlite-wal-full', 'sqlite-wal-normal', 'sqlite-delete-full'] as const;
export type StorageFormat = typeof storageFormats[number];
export type StorageBatch = { batchId: string; seed: number; rows: number; payloadBytes: number };
type Record = { id: string; value: number; payload: string };

// This reads committed files independently of worker acknowledgements and metrics.
export async function verifyStorage(path: string, format: StorageFormat, batches: StorageBatch[]) {
  const reasons: string[] = []; let records: Record[] = [];
  try {
    if (format.startsWith('sqlite')) {
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
