import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { startFixtures } from '../fixtures/server.ts';
import { executeWorkflow } from '../src/workflow.ts';
import type {WorkflowJob} from '../src/workflow.ts';
import {validateWorkflowCampaign,type WorkflowCampaign} from '../src/workflow-campaign.ts';

test('the integrated runner rejects excessive retained data and a batch matrix with no bounded configuration',()=>{
  const c:WorkflowCampaign={name:'bounded',backends:['node-http'],blocks:1,jobs:24,pages:8,concurrency:[4],batchJobs:[4],maxBatchWaitMs:10,leaseMs:30000,timeoutMs:15000,delayMs:2,seed:42,faults:['none']};
  assert.throws(()=>validateWorkflowCampaign({...c,jobs:1000,pages:1000}),/budget/);assert.throws(()=>validateWorkflowCampaign({...c,concurrency:[1],batchJobs:[4]}),/bounded/);
});

test('HTTP acquisition is validated, committed and acknowledged before a workflow reports completion',async()=>{
  const fixtures=await startFixtures();
  try{const fixture=await fixtures.createRun({scenario:'http-catalog',seed:42,pages:1});
    const task={scenario:'http-catalog',seed:42,pages:1,actions:1,timeoutMs:10000,variant:'normal',url:fixture.url};
    const result=await executeWorkflow({path:join(await mkdtemp(join(tmpdir(),'goodfetch-pipeline-')),'state.sqlite'),backend:'node-http',concurrency:1,batchJobs:1,maxBatchWaitMs:10,leaseMs:30000,timeoutMs:10000},[{jobId:'job-0',task}],async(_id,records)=>fixtures.verify(fixture.id,{records}));
    assert.equal(result.snapshot.jobs[0]!.state,'acked');assert.deepEqual(result.snapshot.jobs[0]!.records,[{id:'P-001',price:1201},{id:'P-002',price:1238}]);assert.equal(result.verifiedJobs,1);
  }finally{await fixtures.close()}
});

async function corpus(fixtures:Awaited<ReturnType<typeof startFixtures>>,count:number){
  const ids=new Map<string,string>();const jobs:WorkflowJob[]=[];
  for(let i=0;i<count;i++){const fixture=await fixtures.createRun({scenario:'http-catalog',seed:42,pages:2,delayMs:2});const jobId=`job-${i}`;ids.set(jobId,fixture.id);jobs.push({jobId,task:{scenario:'http-catalog',seed:42,pages:2,actions:1,timeoutMs:15000,variant:'normal',url:fixture.url}})}
  return {jobs,validate:async(id:string,records:{id:string;price:number}[])=>fixtures.verify(ids.get(id)!,{records})};
}
test('concurrent producers batch results within the queue bound and rerunning acknowledged jobs is idempotent',async()=>{
  const fixtures=await startFixtures();try{const {jobs,validate}=await corpus(fixtures,8);const config={path:join(await mkdtemp(join(tmpdir(),'goodfetch-bounded-')),'state.sqlite'),backend:'node-http' as const,concurrency:4,batchJobs:4,maxBatchWaitMs:50,leaseMs:30000,timeoutMs:15000};
    const result=await executeWorkflow(config,jobs,validate);assert.equal(result.verifiedJobs,8);assert.equal(result.verifiedRecords,32);assert.ok(result.peakBufferedJobs<=4);assert.ok(result.events.some(e=>e.type==='batch-acked'&&Number(e.jobs)>1));
    const again=await executeWorkflow(config,jobs,validate);assert.equal(again.verifiedRecords,32);assert.equal(again.attempts.length,0);assert.ok(again.snapshot.jobs.every(j=>j.attempts===1));
  }finally{await fixtures.close()}
});
for(const fault of ['before-commit','after-commit'] as const)test(`integrated ${fault} recovery acknowledges each job once with exact persisted records`,async()=>{
  const fixtures=await startFixtures();try{const {jobs,validate}=await corpus(fixtures,6);
    const result=await executeWorkflow({path:join(await mkdtemp(join(tmpdir(),'goodfetch-recovery-')),'state.sqlite'),backend:'node-http',concurrency:4,batchJobs:2,maxBatchWaitMs:20,leaseMs:10000,timeoutMs:15000,fault,restartLimit:1},jobs,validate);
    assert.equal(result.verifiedJobs,6);assert.equal(result.verifiedRecords,24);assert.equal(result.restarts,1);assert.ok(result.peakBufferedJobs<=4);assert.ok(result.snapshot.jobs.every(j=>j.state==='acked'&&j.records.length===4));assert.equal(new Set(result.snapshot.jobs.map(j=>j.receipt)).size,6);
    if(fault==='after-commit')assert.equal(result.reconciledJobs,2);else assert.ok(result.snapshot.jobs.some(j=>j.attempts===2));
  }finally{await fixtures.close()}
});
test('an acquisition reporting success with wrong values is rejected before any data commit',async()=>{
  const fixtures=await startFixtures();try{const {jobs,validate}=await corpus(fixtures,1);
    const source="const {createInterface}=require('node:readline');createInterface({input:process.stdin}).on('line',line=>{const m=JSON.parse(line);const result=m.method==='run'?{status:'ok',records:[{id:'P-001',price:1}]}:{ready:true};process.stdout.write(JSON.stringify({version:1,id:m.id,result})+'\\n');if(m.method==='close')process.exit(0)})";
    const result=await executeWorkflow({path:join(await mkdtemp(join(tmpdir(),'goodfetch-negative-')),'state.sqlite'),backend:'node-http',concurrency:1,batchJobs:1,maxBatchWaitMs:10,leaseMs:30000,timeoutMs:15000},jobs,validate,{command:{command:process.execPath,args:['-e',source],timeoutMs:15000}});
    assert.equal(result.verifiedJobs,0);assert.equal(result.snapshot.jobs[0]!.state,'failed');assert.deepEqual(result.snapshot.jobs[0]!.records,[]);assert.ok(result.snapshot.jobs[0]!.error?.includes('wrong price'));
  }finally{await fixtures.close()}
});
