import {test} from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';
import {labRoot} from '../../src/registry.ts';
import {sampleRuntimeResources} from '../../src/runtime-resources.ts';
test('the macOS CPU probe reports the same nanosecond unit as independent getrusage',()=>{
  const start=performance.now();while(performance.now()-start<150){}
  const before=process.resourceUsage();
  const rows=JSON.parse(execFileSync(join(labRoot,'.native/process-probe'),[String(process.pid)],{encoding:'utf8'})) as {pid:number;userNs:number;systemNs:number;timebaseNumer:number;timebaseDenom:number}[];
  const after=process.resourceUsage();const row=rows.find(r=>r.pid===process.pid)!;
  const lower=(before.userCPUTime+before.systemCPUTime)*1000-20000000;const upper=(after.userCPUTime+after.systemCPUTime)*1000+20000000;
  assert.ok(row.userNs+row.systemNs>=lower&&row.userNs+row.systemNs<=upper,`counter ${row.userNs+row.systemNs} outside getrusage ${lower}–${upper}`);
  assert.ok(row.timebaseNumer>0&&row.timebaseDenom>0);
});
test('attaching to an existing coordinator accounts only for CPU after the initial sample',async()=>{
  const sampler=sampleRuntimeResources(process.pid,100,{existingRoot:true});await sampler.ready();const before=process.resourceUsage();
  const began=performance.now();while(performance.now()-began<150){}
  const resources=await sampler.stop();const after=process.resourceUsage();
  const measured=resources.sampledUserCpuMs+resources.sampledSystemCpuMs;const expected=(after.userCPUTime+after.systemCPUTime-before.userCPUTime-before.systemCPUTime)/1000;
  assert.ok(Math.abs(measured-expected)<50,`sample ${measured} ms vs getrusage interval ${expected} ms`);assert.equal(resources.baselines.length,1);
});
