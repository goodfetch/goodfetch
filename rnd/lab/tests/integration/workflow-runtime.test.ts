import {test} from 'node:test';
import assert from 'node:assert/strict';
import {runRuntime} from '../../src/workflow-runtime.ts';

for(const runtime of ['node','rust'] as const){
  test(`${runtime}: complete native workflow validates every acquired record, stays bounded and resumes idempotently`,async()=>{
    const run=await runRuntime({runtime,jobs:12,pages:2,delayMs:2,concurrency:4,batchJobs:4,seed:42});
    assert.equal(run.verifiedJobs,12);assert.equal(run.verifiedRecords,48);assert.ok(run.result.peakBufferedJobs<=4);
    assert.ok(run.events.some(e=>e.type==='batch-acked'&&Number(e.jobs)>1));
    assert.ok(run.snapshot.jobs.every(j=>j.state==='acked'&&j.attempts===1));
    const again=await run.resume();assert.equal(again.verifiedJobs,12);assert.equal(again.result.attempts.length,0);
    assert.deepEqual(again.snapshot.jobs.map(j=>j.receipt),run.snapshot.jobs.map(j=>j.receipt));
    await run.close();
  });
  test(`${runtime}: independently detected false success never reaches storage`,async()=>{
    const run=await runRuntime({runtime,jobs:2,pages:1,delayMs:0,concurrency:2,batchJobs:2,seed:42,corruptOutput:true});
    try{assert.equal(run.verifiedJobs,0);assert.ok(run.snapshot.jobs.every(j=>j.state==='failed'&&j.records.length===0))}finally{await run.close()}
  });
}
for(const runtime of ['node','rust'] as const)for(const fault of ['before-commit','after-commit','during-ack','after-ack'] as const){
  test(`${runtime}: ${fault} recovers exact results after a lost transactional response`,async()=>{
    const run=await runRuntime({runtime,jobs:12,pages:2,delayMs:2,concurrency:4,batchJobs:2,seed:42,fault,leaseMs:1000});
    try{
      assert.equal(run.verifiedJobs,12);assert.equal(run.verifiedRecords,48);assert.equal(run.result.restarts,1);
      assert.ok(run.snapshot.jobs.every(j=>j.state==='acked'));assert.equal(new Set(run.snapshot.jobs.map(j=>j.receipt)).size,12);
      assert.ok(run.result.peakBufferedJobs<=4);assert.equal(new Set(run.events.filter(e=>e.type==='job-acked'||e.type==='reconciled').map(e=>e.jobId)).size,12);
    }finally{await run.close()}
  });
}
for(const runtime of ['node','rust'] as const)test(`${runtime}: repeated writer deaths preserve bounded backpressure and exact committed data`,async()=>{
  const run=await runRuntime({runtime,jobs:16,pages:2,delayMs:2,concurrency:4,batchJobs:2,seed:42,faults:['before-commit','during-ack','after-ack'],leaseMs:1000,writerDelayMs:20});
  try{assert.equal(run.verifiedJobs,16);assert.equal(run.result.restarts,3);assert.ok(run.result.peakBufferedJobs<=4);assert.equal(new Set(run.snapshot.jobs.map(j=>j.receipt)).size,16)}finally{await run.close()}
});
for(const runtime of ['node','rust'] as const)for(const coordinatorCrashAt of ['job-claimed','batch-committed'] as const)test(`${runtime}: a new coordinator resumes after ${coordinatorCrashAt}`,async()=>{
  const run=await runRuntime({runtime,jobs:12,pages:2,delayMs:2,concurrency:4,batchJobs:2,seed:42,leaseMs:1000,coordinatorCrashAt});
  try{assert.equal(run.coordinatorRestarts,1);assert.equal(run.verifiedJobs,12);assert.equal(run.verifiedRecords,48);assert.equal(new Set(run.snapshot.jobs.map(j=>j.receipt)).size,12);assert.ok(run.coordinatorRecoveryMs!==null&&run.coordinatorRecoveryMs!==undefined);assert.ok(run.snapshot.jobs.every(j=>j.state==='acked'))}finally{await run.close()}
});
