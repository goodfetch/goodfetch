// Sinks beyond SQLite and JSONL (D-sinks): PostgreSQL through COPY, a DuckDB table, and one Parquet file per batch.
// The storage worker writes through them and the storage oracle reads them back on its own connection.
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { labRoot } from './registry.ts';

const load = createRequire(join(labRoot, 'adapters/node/package.json'));
export type PgClient = { connect(): Promise<void>; on(event: 'error', listener: (error: Error) => void): void; query(sql: string, values?: unknown[]): Promise<{ rows: Record<string, unknown>[] }>; end(): Promise<void> };
export const pg = () => load('pg') as { Client: new (options: { connectionString: string }) => PgClient };
export const copyFrom = () => (load('pg-copy-streams') as { from: (sql: string) => NodeJS.WritableStream }).from;
type DuckValue = { getRowObjectsJS(): Promise<Record<string, unknown>[]> };
export type DuckConnection = { run(sql: string): Promise<unknown>; runAndReadAll(sql: string): Promise<DuckValue>; createAppender(table: string): Promise<{ appendVarchar(v: string): void; appendBigInt(v: bigint): void; endRow(): void; flushSync(): void; closeSync(): void }>; closeSync(): void };
export const duckdb = () => load('@duckdb/node-api') as { DuckDBInstance: { create(path: string, options?: Record<string, string>): Promise<{ connect(): Promise<DuckConnection>; closeSync(): void }> } };

// PostgreSQL runs as a server; one fresh table per cell, named after the cell's store path.
export const pgUrl = () => { const url = process.env.AUTOMAN_PG_URL; if (!url) throw new Error('AUTOMAN_PG_URL is not set'); return url; };
export const pgTable = (path: string) => `records_${createHash('sha256').update(path).digest('hex').slice(0, 16)}`;
export const sinkFormats = ['postgres-copy', 'duckdb-table', 'parquet-batch'] as const;
