// TLS ClientHello parsing and JA4 fingerprint (FoxIO spec), computed from the first bytes of a connection.
import { createHash } from 'node:crypto';

const grease = (value: number) => (value & 0x0f0f) === 0x0a0a;
const hex4 = (value: number) => value.toString(16).padStart(4, '0');
const sha12 = (text: string) => createHash('sha256').update(text).digest('hex').slice(0, 12);

export type ClientHello = { ja4: string; ja4Base: string; resumed: boolean; sni: string | null; alpn: string[]; version: string; ciphers: number[]; extensions: number[]; signatureAlgorithms: number[] };

// Returns null until the whole first TLS record has arrived.
export function parseClientHello(buffer: Buffer): ClientHello | null {
  if (buffer.length < 5) return null;
  if (buffer[0] !== 22) throw new Error('not a TLS handshake');
  const recordEnd = 5 + buffer.readUInt16BE(3);
  if (buffer.length < recordEnd) return null;
  let at = 5; if (buffer[at] !== 1) throw new Error('not a ClientHello'); at += 4;
  const legacyVersion = buffer.readUInt16BE(at); at += 2 + 32;
  at += 1 + buffer[at]!;
  const cipherEnd = at + 2 + buffer.readUInt16BE(at); at += 2;
  const ciphers: number[] = []; for (; at < cipherEnd; at += 2) ciphers.push(buffer.readUInt16BE(at));
  at += 1 + buffer[at]!;
  const extensionsEnd = at + 2 + buffer.readUInt16BE(at); at += 2;
  const extensions: number[] = []; let sni: string | null = null; const alpn: string[] = []; let versions: number[] = []; let signatureAlgorithms: number[] = [];
  while (at < Math.min(extensionsEnd, recordEnd)) {
    const type = buffer.readUInt16BE(at), length = buffer.readUInt16BE(at + 2), data = buffer.subarray(at + 4, at + 4 + length); at += 4 + length;
    if (grease(type)) continue;
    extensions.push(type);
    if (type === 0x0000 && data.length > 5) sni = data.subarray(5, 5 + data.readUInt16BE(3)).toString();
    if (type === 0x0010) for (let i = 2; i < data.length;) { const n = data[i]!; alpn.push(data.subarray(i + 1, i + 1 + n).toString()); i += 1 + n; }
    if (type === 0x002b) for (let i = 1; i + 1 < data.length; i += 2) versions.push(data.readUInt16BE(i));
    if (type === 0x000d) for (let i = 2; i + 1 < data.length; i += 2) signatureAlgorithms.push(data.readUInt16BE(i));
  }
  versions = versions.filter(v => !grease(v));
  const top = versions.length ? Math.max(...versions) : legacyVersion;
  const version = ({ 0x0304: '13', 0x0303: '12', 0x0302: '11', 0x0301: '10' } as Record<number, string>)[top] ?? '00';
  const realCiphers = ciphers.filter(c => !grease(c));
  const first = alpn[0] ?? ''; const alpnCode = first ? `${first[0]}${first.at(-1)}` : '00';
  const count = (n: number) => String(Math.min(n, 99)).padStart(2, '0');
  const b = sha12([...realCiphers].sort((x, y) => x - y).map(hex4).join(','));
  // GREASE is ignored everywhere, signature algorithms included: Chrome draws a new GREASE value for each connection.
  signatureAlgorithms = signatureAlgorithms.filter(s => !grease(s));
  const ja4Of = (exts: number[]) => {
    const a = `t${version}${sni !== null ? 'd' : 'i'}${count(realCiphers.length)}${count(exts.length)}${alpnCode}`;
    const sortedExtensions = exts.filter(e => e !== 0x0000 && e !== 0x0010).sort((x, y) => x - y).map(hex4).join(',');
    return `${a}_${b}_${sha12(signatureAlgorithms.length ? `${sortedExtensions}_${signatureAlgorithms.map(hex4).join(',')}` : sortedExtensions)}`;
  };
  // A resumed handshake adds pre_shared_key (and possibly early_data); ja4Base leaves them out so one client compares with itself.
  const resumed = extensions.includes(0x0029);
  return { ja4: ja4Of(extensions), ja4Base: ja4Of(extensions.filter(e => e !== 0x0029 && e !== 0x002a)), resumed, sni, alpn, version, ciphers: realCiphers, extensions, signatureAlgorithms };
}
