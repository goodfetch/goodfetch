import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { parseClientHello } from '../fixtures/tls-fingerprint.ts';

// A minimal TLS 1.3 ClientHello: GREASE in ciphers and extensions, SNI, ALPN h2, supported_versions, signature algorithms.
function hello({ sigGrease = 0x0a0a, psk = false } = {}) {
  const u16 = (n: number) => [n >> 8, n & 0xff];
  const ext = (type: number, data: number[]) => [...u16(type), ...u16(data.length), ...data];
  const sni = [...u16(12), 0, ...u16(9), ...Buffer.from('localhost')];
  const alpn = [...u16(3), 2, ...Buffer.from('h2')];
  const extensions = [...ext(0x1a1a, []), ...ext(0x0000, sni), ...ext(0x0010, alpn), ...ext(0x002b, [2, ...u16(0x0304)]), ...ext(0x000d, [...u16(6), ...u16(sigGrease), ...u16(0x0403), ...u16(0x0804)]), ...(psk ? ext(0x0029, [0, 0]) : [])];
  const body = [...u16(0x0303), ...new Array(32).fill(7), 0, ...u16(6), ...u16(0x0a0a), ...u16(0x1302), ...u16(0x1301), 1, 0, ...u16(extensions.length), ...extensions];
  const handshake = [1, 0, ...u16(body.length), ...body];
  return Buffer.from([22, 3, 1, ...u16(handshake.length), ...handshake]);
}

test('JA4 is computed from the ClientHello without GREASE, with sorted ciphers and extensions', () => {
  const sha = (text: string) => createHash('sha256').update(text).digest('hex').slice(0, 12);
  const parsed = parseClientHello(hello())!;
  assert.equal(parsed.sni, 'localhost');
  assert.deepEqual(parsed.alpn, ['h2']);
  assert.equal(parsed.ja4, `t13d0204h2_${sha('1301,1302')}_${sha('000d,002b_0403,0804')}`);
});

test('an incomplete record is not parsed yet', () => {
  assert.equal(parseClientHello(hello().subarray(0, 20)), null);
});

test('GREASE in signature algorithms, drawn anew for each connection by Chrome, does not change JA4', () => {
  assert.equal(parseClientHello(hello({ sigGrease: 0x2a2a }))!.ja4, parseClientHello(hello({ sigGrease: 0x5a5a }))!.ja4);
});

test('a resumed handshake adds pre_shared_key to JA4 but keeps the base fingerprint of the client', () => {
  const fresh = parseClientHello(hello())!, resumed = parseClientHello(hello({ psk: true }))!;
  assert.equal(resumed.resumed, true); assert.equal(fresh.resumed, false);
  assert.notEqual(resumed.ja4, fresh.ja4);
  assert.equal(resumed.ja4Base, fresh.ja4);
});
