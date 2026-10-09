// Topology lane (D-cloud-topology): how a hosted browser session is handed to a tenant. A session is one tenant visiting a page
// that sets a tenant cookie; the oracle requires the page to be served, the cookie to be set, and the next tenant never to see
// the previous tenant's cookie. Policies: a fresh container per session (cold), a pool of started containers discarded after
// use and refilled in the background (warm-pool), and a pool that hands containers back without reset (reuse-pool, the negative
// case). Firecracker snapshots need /dev/kvm, which the Docker Desktop VM does not expose: the cell probes it and is unsupported.
import { createServer } from 'node:http';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import type { AddressInfo } from 'node:net';
import { CdpConnection, CdpPage } from '../adapters/cdp-node/cdp.ts';
import { quantiles } from './statistics.ts';

const run = promisify(execFile);
export const topologyPolicies = ['cold-container', 'warm-pool', 'reuse-pool', 'firecracker-snapshot'] as const;
export type TopologyPolicy = typeof topologyPolicies[number];
export type TopologyConfig = { policies: TopologyPolicy[]; image: string; sessions: number; poolSize: number; blocks: number; seed: number; timeoutMs: number };
export type TopologyCell = { policy: TopologyPolicy; variant: 'isolation' };
export const topologyCombinations = (config: TopologyConfig): TopologyCell[] => config.policies.map(policy => ({ policy, variant: 'isolation' }));

type Container = { id: string; cdpPort: number };
const docker = async (...args: string[]) => (await run('docker', args, { timeout: 60_000 })).stdout.trim();
async function start(image: string, timeoutMs: number): Promise<Container> {
  const id = await docker('run', '-d', '--rm', '--label', 'automan-lab=topology', '-p', '127.0.0.1::9222', image);
  const cdpPort = Number((await docker('port', id, '9222')).split('\n')[0]!.split(':').pop());
  const deadline = performance.now() + timeoutMs;
  for (;;) { try { const response = await fetch(`http://127.0.0.1:${cdpPort}/json/version`); if (response.ok) return { id, cdpPort }; } catch { /* not listening yet */ } if (performance.now() > deadline) throw new Error('container browser did not start'); await new Promise(resolve => setTimeout(resolve, 20)); }
}
const stop = (container: Container) => docker('rm', '-f', container.id).catch(() => '');

// One tenant session: open the page twice; the first visit must arrive without a cookie and set the tenant's, the second must carry it.
async function session(container: Container, url: string, tenant: string, timeoutMs: number) {
  const { webSocketDebuggerUrl } = await (await fetch(`http://127.0.0.1:${container.cdpPort}/json/version`)).json() as { webSocketDebuggerUrl: string };
  const endpoint = new URL(webSocketDebuggerUrl); endpoint.hostname = '127.0.0.1'; endpoint.port = String(container.cdpPort);
  const connection = await CdpConnection.connect(endpoint.href);
  try {
    const page = await CdpPage.open(connection, timeoutMs); const seen: string[] = [];
    for (let visit = 0; visit < 2; visit++) { await page.goto(`${url}?tenant=${tenant}`); seen.push(await page.evaluate<string>('document.title')); }
    await page.close(); return seen;
  } finally { connection.close(); }
}

export async function runTopologyCell(config: TopologyConfig, cell: TopologyCell, block: number): Promise<Record<string, unknown>> {
  if (cell.policy === 'firecracker-snapshot') {
    const kvm = await docker('run', '--rm', '--device', '/dev/kvm', config.image, '--version').then(() => true, () => false);
    return { ...cell, block, status: 'unsupported', reason: kvm ? 'firecracker is not packaged in this lab' : 'no /dev/kvm in the Docker Desktop VM: Firecracker needs KVM, available on a Linux host or a bare-metal instance' };
  }
  // The tenant page: it reports the cookie it receives in the title and sets the visiting tenant's cookie.
  const server = createServer((req, res) => { const tenant = new URL(req.url ?? '/', 'http://x').searchParams.get('tenant'); res.setHeader('set-cookie', `tenant=${tenant}; Path=/`); res.setHeader('content-type', 'text/html'); res.end(`<title>${req.headers.cookie ?? 'none'}</title>`); });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const url = `http://host.docker.internal:${(server.address() as AddressInfo).port}/`;
  const acquireMs: number[] = [], taskMs: number[] = [], reasons: string[] = []; let leaks = 0, failures = 0; const live = new Set<Container>();
  const pool: Promise<Container>[] = [];
  const refill = () => { const next = start(config.image, config.timeoutMs); next.then(c => live.add(c), () => {}); pool.push(next); };
  try {
    if (cell.policy !== 'cold-container') { for (let i = 0; i < config.poolSize; i++) refill(); await Promise.all(pool); }
    for (let n = 0; n < config.sessions; n++) {
      const tenant = `t${block}-${n}`; const began = performance.now();
      try {
        const container = cell.policy === 'cold-container' ? await start(config.image, config.timeoutMs) : await pool.shift()!;
        live.add(container); const acquired = performance.now(); acquireMs.push(acquired - began);
        const seen = await session(container, url, tenant, config.timeoutMs); taskMs.push(performance.now() - began);
        if (seen[0] !== 'none') { leaks++; reasons.push(`${tenant} arrived with "${seen[0]}"`); }
        if (seen[1] !== `tenant=${tenant}`) { failures++; reasons.push(`${tenant} second visit saw "${seen[1]}"`); }
        // Cold and warm discard the container after the session; reuse hands it back to the pool as it is.
        if (cell.policy === 'reuse-pool') pool.push(Promise.resolve(container)); else { live.delete(container); void stop(container); if (cell.policy === 'warm-pool') refill(); }
      } catch (error) { failures++; reasons.push(`${tenant}: ${error instanceof Error ? error.message : String(error)}`); if (cell.policy !== 'cold-container' && pool.length < config.poolSize) refill(); }
    }
    // The memory a warm pool keeps: idle containers waiting for a tenant.
    await Promise.allSettled(pool);
    const idle = [...live][0]; let idleBytes: number | null = null;
    if (idle) { const usage = await docker('stats', '--no-stream', '--format', '{{.MemUsage}}', idle.id).catch(() => ''); const m = usage.match(/([\d.]+)\s*([KMG]i?B)/); if (m) idleBytes = Number(m[1]) * ({ KiB: 1024, MiB: 1024 ** 2, GiB: 1024 ** 3, KB: 1e3, MB: 1e6, GB: 1e9 } as Record<string, number>)[m[2]!]!; }
    const verified = !leaks && !failures;
    return { ...cell, block, status: verified ? 'verified' : 'failed', sessions: config.sessions, leaks, failures, startupMs: quantiles(acquireMs).p50, taskP50MsVerified: verified ? quantiles(taskMs).p50 : null,
      idleContainerBytes: idleBytes, poolBytes: idleBytes !== null && cell.policy !== 'cold-container' ? idleBytes * config.poolSize : 0, reasons: reasons.slice(0, 8) };
  } finally {
    server.close();
    for (const pending of pool) pending.then(c => stop(c), () => {});
    for (const container of live) await stop(container);
  }
}
