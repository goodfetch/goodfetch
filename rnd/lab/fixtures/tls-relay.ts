// TLS behind a TCP relay: the relay reads the ClientHello, forwards every byte unchanged to an HTTP/2 (and HTTP/1.1) server,
// and indexes the fingerprint by its upstream port so each request can be tied to the ClientHello of its connection.
import { createServer as createTcpServer, connect as connectTcp, type AddressInfo, type Socket } from 'node:net';
import { createSecureServer, type Http2ServerRequest, type Http2ServerResponse } from 'node:http2';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { IncomingMessage } from 'node:http';
import { parseClientHello, type ClientHello } from './tls-fingerprint.ts';

// A self-signed certificate for localhost, made once per process with the system openssl.
let certificate: { key: Buffer; cert: Buffer } | undefined;
export function localCertificate() {
  if (!certificate) {
    const directory = mkdtempSync(join(tmpdir(), 'automan-tls-'));
    execFileSync('openssl', ['req', '-x509', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:prime256v1', '-nodes', '-days', '2', '-subj', '/CN=localhost', '-addext', 'subjectAltName=DNS:localhost', '-keyout', join(directory, 'key.pem'), '-out', join(directory, 'cert.pem')], { stdio: 'ignore' });
    certificate = { key: readFileSync(join(directory, 'key.pem')), cert: readFileSync(join(directory, 'cert.pem')) };
  }
  return certificate;
}

// HTTP/2 settings in the Akamai fingerprint order, and the pseudo-header order of the request.
export function http2Signature(req: Http2ServerRequest) {
  const settings = req.stream.session?.remoteSettings as Record<string, number | boolean> | undefined;
  const ids: [string, number][] = [['headerTableSize', 1], ['enablePush', 2], ['maxConcurrentStreams', 3], ['initialWindowSize', 4], ['maxFrameSize', 5], ['maxHeaderListSize', 6]];
  const values = ids.filter(([name]) => settings?.[name] !== undefined).map(([name, id]) => `${id}:${Number(settings![name])}`).join(';');
  const pseudo = req.rawHeaders.filter((_, i) => i % 2 === 0 && req.rawHeaders[i]!.startsWith(':')).map(name => name[1]).join('');
  return `${values}|${pseudo}`;
}

export type Fingerprint = { ja4: string | null; ja4Base: string | null; resumed: boolean; alpn: string[]; http: string; h2: string | null; headerOrder: string };
type AnyRequest = IncomingMessage | Http2ServerRequest;

export async function startTlsRelay(handle: (req: Http2ServerRequest, res: Http2ServerResponse) => void) {
  const helloByPort = new Map<number, ClientHello>();
  const secure = createSecureServer({ ...localCertificate(), allowHTTP1: true }, handle);
  secure.on('secureConnection', socket => { (socket as Socket & { clientHello?: ClientHello }).clientHello = helloByPort.get(socket.remotePort ?? -1); });
  await new Promise<void>(resolve => secure.listen(0, '127.0.0.1', resolve));
  const securePort = (secure.address() as AddressInfo).port;
  const sockets = new Set<Socket>();
  const tcp = createTcpServer(client => {
    const upstream = connectTcp(securePort, '127.0.0.1');
    for (const socket of [client, upstream]) { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); socket.on('error', () => { client.destroy(); upstream.destroy(); }); }
    let buffered = Buffer.alloc(0), parsed = false;
    client.on('data', (chunk: Buffer) => {
      if (!parsed) {
        buffered = Buffer.concat([buffered, chunk]);
        try { const hello = parseClientHello(buffered); if (hello) { parsed = true; const register = () => { helloByPort.set(upstream.localPort!, hello); upstream.write(buffered); }; if (upstream.localPort) register(); else upstream.once('connect', register); } } catch { client.destroy(); upstream.destroy(); }
        return;
      }
      upstream.write(chunk);
    });
    upstream.on('data', chunk => client.write(chunk));
    client.on('end', () => upstream.end()); upstream.on('end', () => client.end());
  });
  await new Promise<void>(resolve => tcp.listen(0, '127.0.0.1', resolve));
  return {
    origin: `https://localhost:${(tcp.address() as AddressInfo).port}`,
    // The fingerprint of a request that came through the relay, or null for a plain HTTP request.
    fingerprint(req: AnyRequest): Fingerprint | null {
      const socket = ('stream' in req ? req.stream.session?.socket : req.socket) as (Socket & { clientHello?: ClientHello }) | undefined;
      const hello = socket?.clientHello ?? helloByPort.get(socket?.remotePort ?? -1);
      if (!hello && !('stream' in req) && socket?.localPort !== securePort) return null;
      return { ja4: hello?.ja4 ?? null, ja4Base: hello?.ja4Base ?? null, resumed: hello?.resumed ?? false, alpn: hello?.alpn ?? [], http: req.httpVersion, h2: 'stream' in req ? http2Signature(req) : null, headerOrder: req.rawHeaders.filter((_, i) => i % 2 === 0 && !req.rawHeaders[i]!.startsWith(':')).join(',').toLowerCase() };
    },
    async close() { for (const socket of sockets) socket.destroy(); await Promise.all([new Promise<void>(resolve => tcp.close(() => resolve())), new Promise<void>(resolve => secure.close(() => resolve()))]); }
  };
}
