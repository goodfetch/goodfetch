// Several sites behind one local server, one path prefix per domain: /d/<domain>/job/<jobId>. Each answer takes the domain's
// service time; the server counts fetches per job and the peak number of requests in flight per domain.
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import type { AddressInfo } from 'node:net';

// The records a job must return, derived from its id so the oracle needs no shared state with the runtime.
export const expectedRecords = (jobId: string) => { const h = createHash('sha256').update(jobId).digest(); return [{ id: `${jobId}-a`, price: 1000 + h.readUInt16BE(0) % 9000 }, { id: `${jobId}-b`, price: 1000 + h.readUInt16BE(2) % 9000 }]; };

export async function startDomainSite(serviceMs: number) {
  const fetches = new Map<string, { domain: string; count: number; startedAt: number; endedAt: number }>();
  const inflight = new Map<string, number>(); const peak = new Map<string, number>();
  const server = createServer((req, res) => {
    const match = req.url?.match(/^\/d\/([a-z0-9-]+)\/job\/([A-Za-z0-9-]+)$/); if (!match) { res.writeHead(404).end(); return; }
    const [, domain, jobId] = match as unknown as [string, string, string]; const startedAt = performance.now();
    const running = (inflight.get(domain) ?? 0) + 1; inflight.set(domain, running); peak.set(domain, Math.max(peak.get(domain) ?? 0, running));
    const seen = fetches.get(jobId); fetches.set(jobId, { domain, count: (seen?.count ?? 0) + 1, startedAt: seen?.startedAt ?? startedAt, endedAt: 0 });
    setTimeout(() => {
      inflight.set(domain, (inflight.get(domain) ?? 1) - 1); fetches.get(jobId)!.endedAt = performance.now();
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      res.end(`<main>${expectedRecords(jobId).map(r => `<article data-id="${r.id}" data-price="${r.price}"></article>`).join('')}</main>`);
    }, serviceMs);
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  return { origin, url: (domain: string, jobId: string) => `${origin}/d/${domain}/job/${jobId}`, fetches, peak, close: () => new Promise<void>(resolve => { server.closeAllConnections(); server.close(() => resolve()); }) };
}
