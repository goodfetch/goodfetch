import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {WorkerClient} from '../../src/worker-client.ts';
import {labRoot} from '../../src/registry.ts';

const task={scenario:'http-catalog',url:'http://127.0.0.1/catalog',seed:42,pages:1,actions:1,timeoutMs:1000,variant:'normal'};
for(const runtime of ['node-shared','rust'] as const)test(`${runtime}: durable records, idempotent receipts and stale-token fencing use the shared SQLite`,async()=>{
  const path=join(await mkdtemp(join(tmpdir(),'automan-shared-ledger-')),'state.sqlite');
  const w=new WorkerClient(runtime==='rust'?{command:join(labRoot,'.native/automan-workflow-rust'),args:['sink'],cwd:labRoot,timeoutMs:10000}:{command:process.execPath,args:[join(labRoot,'adapters/workflow-node/worker.ts')],env:{...process.env,AUTOMAN_SHARED_SQLITE:'1'},cwd:labRoot,timeoutMs:10000});
  try{
    await w.call('prepare',{path,leaseMs:10000});
    const info=await w.call('hello',{}) as {sqlite:string;sourceId:string;libraryPath:string};
    assert.equal(info.sqlite,'3.53.4');assert.equal(info.libraryPath,join(labRoot,'.native/libautoman_sqlite.dylib'));
    await w.call('run',{op:'submit',jobs:[{jobId:'job-1',task}]});
    const claim=await w.call('run',{op:'claim'}) as {jobId:string;token:string};
    const records=[{id:'P-001',price:1201},{id:'P-002',price:1238}];
    const first=await w.call('run',{op:'commit',entries:[{...claim,records}]}) as {receipts:unknown[]};
    assert.deepEqual(await w.call('run',{op:'commit',entries:[{...claim,records:records.toReversed()}]}),first);
    await assert.rejects(w.call('run',{op:'commit',entries:[{...claim,token:'stale',records}]}),/stale/);
    await assert.rejects(w.call('run',{op:'commit',entries:[{...claim,records:[{id:'P-001',price:7}]}]}),/conflict/);
    await w.call('run',{op:'ack',receipts:first.receipts});await w.call('run',{op:'ack',receipts:first.receipts});
    const snapshot=await w.call('run',{op:'snapshot'}) as {jobs:{state:string;records:unknown[]}[];integrity:string[];foreignKeys:unknown[]};
    assert.deepEqual(snapshot.integrity,['ok']);assert.deepEqual(snapshot.foreignKeys,[]);
    assert.equal(snapshot.jobs[0]!.state,'acked');assert.deepEqual(snapshot.jobs[0]!.records,records);
  }finally{await w.close()}
});
