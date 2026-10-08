// Proxy egress of a session (ADR-002 §3, proxies): the session's browser and HTTP tier always talk to one local egress, which
// tunnels each connection through the session's sticky upstream proxy. On a failed upstream the egress marks it down for a
// cooldown, moves the session to another healthy proxy and charges the session's retry budget; the browser context, its
// cookies and its TLS state stay the same.
import { createServer, type Server } from 'node:http';
import { connect, type Socket, type AddressInfo } from 'node:net';

export type UpstreamProxy = { url: string; user: string; password: string };

// Health and sticky assignment shared by every session of a worker.
export class ProxyPool {
  readonly proxies: UpstreamProxy[]; readonly cooldownMs: number;
  private downUntil = new Map<number, number>(); private assigned = new Map<string, number>(); private load = new Map<number, number>();
  constructor(proxies: UpstreamProxy[], cooldownMs = 30_000) { this.proxies = proxies; this.cooldownMs = cooldownMs; }
  private healthy(index: number) { return (this.downUntil.get(index) ?? 0) <= performance.now(); }
  // The session keeps its proxy while it is healthy; otherwise it takes the least-loaded healthy one.
  stickyFor(session: string): number | null {
    const current = this.assigned.get(session); if (current !== undefined && this.healthy(current)) return current;
    const candidates = this.proxies.map((_, i) => i).filter(i => this.healthy(i)).sort((a, b) => (this.load.get(a) ?? 0) - (this.load.get(b) ?? 0) || a - b);
    const next = candidates[0]; if (next === undefined) return null;
    if (current !== undefined) this.load.set(current, (this.load.get(current) ?? 1) - 1);
    this.assigned.set(session, next); this.load.set(next, (this.load.get(next) ?? 0) + 1); return next;
  }
  markDown(index: number) { this.downUntil.set(index, performance.now() + this.cooldownMs); }
  release(session: string) { const current = this.assigned.get(session); if (current !== undefined) this.load.set(current, (this.load.get(current) ?? 1) - 1); this.assigned.delete(session); }
}

// CONNECT through one upstream with the session's sticky credentials; resolves with the open tunnel or rejects.
function tunnelThrough(proxy: UpstreamProxy, session: string, target: string, timeoutMs: number): Promise<Socket> {
  const { hostname, port } = new URL(proxy.url);
  return new Promise((resolve, reject) => {
    const socket = connect(Number(port), hostname); const timer = setTimeout(() => { socket.destroy(); reject(new Error('upstream proxy timeout')); }, timeoutMs);
    let head = Buffer.alloc(0);
    const fail = (error: Error) => { clearTimeout(timer); socket.destroy(); reject(error); };
    socket.once('error', fail);
    socket.once('connect', () => socket.write(`CONNECT ${target} HTTP/1.1\r\nHost: ${target}\r\nProxy-Authorization: Basic ${Buffer.from(`${proxy.user}-session-${session}:${proxy.password}`).toString('base64')}\r\n\r\n`));
    const onData = (chunk: Buffer) => {
      head = Buffer.concat([head, chunk]); const end = head.indexOf('\r\n\r\n'); if (end < 0) return;
      socket.off('data', onData); clearTimeout(timer); socket.off('error', fail);
      if (!/^HTTP\/1\.[01] 200/.test(head.subarray(0, end).toString())) { socket.destroy(); reject(new Error(`upstream refused: ${head.subarray(0, end).toString().split('\r\n')[0]}`)); return; }
      if (head.length > end + 4) socket.unshift(head.subarray(end + 4));
      resolve(socket);
    };
    socket.on('data', onData);
  });
}

export class ProxyExhaustedError extends Error { readonly code = 'E_PROXY_EXHAUSTED'; }

export async function startEgress(pool: ProxyPool, session: string, budget: { retries: number; limit: number }, timeoutMs = 5000) {
  const state = { exhausted: false, failovers: 0, bytes: new Map<number, number>() };
  const sockets = new Set<Socket>();
  const server: Server = createServer((_req, res) => { res.writeHead(405).end(); });
  server.on('connect', async (req, client: Socket) => {
    sockets.add(client); client.on('close', () => sockets.delete(client)); client.on('error', () => client.destroy());
    for (;;) {
      const index = pool.stickyFor(session);
      if (index === null) { state.exhausted = true; client.end('HTTP/1.1 502 Bad Gateway\r\nX-Automan-Error: E_PROXY_EXHAUSTED\r\n\r\n'); return; }
      try {
        const upstream = await tunnelThrough(pool.proxies[index]!, session, req.url ?? '', timeoutMs);
        sockets.add(upstream); upstream.on('close', () => sockets.delete(upstream)); upstream.on('error', () => { upstream.destroy(); client.destroy(); });
        upstream.on('data', chunk => state.bytes.set(index, (state.bytes.get(index) ?? 0) + chunk.length));
        client.write('HTTP/1.1 200 Connection Established\r\n\r\n'); upstream.pipe(client); client.pipe(upstream); return;
      } catch {
        // A failed upstream is a rotation: it costs one retry from the session budget and waits out its cooldown.
        pool.markDown(index); state.failovers++;
        if (budget.retries >= budget.limit) { state.exhausted = true; client.end('HTTP/1.1 502 Bad Gateway\r\nX-Automan-Error: E_PROXY_EXHAUSTED\r\n\r\n'); return; }
        budget.retries++;
      }
    }
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  return {
    url: `http://127.0.0.1:${(server.address() as AddressInfo).port}`, state,
    close: () => new Promise<void>(resolve => { for (const socket of sockets) socket.destroy(); server.close(() => resolve()); pool.release(session); })
  };
}
