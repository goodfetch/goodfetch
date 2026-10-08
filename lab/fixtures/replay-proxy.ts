// Frozen real pages (T1): an HTTP proxy that serves archived documents at their original host and path, over plain HTTP, and
// refuses everything else. Every browser of a campaign goes through it, so all of them see the same bytes and nothing reaches
// the live web: a request that is not in the archive, or an HTTPS tunnel, is refused and counted.
import { createServer } from 'node:http';
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
  const server = createServer((req, res) => {
    const path = req.url && /^http:\/\//.test(req.url) ? archive.get(new URL(req.url).href) : undefined;
    if (path) { stats.hits++; res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' }); res.end(gunzipSync(readFileSync(path))); return; }
    stats.misses++; stats.missesByPage.set(current, (stats.missesByPage.get(current) ?? 0) + 1);
    res.writeHead(404, { 'content-type': 'text/plain' }); res.end('not in the archive');
  });
  server.on('connect', (_req, socket) => { stats.tunnels++; stats.missesByPage.set(current, (stats.missesByPage.get(current) ?? 0) + 1); socket.end('HTTP/1.1 403 Forbidden\r\n\r\n'); });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  return {
    url: `http://127.0.0.1:${(server.address() as AddressInfo).port}`, stats,
    // Misses are attributed to the page being loaded when they arrive.
    loading(url: string) { current = url; },
    close: () => new Promise<void>(resolve => { server.closeAllConnections(); server.close(() => resolve()); })
  };
}
