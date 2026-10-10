import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runCampaign, validateCampaign, type Campaign } from '../src/campaign.ts';

const config: Campaign = { name: 'contract', backends: ['node-http'], scenarios: ['http-catalog'], seed: 42, blocks: 1, iterations: 2, warmup: 0, concurrency: 1, mode: 'closed', timeoutMs: 1000, maxDurationMs: 1000, pages: 1, actions: 1 };
test('campaign rejects an empty mutation corpus before starting workers', () => {
  assert.throws(() => validateCampaign({ ...config, variants: [] }), /variant/);
});

test('campaign rejects a retained fixture corpus that exceeds the coordinator budget', () => {
  assert.throws(() => validateCampaign({ ...config, iterations: 100000, pages: 5000 }), /corpus budget/);
});

test('a worker reporting success with incomplete data fails the private oracle and gets no performance trials', async () => {
  const outputRoot = await mkdtemp(join(tmpdir(), 'automan-lab-contract-'));
  const script = `const lines=require('node:readline').createInterface({input:process.stdin});lines.on('line',line=>{const m=JSON.parse(line);const result=m.method==='hello'?{capabilities:['http-catalog']} : m.method==='run'?{status:'ok',records:[]} : {};process.stdout.write(JSON.stringify({version:1,id:m.id,result})+'\\n');if(m.method==='close'){lines.close();process.stdin.destroy()}})`;
  const result = await runCampaign(config, { outputRoot, skipEnvironment: true, command: () => ({ command: process.execPath, args: ['-e', script], timeoutMs: 1000 }) });
  assert.equal(result.rows.length, 1); assert.equal(result.rows[0]!.status, 'failed');
  assert.equal(result.summary.summaries[0]!.measurement.attempts, 0);
  const persisted = (await readFile(join(result.directory, 'attempts.jsonl'), 'utf8')).trim().split('\n').map(line => JSON.parse(line));
  assert.deepEqual(persisted, JSON.parse(JSON.stringify(result.rows)));
});
