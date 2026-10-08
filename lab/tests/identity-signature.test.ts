import { test } from 'node:test';
import assert from 'node:assert/strict';
import { verify, jwkToKeyID } from 'web-bot-auth';
import { verifierFromJWK } from 'web-bot-auth/crypto';
import { signatureBase, signBase, KeyStore, signRequest, thumbprint } from '../src/identity/signature.ts';

// RFC 9421 Appendix B.1.4 test key, and the request of Appendix B.2.6.
const rfcKey = { d: 'n4Ni-HpISpVObnQMW0wOhCKROaIKqKtW_2ZYb2p9KcU', x: 'JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs' };
const rfcRequest = { method: 'POST', url: 'https://example.com/foo?param=Value&Pet=dog', headers: { date: 'Tue, 20 Apr 2021 02:07:55 GMT', 'content-type': 'application/json', 'content-length': '18' } };

test('the signer reproduces the Ed25519 signature of RFC 9421 Appendix B.2.6', () => {
  const base = signatureBase(rfcRequest, ['"date"', '"@method"', '"@path"', '"@authority"', '"content-type"', '"content-length"'], ';created=1618884473;keyid="test-key-ed25519"');
  assert.equal(base, ['"date": Tue, 20 Apr 2021 02:07:55 GMT', '"@method": POST', '"@path": /foo', '"@authority": example.com', '"content-type": application/json', '"content-length": 18',
    '"@signature-params": ("date" "@method" "@path" "@authority" "content-type" "content-length");created=1618884473;keyid="test-key-ed25519"'].join('\n'));
  assert.equal(signBase(base, KeyStore.fromJwk(rfcKey).keys[0]!.privateKey), 'wqcAqbmYJ2ji2glfAMaRy4gruYYnx2nEFN2HN6jrnDnQCK1u02Gb04v9EDgwUPiu4A0w6vuQv5lIp5WPpBKRCw==');
});

test('key ids are RFC 7638 thumbprints: the RFC 8037 Appendix A.3 vector, and the value of the verifier library', async () => {
  assert.equal(thumbprint({ crv: 'Ed25519', kty: 'OKP', x: '11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo' }), 'kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k');
  const library = await jwkToKeyID({ kty: 'OKP', crv: 'Ed25519', x: rfcKey.x }, data => crypto.subtle.digest('SHA-256', data), bytes => Buffer.from(bytes).toString('base64url'));
  assert.equal(thumbprint({ crv: 'Ed25519', kty: 'OKP', x: rfcKey.x }), library);
});

test('a signed request is accepted by the Cloudflare web-bot-auth verifier, bound to its authority and to the agent directory', async () => {
  const store = new KeyStore(); const key = store.active()!;
  const headers = signRequest({ method: 'GET', url: 'https://localhost:4433/catalog?p=0', headers: {} }, store, 'http://127.0.0.1:5555');
  const verifier = await verifierFromJWK(key.jwk as JsonWebKey);
  const request = (url: string) => new Request(url, { headers: { 'signature-agent': headers['signature-agent'], 'signature-input': headers['signature-input'], signature: headers.signature } });
  const verified = await verify(request('https://localhost:4433/catalog?p=0'), { resolver: candidate => { assert.equal(candidate.keyid, key.kid); return verifier; } });
  assert.equal(verified.signatureAgent?.uri, 'http://127.0.0.1:5555');
  assert.ok(verified.expires.getTime() - verified.created.getTime() <= 60_000);
  await assert.rejects(verify(request('https://other.test:4433/catalog?p=0'), { resolver: () => verifier }), 'a signature replayed on another authority is refused');
});

test('rotation keeps the retired key published; revoking the active key falls back to a retired one, and with none left signing stops', () => {
  const store = new KeyStore(); const first = store.active()!.kid; const second = store.rotate();
  assert.deepEqual(store.published().map(k => [k.kid, k.state]), [[first, 'retired'], [second, 'active']]);
  store.revoke(second);
  assert.equal(store.active()?.kid, first);
  assert.deepEqual(store.published().map(k => k.kid), [first]);
  store.revoke(first);
  assert.throws(() => signRequest({ method: 'GET', url: 'https://localhost/', headers: {} }, store, 'http://127.0.0.1:1'), /no active signing key/);
});
