import { open, readFile, rename } from 'node:fs/promises';
import { dirname } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { serve, type Adapter } from '../../src/worker-server.ts';
import type { StorageBatch, StorageFormat } from '../../src/storage.ts';

type Options = { path: string; format: StorageFormat; busyTimeoutMs: number };
let options: Options, file: Awaited<ReturnType<typeof open>> | undefined, db: DatabaseSync | undefined;
let queue = Promise.resolve(); let sequence = 0;
let records: {id:string;value:number;payload:string}[] = [];
const adapter: Adapter<StorageBatch, Options> = {
  info() {
    const probe = db ?? new DatabaseSync(':memory:');
    try { return { backend: 'storage-node', runtime: process.version, sqlite: probe.prepare('SELECT sqlite_version() AS version').get()?.version,
      execution: 'DatabaseSync blocks one event loop; one connection per worker process',
      ...(db ? Object.fromEntries(['journal_mode','synchronous','busy_timeout','wal_autocheckpoint'].map(key => [key, Object.values(db!.prepare(`PRAGMA ${key}`).get()!)[0]])) : {}),
      compileOptions: probe.prepare('PRAGMA compile_options').all().map(row => Object.values(row)[0]) }; }
    finally { if (!db) probe.close(); }
  },
  async prepare(input) {
    options = input;
    if (input.format.startsWith('sqlite')) {
      db = new DatabaseSync(input.path);
      db.exec(`PRAGMA busy_timeout=${input.busyTimeoutMs}; PRAGMA journal_mode=${input.format.includes('wal') ? 'WAL' : 'DELETE'}; PRAGMA synchronous=${input.format.endsWith('normal') ? 'NORMAL' : 'FULL'}; PRAGMA wal_autocheckpoint=1000; CREATE TABLE IF NOT EXISTS records(id TEXT PRIMARY KEY, value INTEGER NOT NULL, payload TEXT NOT NULL);`);
    } else if (input.format.startsWith('jsonl')) file = await open(input.path, 'a');
    else { records = await readFile(input.path, 'utf8').then(text => JSON.parse(text)).catch(error => { if (error.code !== 'ENOENT') throw error; return []; }); }
  },
  async run(batch) {
    const start = performance.now(); const serializationStart = performance.now();
    const payload = 'x'.repeat(batch.payloadBytes);
    const added = Array.from({ length: batch.rows }, (_, i) => ({ id: `${batch.batchId}:${i}`, value: batch.seed + i, payload }));
    // Serialization is separately instrumented even for relational inserts.
    const json = added.map(record => JSON.stringify(record)).join('\n') + '\n';
    const serializationMs = performance.now() - serializationStart;
    const work = async () => {
      if (db) {
        db.exec('BEGIN IMMEDIATE');
        try { const insert = db.prepare('INSERT INTO records(id,value,payload) VALUES(?,?,?)'); for (const record of added) insert.run(record.id, record.value, record.payload); db.exec('COMMIT'); }
        catch (error) { db.exec('ROLLBACK'); throw error; }
      } else if (file) { await file.writeFile(json); if (options.format === 'jsonl-durable') await file.sync(); }
      else {
        records.push(...added);
        const temporary = `${options.path}.${process.pid}.${sequence++}.tmp`;
        const snapshot = await open(temporary, 'wx');
        try { await snapshot.writeFile(JSON.stringify(records)); await snapshot.sync(); } finally { await snapshot.close(); }
        await rename(temporary, options.path);
        const directory = await open(dirname(options.path), 'r'); try { await directory.sync(); } finally { await directory.close(); }
      }
    };
    const pending = queue.then(work); queue = pending.catch(() => {}); await pending;
    return { status: 'ok', rows: added.length, metrics: { serializationMs, writerAckMs: performance.now() - start, encodedBytes: Buffer.byteLength(json) } };
  },
  async close() { await queue; if (db) { db.close(); db = undefined; } if (file) { await file.close(); file = undefined; } }
};
serve(adapter);
