// Web Bot Auth for the identified profile: HTTP Message Signatures (RFC 9421) with Ed25519 keys, a key directory served by
// the agent, rotation and revocation. Written from the RFC and the draft, independently of the verifier used by the fixtures.
import { createHash, createPrivateKey, generateKeyPairSync, randomBytes, sign, type KeyObject } from 'node:crypto';
import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';

export const directoryPath = '/.well-known/http-message-signatures-directory';
const b64url = (bytes: Buffer) => bytes.toString('base64url');

// RFC 7638 thumbprint of an Ed25519 public JWK: its required members in lexicographic order, SHA-256, base64url.
export const thumbprint = (jwk: { crv: string; kty: string; x: string }) => b64url(createHash('sha256').update(JSON.stringify({ crv: jwk.crv, kty: jwk.kty, x: jwk.x })).digest());

export type Message = { method: string; url: string; headers: Record<string, string> };
// The value of one covered component (RFC 9421 §2): derived components, plain fields, or a dictionary member by key.
function componentValue(message: Message, id: string): string {
  const [, name, key] = id.match(/^"([^"]+)"(?:;key="([^"]+)")?(?:;req)?$/) ?? [];
  if (!name) throw new Error(`unsupported component ${id}`);
  const url = new URL(message.url);
  if (name === '@method') return message.method.toUpperCase();
  if (name === '@path') return url.pathname;
  if (name === '@authority') return url.port ? `${url.hostname.toLowerCase()}:${url.port}` : url.hostname.toLowerCase();
  if (name === '@target-uri') return url.href;
  const field = Object.entries(message.headers).find(([header]) => header.toLowerCase() === name)?.[1];
  if (field === undefined) throw new Error(`missing field ${name}`);
  if (!key) return field.trim();
  const member = field.split(',').map(part => part.trim()).find(part => part.startsWith(`${key}=`));
  if (!member) throw new Error(`missing member ${key} of ${name}`);
  return member.slice(key.length + 1);
}
export function signatureBase(message: Message, components: string[], params: string) {
  return [...components.map(id => `${id}: ${componentValue(message, id)}`), `"@signature-params": (${components.join(' ')})${params}`].join('\n');
}
export const signBase = (base: string, key: KeyObject) => sign(null, Buffer.from(base), key).toString('base64');

type Key = { kid: string; privateKey: KeyObject; jwk: { kty: 'OKP'; crv: 'Ed25519'; x: string; kid: string; use: 'sig' }; state: 'active' | 'retired' | 'revoked' };

// The keys of one agent installation: one active signing key, retired keys still published for a grace period, revoked keys withdrawn.
export class KeyStore {
  readonly keys: Key[] = [];
  constructor() { this.rotate(); }
  static fromJwk(jwk: { d: string; x: string }) {
    const store = Object.create(KeyStore.prototype) as KeyStore; (store as { keys: Key[] }).keys = [];
    store.add(createPrivateKey({ key: { kty: 'OKP', crv: 'Ed25519', d: jwk.d, x: jwk.x }, format: 'jwk' })); return store;
  }
  private add(privateKey: KeyObject) {
    const { x } = privateKey.export({ format: 'jwk' }) as { x: string }; const kid = thumbprint({ crv: 'Ed25519', kty: 'OKP', x });
    for (const key of this.keys) if (key.state === 'active') key.state = 'retired';
    this.keys.push({ kid, privateKey, jwk: { kty: 'OKP', crv: 'Ed25519', x, kid, use: 'sig' }, state: 'active' }); return kid;
  }
  rotate() { return this.add(generateKeyPairSync('ed25519').privateKey); }
  // A revoked key leaves the directory at once; if it was the active key, the newest retired key signs until the next rotation.
  revoke(kid: string) {
    const key = this.keys.find(k => k.kid === kid); if (!key) throw new Error('unknown key');
    const wasActive = key.state === 'active'; key.state = 'revoked';
    if (wasActive) { const fallback = this.keys.filter(k => k.state === 'retired').at(-1); if (fallback) fallback.state = 'active'; }
  }
  active() { return this.keys.find(key => key.state === 'active') ?? null; }
  published() { return this.keys.filter(key => key.state !== 'revoked'); }
}

export class SigningError extends Error { readonly code = 'E_NO_SIGNING_KEY'; }

// Headers that sign one request: the request is bound to its authority and to the agent's directory, with a short validity and a fresh nonce.
export function signRequest(message: Message, store: KeyStore, agentOrigin: string, { label = 'sig1', ttlSeconds = 60, now = Date.now() } = {}) {
  const key = store.active(); if (!key) throw new SigningError('no active signing key');
  const signatureAgent = `${label}="${agentOrigin}"`;
  const created = Math.floor(now / 1000);
  const params = `;created=${created};expires=${created + ttlSeconds};keyid="${key.kid}";alg="ed25519";nonce="${randomBytes(64).toString('base64')}";tag="web-bot-auth"`;
  const components = ['"@authority"', `"signature-agent";key="${label}"`];
  const signed = { ...message, headers: { ...message.headers, 'signature-agent': signatureAgent } };
  return { 'signature-agent': signatureAgent, 'signature-input': `${label}=(${components.join(' ')})${params}`, signature: `${label}=:${signBase(signatureBase(signed, components, params), key.privateKey)}:`, keyid: key.kid };
}

// The agent's key directory: a JWK set of the published keys, the response signed by each of them over the requested authority.
export async function serveDirectory(store: KeyStore): Promise<{ origin: string; fetches: number; close(): Promise<void> }> {
  const state = { fetches: 0 };
  const server: Server = createServer((req, res) => {
    if (req.url !== directoryPath) { res.writeHead(404).end(); return; }
    state.fetches++;
    const authority = req.headers.host ?? ''; const created = Math.floor(Date.now() / 1000);
    const signatures = store.published().map((key, index) => {
      const label = `sig${index + 1}`; const params = `;created=${created};expires=${created + 300};keyid="${key.kid}";alg="ed25519";tag="http-message-signatures-directory"`;
      const base = signatureBase({ method: 'GET', url: `http://${authority}${directoryPath}`, headers: {} }, ['"@authority";req'], params);
      return { input: `${label}=("@authority";req)${params}`, signature: `${label}=:${signBase(base, key.privateKey)}:` };
    });
    res.writeHead(200, { 'content-type': 'application/http-message-signatures-directory+json', 'cache-control': 'max-age=0', 'signature-input': signatures.map(s => s.input).join(', '), signature: signatures.map(s => s.signature).join(', ') });
    res.end(JSON.stringify({ keys: store.published().map(key => key.jwk) }));
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  return { origin: `http://127.0.0.1:${(server.address() as AddressInfo).port}`, get fetches() { return state.fetches; }, close: () => new Promise(resolve => { server.closeAllConnections(); server.close(() => resolve()); }) };
}
