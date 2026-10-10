import { open, readFile, rename, mkdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { pg, copyFrom, pgUrl, pgTable, duckdb, type PgClient, type DuckConnection } from '../../src/sinks.ts';
import { DatabaseSync } from 'node:sqlite';
import { serve, type Adapter } from '../../src/worker-server.ts';
import type { StorageBatch, StorageFormat } from '../../src/storage.ts';

type Options = { path: string; format: StorageFormat; busyTimeoutMs: number };
let options: Options, file: Awaited<ReturnType<typeof open>> | undefined, db: DatabaseSync | undefined;
let queue = Promise.resolve(); let sequence = 0;
let records: {id:string;value:number;payload:string}[] = [];
// The additional sinks: a PostgreSQL client, a DuckDB database (also used, in memory, to encode Parquet parts).
let postgres: PgClient | undefined, duck: { instance: { closeSync(): void }; connection: DuckConnection } | undefined;
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
    if (input.format === 'postgres-copy') {
      postgres = new (pg().Client)({ connectionString: pgUrl() }); await postgres.connect();
      await postgres.query(`CREATE TABLE IF NOT EXISTS ${pgTable(input.path)} (id TEXT PRIMARY KEY, value BIGINT NOT NULL, payload TEXT NOT NULL)`);
    } else if (input.format === 'duckdb-table' || input.format === 'parquet-batch') {
      if (input.format === 'parquet-batch') await mkdir(input.path, { recursive: true });
      const instance = await duckdb().DuckDBInstance.create(input.format === 'duckdb-table' ? input.path : ':memory:'); const connection = await instance.connect(); duck = { instance, connection };
      await connection.run(`CREATE ${input.format === 'parquet-batch' ? 'TEMP ' : ''}TABLE IF NOT EXISTS records (id VARCHAR PRIMARY KEY, value BIGINT NOT NULL, payload VARCHAR NOT NULL)`);
    } else if (input.format.startsWith('sqlite')) {
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
      if (postgres) {
        // One transaction per batch, rows streamed through COPY; the acknowledgement follows the commit (synchronous_commit on).
        await postgres.query('BEGIN');
        try { await pipeline(Readable.from(added.map(r => `${r.id},${r.value},${r.payload}\n`)), (postgres as unknown as { query(stream: unknown): NodeJS.WritableStream }).query(copyFrom()(`COPY ${pgTable(options.path)} (id, value, payload) FROM STDIN WITH (FORMAT csv)`))); await postgres.query('COMMIT'); }
        catch (error) { await postgres.query('ROLLBACK').catch(() => {}); throw error; }
      } else if (duck) {
        const append = async () => { const appender = await duck!.connection.createAppender('records'); for (const r of added) { appender.appendVarchar(r.id); appender.appendBigInt(BigInt(r.value)); appender.appendVarchar(r.payload); appender.endRow(); } appender.flushSync(); appender.closeSync(); };
        if (options.format === 'duckdb-table') { await duck.connection.run('BEGIN TRANSACTION'); try { await append(); await duck.connection.run('COMMIT'); } catch (error) { await duck.connection.run('ROLLBACK').catch(() => {}); throw error; } }
        else {
          // A Parquet part per batch: encoded to a temporary file, synced, renamed into place, then the directory synced.
          await duck.connection.run('DELETE FROM records'); await append();
          const final = join(options.path, `part-${batch.batchId}.parquet`), temporary = `${final}.${process.pid}.tmp`;
          await duck.connection.run(`COPY records TO '${temporary.replaceAll("'", "''")}' (FORMAT parquet)`);
          const part = await open(temporary, 'r+'); try { await part.sync(); } finally { await part.close(); }
          await rename(temporary, final);
          const directory = await open(options.path, 'r'); try { await directory.sync(); } finally { await directory.close(); }
        }
      } else if (db) {
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
  async close() { await queue; if (postgres) { await postgres.end(); postgres = undefined; } if (duck) { duck.connection.closeSync(); duck.instance.closeSync(); duck = undefined; } if (db) { db.close(); db = undefined; } if (file) { await file.close(); file = undefined; } }
};
serve(adapter);
