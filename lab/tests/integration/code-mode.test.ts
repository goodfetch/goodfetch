import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { startFixtures } from '../../fixtures/server.ts';
import { launchBrowser } from '../../src/core/browser-host.ts';
import { browserPath } from '../../src/environment.ts';
import { CdpConnection, CdpPage } from '../../adapters/cdp-node/cdp.ts';
import { waitScript, condition } from '../../adapters/browser-scripts/scripts.ts';
import { runInterface, type Interface } from '../../src/ai/interfaces.ts';

// A scripted model that behaves as GPT-5.6 Luna did in the arena: a first program that books but forgets page.done(),
// then, told so, the whole program again with done at the end.
async function scriptedModel() {
  let calls = 0;
  const server = createServer((req, res) => {
    let body = ''; req.on('data', c => { body += c; }); req.on('end', () => {
      const { messages } = JSON.parse(body) as { messages: { role: string; content: string }[] };
      const { elements } = JSON.parse(messages[1]!.content) as { elements: { ref: number; tag: string; type: string; label: string; text: string }[] };
      const ref = (match: (e: { tag: string; type: string; label: string; text: string }) => boolean) => elements.find(match)!.ref;
      const steps = [`await page.fill(${ref(e => /name/i.test(e.label))}, 'Ada Lovelace');`, `await page.fill(${ref(e => e.type === 'email')}, 'ada@example.test');`, `await page.select(${ref(e => e.tag === 'select')}, 'pro');`,
        `await page.click(${ref(e => e.type === 'checkbox')});`, `await page.click(${ref(e => e.tag === 'button' && /book/i.test(e.text))});`];
      const program = calls++ === 0 ? steps.join('\n') : [...steps, 'await page.done();'].join('\n');
      res.writeHead(200, { 'content-type': 'application/json' }); res.end(JSON.stringify({ choices: [{ message: { content: program } }], usage: { prompt_tokens: 10, completion_tokens: 10 } }));
    });
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  return { url: `http://127.0.0.1:${(server.address() as AddressInfo).port}/v1/chat/completions`, close: () => new Promise<void>(resolve => server.close(() => resolve())) };
}

async function book(kind: Interface) {
  const [fixtures, browser, model] = await Promise.all([startFixtures(), launchBrowser(browserPath()), scriptedModel()]);
  const connection = await CdpConnection.connect(browser.wsEndpoint);
  try {
    const fixture = await fixtures.createRun({ scenario: 'agent', seed: 5, variant: 'baseline' as never }); const page = await CdpPage.open(connection, 10_000);
    await page.goto(fixture.url); await page.evaluate(waitScript(condition('body-ready'), 10_000));
    const result = await runInterface(kind, page, { wsEndpoint: browser.wsEndpoint, targetId: page.targetId }, { kind: 'book', name: 'Ada Lovelace', email: 'ada@example.test', plan: 'pro' }, { url: model.url, model: 'scripted' });
    await page.evaluate(waitScript(condition('done'), 2000)).catch(() => {}); await new Promise(resolve => setTimeout(resolve, 100));
    return { result, verdict: await fixtures.verify(fixture.id, result.finished ? { task: 'book' } : { task: 'book', error: result.error ?? 'unfinished' }) };
  } finally { connection.close(); await Promise.all([browser.close(), fixtures.close(), model.close()]); }
}

test('a repaired code-mode program replays its effects: the consent is unticked and the booking sent twice', async () => {
  const { result, verdict } = await book('code-mode');
  assert.equal(result.finished, true); assert.equal(verdict.status, 'failed', 'the oracle refutes the declared success');
});

test('with the effect journal the repaired program skips what the page already received, and the booking is sent once', async () => {
  const { result, verdict } = await book('code-mode-journal');
  assert.equal(verdict.status, 'verified', verdict.reasons.join('; '));
  assert.equal(result.trace.filter(line => line.startsWith('skip ')).length, 5);
});
