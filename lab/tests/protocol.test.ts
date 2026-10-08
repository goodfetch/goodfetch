import { test } from 'node:test';
import assert from 'node:assert/strict';
import { WorkerClient } from '../src/worker-client.ts';

test('hung adapter is terminated within a bounded deadline and remaining calls reject', async () => {
  const worker = new WorkerClient({ command: process.execPath, args: ['-e', 'process.stdin.resume()'], timeoutMs: 100 });
  const began = performance.now();
  await assert.rejects(worker.call('hello', {}), /timeout/);
  assert.ok(performance.now() - began < 2000);
  await assert.rejects(worker.call('hello', {}), /closed/);
  await worker.close();
});

test('out-of-order JSONL responses are correlated to their requests', async () => {
  const script = `const q=[];require('node:readline').createInterface({input:process.stdin}).on('line',line=>{const m=JSON.parse(line);if(m.method==='close'){process.stdout.write(JSON.stringify({version:1,id:m.id,result:{}})+'\\n');return}q.push(m);if(q.length===2)for(const r of q.reverse())process.stdout.write(JSON.stringify({version:1,id:r.id,result:r.params})+'\\n')})`;
  const worker = new WorkerClient({ command: process.execPath, args: ['-e', script], timeoutMs: 1000 });
  try { assert.deepEqual(await Promise.all([worker.call('run', { label: 'first' }), worker.call('run', { label: 'second' })]), [{ label: 'first' }, { label: 'second' }]); }
  finally { await worker.close(); }
});

test('a corrupt adapter response aborts the pending task instead of accepting a false success', async () => {
  const worker = new WorkerClient({ command: process.execPath, args: ['-e', `process.stdin.on('data',()=>process.stdout.write('not-json\\n'))`], timeoutMs: 1000 });
  try { await assert.rejects(worker.call('run', {}), /invalid JSONL/); }
  finally { await worker.close(); }
});

test('deadline cleanup also terminates an owned descendant that ignores SIGTERM', { skip: process.platform === 'win32' }, async () => {
  const script = `const c=require('node:child_process').spawn(process.execPath,['-e',"process.on('SIGTERM',()=>{});setInterval(()=>{},1000)"],{stdio:'ignore'});process.on('SIGTERM',()=>{});require('node:readline').createInterface({input:process.stdin}).on('line',line=>{const m=JSON.parse(line);if(m.method==='hello')process.stdout.write(JSON.stringify({version:1,id:m.id,result:{pid:c.pid}})+'\\n')})`;
  const worker = new WorkerClient({ command: process.execPath, args: ['-e', script], timeoutMs: 150 });
  // Only run is meant to hit the 150 ms deadline; process start under a loaded test run can take longer than that.
  const info = await worker.call('hello', {}, 5000) as { pid: number };
  try { await assert.rejects(worker.call('run', {}), /timeout/); }
  finally { await worker.close(); }
  let alive = true; const deadline = performance.now() + 1500;
  while (alive && performance.now() < deadline) { try { process.kill(info.pid, 0); await new Promise(resolve => setTimeout(resolve, 20)); } catch { alive = false; } }
  assert.equal(alive, false, 'owned child survived deadline cleanup');
});
