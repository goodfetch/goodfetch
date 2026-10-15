import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,readFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {spawn} from 'node:child_process';
import {startFixtures} from '../fixtures/server.ts';
import {readWorkflow} from '../src/workflow.ts';
import {labRoot} from '../src/registry.ts';

for(const target of ['job-claimed','batch-committed'])test(`a new coordinator process recovers persisted state after a complete ${target} stop`,{skip:process.platform==='win32'},async()=>{
  const fixtures=await startFixtures();const directory=await mkdtemp(join(tmpdir(),'goodfetch-coordinator-'));const path=join(directory,'state.sqlite');const flag=join(directory,'paused.json');const output=join(directory,'completed.json');
  const fixture=await fixtures.createRun({scenario:'http-catalog',seed:42,pages:1});
  const jobs=[{jobId:'job-0',task:{scenario:'http-catalog',seed:42,pages:1,actions:1,timeoutMs:10000,variant:'normal',url:fixture.url}}];
  const config={path,backend:'node-http',concurrency:1,batchJobs:1,maxBatchWaitMs:10,leaseMs:1000,timeoutMs:10000};
  const source=`import {writeFile} from 'node:fs/promises';import {executeWorkflow} from ${JSON.stringify(pathToFileURL(join(labRoot,'src/workflow.ts')).href)};const p=JSON.parse(process.argv[1]);let owned=[];const result=await executeWorkflow(p.config,p.jobs,async(_id,records)=>({status:JSON.stringify(records)==='[{"id":"P-001","price":1201},{"id":"P-002","price":1238}]'?'verified':'failed',reasons:[]}),{onEvent:async row=>{if(row.type==='workers-started')owned=[row.sinkPid,row.acquisitionPid];if(row.type===p.target){await writeFile(p.flag,JSON.stringify({owned,coordinator:process.pid}));await new Promise(()=>{})}}});await writeFile(p.output,JSON.stringify({verifiedJobs:result.verifiedJobs,reconciledJobs:result.reconciledJobs}));`;
  let owned:number[]=[];const first=spawn(process.execPath,['--input-type=module','-e',source,JSON.stringify({config,jobs,flag,output,target})],{detached:true,stdio:'ignore'});const firstExited=new Promise(resolve=>first.once('exit',resolve));
  try{
    const deadline=performance.now()+15000;let paused:{owned:number[];coordinator:number}|undefined;
    while(performance.now()<deadline&&first.exitCode===null&&first.signalCode===null){try{paused=JSON.parse(await readFile(flag,'utf8'));break}catch(error){if((error as NodeJS.ErrnoException).code!=='ENOENT'&&!(error instanceof SyntaxError))throw error/* absent, or read while the child is still writing it */}await new Promise(resolve=>setTimeout(resolve,20))}
    assert.ok(paused,'coordinator did not expose its durable stop point');assert.equal(paused.coordinator,first.pid);owned=paused.owned;assert.equal(owned.length,2);assert.ok(owned.every(pid=>Number.isInteger(pid)&&pid>0));
    const before=readWorkflow(path);assert.equal(before.jobs[0]!.state,target==='job-claimed'?'leased':'committed');assert.equal(before.jobs[0]!.records.length,target==='job-claimed'?0:2);
    for(const pid of [first.pid!,...owned]){try{process.kill(-pid,'SIGKILL')}catch{}}
    await firstExited;
    const resumed=spawn(process.execPath,['--input-type=module','-e',source,JSON.stringify({config,jobs,flag,output,target:'never'})],{detached:true,stdio:'ignore'});
    const code=await new Promise<number|null>((resolve,reject)=>{const timer=setTimeout(()=>{try{process.kill(-resumed.pid!,'SIGKILL')}catch{}reject(new Error('coordinator resume deadline'))},15000);resumed.once('error',reject);resumed.once('exit',code=>{clearTimeout(timer);resolve(code)})});assert.equal(code,0);
    const completed=JSON.parse(await readFile(output,'utf8'));assert.equal(completed.verifiedJobs,1);assert.equal(completed.reconciledJobs,target==='batch-committed'?1:0);
    const after=readWorkflow(path);assert.equal(after.jobs[0]!.state,'acked');assert.equal((await fixtures.verify(fixture.id,{records:after.jobs[0]!.records})).status,'verified');assert.equal(after.jobs[0]!.attempts,target==='job-claimed'?2:1);
  }finally{for(const pid of [first.pid!,...owned]){try{process.kill(-pid,'SIGKILL')}catch{}}await fixtures.close()}
});
