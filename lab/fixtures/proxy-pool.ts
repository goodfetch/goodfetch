// Local upstream proxies for the identity lane: HTTP CONNECT with Basic credentials, the username naming a sticky session as
// residential providers do ("user-session-<id>"). Each proxy records its tunnels and can be taken down at once, closing them.
import { createServer, type Server } from 'node:http';
import { connect, type Socket, type AddressInfo } from 'node:net';

export type Tunnel = { proxy: number; session: string | null; target: string; at: number; bytes: number };

export async function startProxyPool(size: number, credentials = { user: 'lab', password: 'fixture' }) {
  const tunnels: Tunnel[] = []; const down = new Set<number>(); const servers: Server[] = []; const sockets = new Map<number, Set<Socket>>();
  const ports: number[] = [];
  for (let index = 0; index < size; index++) {
    const open = new Set<Socket>(); sockets.set(index, open);
    const server = createServer((_req, res) => { res.writeHead(405).end(); });
    server.on('connect', (req, client: Socket) => {
      const auth = Buffer.from((req.headers['proxy-authorization'] ?? '').replace(/^Basic /, ''), 'base64').toString();
      const [user = '', password] = auth.split(':');
      if (!user.startsWith(`${credentials.user}-session-`) && user !== credentials.user || password !== credentials.password) { client.end('HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic\r\n\r\n'); return; }
      const [host, port] = (req.url ?? '').split(':'); const tunnel: Tunnel = { proxy: index, session: user.startsWith(`${credentials.user}-session-`) ? user.slice(`${credentials.user}-session-`.length) : null, target: req.url ?? '', at: performance.now(), bytes: 0 };
      tunnels.push(tunnel);
      const upstream = connect(Number(port), host === 'localhost' ? '127.0.0.1' : host!, () => { client.write('HTTP/1.1 200 Connection Established\r\n\r\n'); upstream.pipe(client); client.pipe(upstream); });
      for (const socket of [client, upstream]) { open.add(socket); socket.on('close', () => open.delete(socket)); socket.on('error', () => { client.destroy(); upstream.destroy(); }); }
      upstream.on('data', chunk => { tunnel.bytes += chunk.length; }); client.on('data', chunk => { tunnel.bytes += chunk.length; });
    });
    await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
    ports.push((server.address() as AddressInfo).port); servers.push(server);
  }
  return {
    urls: ports.map(port => `http://127.0.0.1:${port}`), credentials, tunnels, down,
    // Sticky sessions: each session's tunnels go through one proxy, and move to another only after that proxy went down.
    judge(variant: string, sessions: string[], downAt: Map<number, number>) {
      const reasons: string[] = [];
      for (const session of sessions) {
        const own = tunnels.filter(t => t.session === session); const order = own.map(t => t.proxy).filter((p, i, all) => i === 0 || all[i - 1] !== p);
        if (!own.length && variant !== 'proxy-exhausted') reasons.push(`session ${session} opened no tunnel`);
        for (let i = 1; i < own.length; i++) if (own[i]!.proxy !== own[i - 1]!.proxy && !(downAt.has(own[i - 1]!.proxy) && downAt.get(own[i - 1]!.proxy)! <= own[i]!.at)) reasons.push(`session ${session} changed proxy without a failure`);
        if (new Set(order).size !== order.length) reasons.push(`session ${session} came back to a proxy it had left`);
        const expected = variant === 'proxy-sticky' ? 1 : variant === 'proxy-failover' ? 2 : null;
        if (expected !== null && new Set(order).size !== expected) reasons.push(`session ${session} used ${new Set(order).size} proxies, expected ${expected}`);
      }
      if (tunnels.some(t => t.session === null || !sessions.includes(t.session))) reasons.push('a tunnel without the sticky session of a known session');
      return reasons;
    },
    // A proxy that goes down refuses new connections and drops its open tunnels.
    takeDown(index: number) { if (down.has(index)) return; down.add(index); servers[index]!.close(); for (const socket of sockets.get(index)!) socket.destroy(); },
    close: async () => { for (const [index] of servers.entries()) if (!down.has(index)) { for (const socket of sockets.get(index)!) socket.destroy(); await new Promise<void>(resolve => servers[index]!.close(() => resolve())); } }
  };
}
