// Runs inside the restarted VM: prints, as JSON, the batches whose 50 rows are all present and intact, and the integrity check.
import { DatabaseSync } from 'node:sqlite';
import { readFile, readdir } from 'node:fs/promises';
const format = process.argv[2]; const dir = `/data/${format}`; let records = [], integrity = 'ok';
try {
  if (format.startsWith('sqlite')) { const db = new DatabaseSync(`${dir}/records.sqlite`); integrity = Object.values(db.prepare('PRAGMA integrity_check').get())[0]; records = db.prepare('SELECT id, value, payload FROM records').all(); db.close(); }
  else if (format.startsWith('jsonl')) { const lines = (await readFile(`${dir}/records.jsonl`, 'utf8')).split('\n').filter(Boolean); for (const line of lines) { try { records.push(JSON.parse(line)); } catch { integrity = 'torn trailing line'; } } }
  else {
    const { DuckDBInstance } = await import('@duckdb/node-api');
    const parts = format === 'parquet-batch' ? (await readdir(dir)).filter(f => f.endsWith('.parquet')) : [];
    const instance = await DuckDBInstance.create(format === 'duckdb-table' ? `${dir}/records.duckdb` : ':memory:'); const connection = await instance.connect();
    if (format === 'duckdb-table' || parts.length) records = await (await connection.runAndReadAll(`SELECT id, CAST(value AS INTEGER) AS value, payload FROM ${format === 'duckdb-table' ? 'records' : `read_parquet('${dir}/*.parquet')`}`)).getRowObjectsJS();
  }
} catch (error) { integrity = `unreadable: ${String(error).slice(0, 200)}`; }
const counts = new Map(); for (const r of records) { const [batch, i] = String(r.id).split(':').map(Number); if (r.value === batch * 1000 + i && r.payload === 'x'.repeat(256)) counts.set(batch, (counts.get(batch) ?? 0) + 1); }
console.log(JSON.stringify({ format, integrity, rows: records.length, complete: [...counts].filter(([, n]) => n === 50).map(([b]) => b).sort((a, b) => a - b), partial: [...counts].filter(([, n]) => n !== 50).map(([b]) => b) }));
