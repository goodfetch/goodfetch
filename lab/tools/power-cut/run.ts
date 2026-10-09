// Simulated power loss for the sinks (durability-power-loss): writers run inside the Linux VM of Docker Desktop and print an
// acknowledgement after each committed batch; PostgreSQL runs in the same VM and is written from the host. The VM process is then
// killed with SIGKILL, so whatever the guest kernel had not flushed to its virtual disk is lost, as on a power cut of the guest.
// After Docker restarts, every acknowledged batch must be present and whole. The host disk itself is not cut: this proves the
// sinks' flush discipline inside the guest, not the behaviour of the Mac's physical disk.
// Usage: AUTOMAN_PG_URL=... node tools/power-cut/run.ts <trials> <out.json>
import { spawn, execFileSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import { writeFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { pg, pgUrl } from '../../src/sinks.ts';

const here = dirname(fileURLToPath(import.meta.url));
const image = 'node:24.20.0-bookworm-slim', box = 'automan-lab-power', pgBox = 'automan-lab-pg';
const formats = ['sqlite-wal-full', 'sqlite-wal-normal', 'jsonl-durable', 'jsonl-buffered', 'duckdb-table', 'parquet-batch'];
const docker = (...args: string[]) => execFileSync('docker', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
const trials = Number(process.argv[2] ?? 3), out = process.argv[3] ?? 'power-cut.json';

// Docker Desktop does not bring a killed VM back on its own: it is restarted, then the daemon must answer container calls.
async function dockerUp(timeoutMs = 300_000) {
  try { execFileSync('docker', ['desktop', 'restart'], { stdio: 'ignore', timeout: 600_000 }); } catch { /* the restart may outlive its command; the daemon is polled below */ }
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) { try { docker('ps', '-a', '--format', '{{.Names}}'); return; } catch { await sleep(2000); } }
  throw new Error('Docker did not come back');
}
async function pgReady() { for (let i = 0; i < 90; i++) { try { docker('exec', pgBox, 'pg_isready', '-U', 'postgres', '-d', 'lab'); const c = new (pg().Client)({ connectionString: pgUrl() }); await c.connect(); await c.end(); return; } catch { await sleep(1000); } } throw new Error('PostgreSQL did not come back'); }

// The box: a Node container with the writer, the verifier and DuckDB's Linux build, data on the container's own disk.
if (!docker('ps', '-a', '--filter', `name=^${box}$`, '--format', '{{.Names}}')) {
  docker('run', '-d', '--name', box, '-w', '/opt/w', image, 'sleep', 'infinity');
  docker('exec', box, 'sh', '-c', 'mkdir -p /opt/w && cd /opt/w && npm init -y >/dev/null && npm install --silent @duckdb/node-api@1.5.5-r.5');
}
docker('start', box);
for (const file of ['writer.mjs', 'verify.mjs']) docker('cp', join(here, file), `${box}:/opt/w/${file}`);
// The tooling is flushed once, so the cuts only ever hit the data the writers produce.
docker('exec', box, 'sync');

const results: unknown[] = [];
for (let trial = 1; trial <= trials; trial++) {
  // Each trial writes under its own directory, so nothing has to be cleared between trials.
  const root = `/data/t${trial}-${Date.now()}`;
  const table = `power_t${trial}_${Date.now()}`;
  const setup = new (pg().Client)({ connectionString: pgUrl() }); await setup.connect(); await setup.query(`CREATE TABLE ${table} (id TEXT PRIMARY KEY, batch INT NOT NULL, value BIGINT NOT NULL, payload TEXT NOT NULL)`); await setup.end();
  const acked = new Map<string, number[]>(formats.concat('postgres').map(f => [f, []]));
  const writers = formats.map(format => { const child = spawn('docker', ['exec', '-e', `DATA_ROOT=${root}`, box, 'node', '/opt/w/writer.mjs', format], { stdio: ['ignore', 'pipe', 'pipe'] }); createInterface({ input: child.stdout }).on('line', line => { const m = line.match(/^ack (\d+)$/); if (m) acked.get(format)!.push(Number(m[1])); }); return child; });
  // PostgreSQL: one transaction per batch, acknowledged after COMMIT returns (synchronous_commit on).
  let pgRunning = true;
  const pgWriter = (async () => { const c = new (pg().Client)({ connectionString: pgUrl() }); await c.connect(); try { for (let batch = 1; pgRunning; batch++) { const values: unknown[] = []; const rows = Array.from({ length: 50 }, (_, i) => { values.push(`${batch}:${i}`, batch, batch * 1000 + i, 'x'.repeat(256)); return `($${i * 4 + 1},$${i * 4 + 2},$${i * 4 + 3},$${i * 4 + 4})`; }); await c.query('BEGIN'); await c.query(`INSERT INTO ${table} VALUES ${rows.join(',')}`, values); await c.query('COMMIT'); acked.get('postgres')!.push(batch); } } catch { /* the VM went away */ } finally { c.end().catch(() => {}); } })();
  await sleep(5000);
  const vm = execFileSync('pgrep', ['-f', 'com.docker.virtualization'], { encoding: 'utf8' }).trim().split('\n')[0]!;
  const ackedAtCut = Object.fromEntries([...acked].map(([f, b]) => [f, b.length]));
  process.kill(Number(vm), 'SIGKILL'); const cutAt = new Date().toISOString();
  pgRunning = false; for (const w of writers) w.kill('SIGKILL'); await Promise.race([pgWriter, sleep(3000)]);
  process.stderr.write(`trial ${trial}: VM ${vm} killed at ${cutAt}; acknowledged ${JSON.stringify(ackedAtCut)}\n`);
  await sleep(3000); await dockerUp(); docker('start', pgBox, box); await pgReady();
  const sinks: Record<string, unknown>[] = [];
  for (const format of formats) {
    const found = JSON.parse(docker('exec', '-e', `DATA_ROOT=${root}`, box, 'node', '/opt/w/verify.mjs', format)) as { integrity: string; rows: number; complete: number[]; partial: number[] };
    const present = new Set(found.complete); const lost = acked.get(format)!.filter(b => !present.has(b));
    sinks.push({ format, acknowledged: acked.get(format)!.length, persistedBatches: found.complete.length, lostAcknowledged: lost.length, lostBatches: lost.slice(0, 10), partialBatches: found.partial, integrity: found.integrity });
  }
  const c = new (pg().Client)({ connectionString: pgUrl() }); await c.connect();
  const rows = (await c.query(`SELECT batch, count(*)::int AS n FROM ${table} GROUP BY batch`)).rows as { batch: number; n: number }[]; await c.end();
  const present = new Set(rows.filter(r => r.n === 50).map(r => r.batch)); const lost = acked.get('postgres')!.filter(b => !present.has(b));
  sinks.push({ format: 'postgres', acknowledged: acked.get('postgres')!.length, persistedBatches: present.size, lostAcknowledged: lost.length, lostBatches: lost.slice(0, 10), partialBatches: rows.filter(r => r.n !== 50).map(r => r.batch), integrity: 'server recovered' });
  results.push({ trial, cutAt, vmPid: Number(vm), sinks });
  process.stderr.write(`trial ${trial}: ${sinks.map(s => `${s.format} lost ${s.lostAcknowledged}/${s.acknowledged}`).join(', ')}\n`);
  await writeFile(out, JSON.stringify({ schemaVersion: 1, method: 'SIGKILL of the Docker Desktop VM (com.docker.virtualization) while writers commit; host disk not cut', image, docker: docker('version', '--format', '{{.Server.Version}}'), results }, null, 1) + '\n');
}
