// User-space TCP latency proxy: adds half the target RTT in each direction, preserving byte order.
// It needs no privileges and runs on macOS and Linux; packet loss and jitter require netem on the reference host.
import { createServer, connect, type Socket } from 'node:net';
import type { Metric } from '../core/decisions.ts';
import type { Availability } from '../core/metrics/registry.ts';

// medianMs and p95Ms describe the shaped round trip; addedMedianMs is the paired median of shaped minus direct,
// so loopback cost and host drift are not charged to the profile.
export type Calibration = { rttMs: number; medianMs: number; p95Ms: number; baselineMedianMs: number; addedMedianMs: number; samples: number; withinTolerance: boolean };

// One FIFO queue per direction: chunks leave in arrival order, each no earlier than its due time.
// Timers run on libuv's cached loop clock and may fire early, so the drain re-arms until the head is really due.
function pipeDelayed(source: Socket, target: Socket, delayMs: number) {
  const queue: { due: number; chunk: Buffer }[] = [];
  let timer: NodeJS.Timeout | undefined, ended = false;
  const drain = () => {
    timer = undefined;
    while (queue.length && queue[0]!.due <= performance.now()) { const { chunk } = queue.shift()!; if (!target.destroyed) target.write(chunk); }
    if (queue.length) timer = setTimeout(drain, queue[0]!.due - performance.now());
    else if (ended) target.end();
  };
  source.on('data', (chunk: Buffer) => {
    if (delayMs === 0 && queue.length === 0) { if (!target.destroyed) target.write(chunk); return; }
    queue.push({ due: performance.now() + delayMs, chunk });
    if (!timer) timer = setTimeout(drain, delayMs);
  });
  source.on('end', () => { ended = true; if (!queue.length) target.end(); });
  source.on('close', () => { if (timer) clearTimeout(timer); });
}

export async function startLatencyProxy({ host, port, rttMs }: { host: string; port: number; rttMs: number }) {
  const sockets = new Set<Socket>();
  const server = createServer(client => {
    const upstream = connect(port, host);
    for (const socket of [client, upstream]) { socket.setNoDelay(true); sockets.add(socket); socket.on('close', () => sockets.delete(socket)); socket.on('error', () => { client.destroy(); upstream.destroy(); }); }
    pipeDelayed(client, upstream, rttMs / 2); pipeDelayed(upstream, client, rttMs / 2);
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const address = server.address() as { port: number };
  return { host: '127.0.0.1', port: address.port, async close() { for (const socket of sockets) socket.destroy(); await new Promise<void>(resolve => server.close(() => resolve())); } };
}

export const withinTolerance = (targetMs: number, measuredMs: number) => Math.abs(measuredMs - targetMs) <= Math.max(2, 0.1 * targetMs);

// Measures the shaped round trip with one-byte ping-pongs, interleaved with direct ping-pongs to the same echo server.
export async function calibrate(rttMs: number, samples = 50): Promise<Calibration> {
  const echo = createServer(socket => { socket.setNoDelay(true); socket.on('data', data => socket.write(data)); socket.on('error', () => socket.destroy()); });
  await new Promise<void>(resolve => echo.listen(0, '127.0.0.1', resolve));
  const echoPort = (echo.address() as { port: number }).port;
  const proxy = await startLatencyProxy({ host: '127.0.0.1', port: echoPort, rttMs });
  const open = async (port: number) => {
    const socket = connect(port, '127.0.0.1'); socket.setNoDelay(true);
    await new Promise<void>((resolve, reject) => { socket.once('connect', resolve); socket.once('error', reject); });
    return socket;
  };
  const roundTrip = async (socket: Socket) => {
    const began = performance.now();
    await new Promise<void>(resolve => { socket.once('data', () => resolve()); socket.write('x'); });
    return performance.now() - began;
  };
  const direct = await open(echoPort), shaped = await open(proxy.port);
  const shapedMs: number[] = [], directMs: number[] = [], addedMs: number[] = [];
  try {
    for (let i = 0; i < samples; i++) {
      const d = await roundTrip(direct), s = await roundTrip(shaped);
      directMs.push(d); shapedMs.push(s); addedMs.push(s - d);
    }
  } finally { direct.destroy(); shaped.destroy(); await proxy.close(); await new Promise<void>(resolve => echo.close(() => resolve())); }
  const median = (values: number[]) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]!;
  const sorted = [...shapedMs].sort((a, b) => a - b), addedMedianMs = median(addedMs);
  return { rttMs, medianMs: median(shapedMs), p95Ms: sorted[Math.min(sorted.length - 1, Math.ceil(0.95 * sorted.length) - 1)]!, baselineMedianMs: median(directMs), addedMedianMs, samples, withinTolerance: withinTolerance(rttMs, addedMedianMs) };
}

// Timing metrics are meaningless when the shaped network is not the declared one; contract metrics stay valid.
export function networkAvailability(metrics: Metric[], calibration: Calibration): Availability {
  if (calibration.withinTolerance) return {};
  const reason = `network calibration out of tolerance: declared ${calibration.rttMs} ms, measured added median ${calibration.addedMedianMs.toFixed(1)} ms`;
  return Object.fromEntries(metrics.filter(metric => metric.kind === 'performance' || metric.kind === 'resource').map(metric => [metric.id, { available: false, reason }]));
}
