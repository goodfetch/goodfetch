import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseRobots, groupsFor, pathAllowed, SitePolicy, classifyResponse, StopError } from '../src/identity/policy.ts';

const robots = `# comment
User-agent: AutomanLab
User-agent: OtherBot
Disallow: /
Allow: /public/

User-agent: *
Disallow: /example/page/disallowed.gif
Allow: /example/page/
Disallow: /*.pdf$
Allow: /tie
Disallow: /tie
Crawl-delay: 2
Content-Signal: search=yes, ai-train=no
`;

test('robots.txt: a named group replaces the * group, longest match wins, allow wins a tie, * and $ are patterns (RFC 9309)', () => {
  const groups = parseRobots(robots);
  const named = groupsFor(groups, 'automanlab'), any = groupsFor(groups, null);
  assert.equal(pathAllowed(named.rules, '/catalog'), false);
  assert.equal(pathAllowed(named.rules, '/public/a'), true);
  assert.equal(pathAllowed(any.rules, '/example/page/disallowed.gif'), false);
  assert.equal(pathAllowed(any.rules, '/example/page/other'), true);
  assert.equal(pathAllowed(any.rules, '/doc.pdf'), false);
  assert.equal(pathAllowed(any.rules, '/doc.pdf?download=1'), true);
  assert.equal(pathAllowed(any.rules, '/tie'), true);
  assert.equal(any.crawlDelay, 2); assert.equal(named.crawlDelay, null);
});

const site = (files: Record<string, { status: number; body: string }>) => async (url: string) => files[new URL(url).pathname] ?? { status: 404, body: '' };

test('an unreachable robots.txt disallows everything, a missing one allows everything', async () => {
  const down = await SitePolicy.load('https://a.test', site({ '/robots.txt': { status: 503, body: '' } }), 'AutomanLab');
  assert.throws(() => down.check('https://a.test/x', 'search'), (e: StopError) => e.code === 'E_POLICY_ROBOTS');
  const missing = await SitePolicy.load('https://a.test', site({}), 'AutomanLab');
  assert.doesNotThrow(() => missing.check('https://a.test/x', 'ai-train'));
});

test('Content-Signal and TDMRep refuse only the purposes they name', async () => {
  const policy = await SitePolicy.load('https://a.test', site({ '/robots.txt': { status: 200, body: robots }, '/.well-known/tdmrep.json': { status: 200, body: JSON.stringify([{ location: '/reserved/*', 'tdm-reservation': 1, 'tdm-policy': 'https://a.test/policy.json' }]) } }), null);
  assert.throws(() => policy.check('https://a.test/x', 'ai-train'), (e: StopError) => e.code === 'E_POLICY_CONTENT_SIGNAL');
  assert.doesNotThrow(() => policy.check('https://a.test/x', 'search'));
  assert.throws(() => policy.check('https://a.test/reserved/a', 'data-collection'), (e: StopError) => e.code === 'E_POLICY_TDM');
  assert.doesNotThrow(() => policy.check('https://a.test/reserved/a', 'agent'));
  assert.equal(policy.crawlDelayMs, 2000);
  assert.match(policy.evidence.robotsSha256!, /^[0-9a-f]{64}$/);
});

test('page-level reservations and noai discard the page for the purposes they cover', async () => {
  const policy = await SitePolicy.load('https://a.test', site({}), null);
  assert.throws(() => policy.checkResponse('ai-input', new Headers({ 'tdm-reservation': '1' }), ''), (e: StopError) => e.code === 'E_POLICY_TDM');
  assert.throws(() => policy.checkResponse('ai-train', new Headers(), '<meta name="robots" content="noindex, noai">'), (e: StopError) => e.code === 'E_POLICY_NOAI');
  assert.doesNotThrow(() => policy.checkResponse('search', new Headers({ 'tdm-reservation': '1' }), '<meta name="robots" content="noai">'));
});

test('refusals, payment, challenges served in 200 and rate limits are stop signals, never content', () => {
  assert.equal(classifyResponse(200, new Headers(), '<html><head><title>Just a moment...</title></head></html>')?.code, 'E_CHALLENGE');
  assert.equal(classifyResponse(403, new Headers(), 'denied')?.code, 'E_ACCESS_DENIED');
  assert.equal(classifyResponse(402, new Headers(), '')?.code, 'E_PAYMENT_REQUIRED');
  assert.deepEqual(classifyResponse(429, new Headers({ 'retry-after': '1' }), ''), { code: 'E_RATE_LIMITED', retryAfterMs: 1000 });
  assert.equal(classifyResponse(200, new Headers(), '<main>catalog</main>'), null);
});
