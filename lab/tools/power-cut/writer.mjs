// Runs inside the Linux VM of Docker: writes batches of 50 rows to one sink until it is killed, and prints "ack <batch>" on stdout
// only once the sink reports the batch committed with the durability its format promises.
import { DatabaseSync } from 'node:sqlite';
import { open, rename, mkdir } from 'node:fs/promises';
const format = process.argv[2]; const dir = `/data/${format}`; await mkdir(dir, { recursive: true });
const rows = batch => Array.from({ length: 50 }, (_, i) => ({ id: `${batch}:${i}`, value: batch * 1000 + i, payload: 'x'.repeat(256) }));
const ack = batch => new Promise(resolve => process.stdout.write(`ack ${batch}\n`, resolve));
let write;
if (format.startsWith('sqlite')) {
  const db = new DatabaseSync(`${dir}/records.sqlite`);
  db.exec(`PRAGMA journal_mode=WAL; PRAGMA synchronous=${format === 'sqlite-wal-normal' ? 'NORMAL' : 'FULL'}; CREATE TABLE IF NOT EXISTS records(id TEXT PRIMARY KEY, value INTEGER NOT NULL, payload TEXT NOT NULL)`);
  const insert = db.prepare('INSERT INTO records VALUES (?,?,?)');
  write = async batch => { db.exec('BEGIN IMMEDIATE'); for (const r of rows(batch)) insert.run(r.id, r.value, r.payload); db.exec('COMMIT'); };
} else if (format.startsWith('jsonl')) {
  const file = await open(`${dir}/records.jsonl`, 'a');
  write = async batch => { await file.writeFile(rows(batch).map(r => JSON.stringify(r)).join('\n') + '\n'); if (format === 'jsonl-durable') await file.sync(); };
} else {
  const { DuckDBInstance } = await import('@duckdb/node-api');
  const instance = await DuckDBInstance.create(format === 'duckdb-table' ? `${dir}/records.duckdb` : ':memory:'); const connection = await instance.connect();
  await connection.run(`CREATE ${format === 'parquet-batch' ? 'TEMP ' : ''}TABLE IF NOT EXISTS records (id VARCHAR PRIMARY KEY, value BIGINT NOT NULL, payload VARCHAR NOT NULL)`);
  const append = async batch => { const a = await connection.createAppender('records'); for (const r of rows(batch)) { a.appendVarchar(r.id); a.appendBigInt(BigInt(r.value)); a.appendVarchar(r.payload); a.endRow(); } a.flushSync(); a.closeSync(); };
  write = format === 'duckdb-table'
    ? async batch => { await connection.run('BEGIN TRANSACTION'); await append(batch); await connection.run('COMMIT'); }
    : async batch => {
      await connection.run('DELETE FROM records'); await append(batch);
      const final = `${dir}/part-${batch}.parquet`, temporary = `${final}.tmp`;
      await connection.run(`COPY records TO '${temporary}' (FORMAT parquet)`);
      const part = await open(temporary, 'r+'); await part.sync(); await part.close(); await rename(temporary, final);
      const d = await open(dir, 'r'); await d.sync(); await d.close();
    };
}
for (let batch = 1; ; batch++) { await write(batch); await ack(batch); }
