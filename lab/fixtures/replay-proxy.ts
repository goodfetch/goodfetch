// Frozen real pages (T1): an HTTP proxy that serves archived documents at their original host and path, over plain HTTP, and
// refuses everything else. Every browser of a campaign goes through it, so all of them see the same bytes and nothing reaches
// the live web: a request that is not in the archive, or an HTTPS tunnel, is refused and counted.
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { createServer as createHttpsServer } from 'node:https';
import { connect } from 'node:net';
import { localCertificate } from './tls-relay.ts';
import { gunzipSync } from 'node:zlib';
import { readFileSync } from 'node:fs';
import type { AddressInfo } from 'node:net';

// The URL a page is replayed at: its original host and path, scheme set to http.
// A page archived without its URL is served on a reserved host named after its file.
export const replayUrl = (original: string, fileId = 'page') => { if (!URL.canParse(original)) return `http://${fileId}.wcxb.invalid/`; const url = new URL(original); url.protocol = 'http:'; url.hash = ''; return url.href; };

export async function startReplayProxy(pages: { url: string; path: string; fileId?: string }[]) {
  const archive = new Map(pages.map(page => [replayUrl(page.url, page.fileId), page.path]));
  const stats = { hits: 0, misses: 0, tunnels: 0, missesByPage: new Map<string, number>() };
  let current = '';
  // Plain HTTP arrives in absolute form; HTTPS to an archived host is terminated here (HSTS-preloaded hosts are never loaded over
  // HTTP by Chrome) and looked up under the same key.
  const serve = (target: string | undefined) => (req: IncomingMessage, res: ServerResponse) => {
    const path = target && URL.canParse(target) ? archive.get(replayUrl(target)) : undefined;
    if (path) { stats.hits++; res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' }); res.end(gunzipSync(readFileSync(path))); return; }
    stats.misses++; stats.missesByPage.set(current, (stats.missesByPage.get(current) ?? 0) + 1);
    res.writeHead(404, { 'content-type': 'text/plain' }); res.end('not in the archive');
  };
  const server = createServer((req, res) => serve(req.url && /^http:\/\//.test(req.url) ? req.url : undefined)(req, res));
  const secure = createHttpsServer(localCertificate(), (req, res) => serve(`https://${req.headers.host}${req.url}`)(req, res));
  await new Promise<void>(resolve => secure.listen(0, '127.0.0.1', resolve));
  const archivedHosts = new Set([...archive.keys()].map(key => new URL(key).hostname));
  server.on('connect', (req, socket) => {
    const [host, port] = (req.url ?? '').split(':');
    if (!archivedHosts.has(host!) || port !== '443') { stats.tunnels++; stats.missesByPage.set(current, (stats.missesByPage.get(current) ?? 0) + 1); socket.end('HTTP/1.1 403 Forbidden\r\n\r\n'); return; }
    const upstream = connect((secure.address() as AddressInfo).port, '127.0.0.1', () => { socket.write('HTTP/1.1 200 Connection Established\r\n\r\n'); upstream.pipe(socket); socket.pipe(upstream); });
    for (const s of [socket, upstream]) s.on('error', () => { socket.destroy(); upstream.destroy(); });
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  return {
    url: `http://127.0.0.1:${(server.address() as AddressInfo).port}`, stats,
    // Misses are attributed to the page being loaded when they arrive.
    loading(url: string) { current = url; },
    close: async () => { server.closeAllConnections(); secure.closeAllConnections(); await Promise.all([new Promise<void>(resolve => server.close(() => resolve())), new Promise<void>(resolve => secure.close(() => resolve()))]); }
  };
}
