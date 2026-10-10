import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { WorkerClient } from '../src/worker-client.ts';
import { labRoot } from '../src/registry.ts';

const task = { scenario:'http-catalog',url:'http://127.0.0.1/catalog',seed:42,pages:1,actions:1,timeoutMs:1000,variant:'normal' };
function worker(){return new WorkerClient({command:process.execPath,args:[join(labRoot,'adapters/workflow-node/worker.ts')],cwd:labRoot,timeoutMs:10000})}
type Claim={jobId:string;token:string};
test('committing a validated job makes its exact records retrievable and replay keeps one receipt', async()=>{
  const path=join(await mkdtemp(join(tmpdir(),'automan-workflow-')),'state.sqlite');const w=worker();
  try{await w.call('prepare',{path,leaseMs:10000});await w.call('run',{op:'submit',jobs:[{jobId:'job-1',task}]});
    const claim=await w.call('run',{op:'claim'}) as Claim;
    const entry={...claim,records:[{id:'P-001',price:1201},{id:'P-002',price:1238}]};
    const first=await w.call('run',{op:'commit',entries:[entry]});const replay=await w.call('run',{op:'commit',entries:[entry]});assert.deepEqual(replay,first);
    const snapshot=await w.call('run',{op:'snapshot'}) as {jobs:{state:string;records:{id:string;price:number}[]}[]};
    assert.deepEqual(snapshot.jobs[0]!.records,[{id:'P-001',price:1201},{id:'P-002',price:1238}]);assert.equal(snapshot.jobs[0]!.state,'committed');
    await assert.rejects(w.call('run',{op:'commit',entries:[{...entry,records:[{id:'P-001',price:1}]}]}),/different|conflict/);
  }finally{await w.close()}
});

for(const fault of ['before-commit','after-commit'] as const)test(`${fault} crash preserves atomic state and a lost response can be reconciled`,async()=>{
  const path=join(await mkdtemp(join(tmpdir(),'automan-workflow-crash-')),'state.sqlite');const first=worker();let claim:Claim;
  const entryRecords=[{id:'P-001',price:1201},{id:'P-002',price:1238}];
  try{await first.call('prepare',{path,leaseMs:10000});await first.call('run',{op:'submit',jobs:[{jobId:'job-1',task}]});claim=await first.call('run',{op:'claim'}) as Claim;
    await assert.rejects(first.call('run',{op:'commit',entries:[{...claim,records:entryRecords}],fault}),/exited|closed/);
  }finally{await first.close()}
  const resumed=worker();try{await resumed.call('prepare',{path,leaseMs:10000});const snapshot=await resumed.call('run',{op:'snapshot'}) as {jobs:{state:string;receipt:string|null;records:unknown[]}[];integrity:string[]};
    assert.deepEqual(snapshot.integrity,['ok']);assert.equal(snapshot.jobs[0]!.state,fault==='before-commit'?'leased':'committed');assert.equal(snapshot.jobs[0]!.records.length,fault==='before-commit'?0:2);
    if(fault==='after-commit'){const receipts=[{jobId:'job-1',receipt:snapshot.jobs[0]!.receipt!}];await resumed.call('run',{op:'ack',receipts});await resumed.call('run',{op:'ack',receipts});const done=await resumed.call('run',{op:'snapshot'}) as {jobs:{state:string;records:unknown[]}[]};assert.equal(done.jobs[0]!.state,'acked');assert.deepEqual(done.jobs[0]!.records,entryRecords)}
  }finally{await resumed.close()}
});

test('an expired lease cannot commit after another claimant receives a new token',async()=>{
  const path=join(await mkdtemp(join(tmpdir(),'automan-fencing-')),'state.sqlite');const w=worker();try{
    await w.call('prepare',{path,leaseMs:1000});await w.call('run',{op:'submit',jobs:[{jobId:'job-1',task}]});const stale=await w.call('run',{op:'claim'}) as Claim&{expires:number};
    while(Date.now()<=stale.expires)await new Promise(resolve=>setTimeout(resolve,20));
    const current=await w.call('run',{op:'claim'}) as Claim;assert.notEqual(current.token,stale.token);const records=[{id:'P-001',price:1201}];
    await assert.rejects(w.call('run',{op:'commit',entries:[{...stale,records}]}),/stale lease/);await w.call('run',{op:'commit',entries:[{...current,records}]});
  }finally{await w.close()}
});
