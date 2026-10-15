import { sqliteLibrary } from './registry.ts';
import {createServer} from 'node:http';
import type {AddressInfo} from 'node:net';
import {randomBytes} from 'node:crypto';
import {mkdtemp,mkdir,writeFile,readFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawn} from 'node:child_process';
import {startFixtures} from '../fixtures/server.ts';
import type {RunSpec} from '../fixtures/server.ts';
import {readWorkflow,type WorkflowJob,type WorkflowConfig} from './workflow.ts';
import {labRoot,type BackendName} from './registry.ts';
import {hashFile} from './environment.ts';
import {sampleRuntimeResources} from './runtime-resources.ts';
export type RuntimeConfig={runtime:'node'|'rust';backend?:BackendName;poolWake?:'poll'|'notify';variant?:RunSpec['variant'];jobs:number;pages:number;delayMs:number;concurrency:number;batchJobs:number;seed:number;fault?:WorkflowConfig['fault'];faults?:WorkflowConfig['fault'][];leaseMs?:number;writerDelayMs?:number;corruptOutput?:boolean;coordinatorCrashAt?:'job-claimed'|'batch-committed';directory?:string};
export type RuntimeResult={peakBufferedJobs:number;attempts:Record<string,unknown>[];verifiedJobs:number;verifiedRecords:number;restarts:number;info:Record<string,unknown>[];workMs:number;startupMs:number;finalizationMs:number;totalMs:number;[key:string]:unknown};
type RuntimeResources=Awaited<ReturnType<ReturnType<typeof sampleRuntimeResources>['stop']>>;
type RuntimeExecution={options:RuntimeConfig;result:RuntimeResult;verifiedJobs:number;verifiedRecords:number;resources:RuntimeResources;runnerWallMs:number;finalOracleMs:number;totalVerifiedWallMs:number;jobsPerSecondIncludingStartupAndOracle:number;sqliteSha256:string;verdicts:{jobId:string;status:string;reasons:string[]}[];events:Record<string,unknown>[];snapshot:ReturnType<typeof readWorkflow>;coordinatorRestarts?:number;coordinatorRecoveryMs?:number|null};
export async function runRuntime(options:RuntimeConfig){
  for(const [key,min,max]of [['jobs',1,10000],['pages',1,5000],['concurrency',1,128],['batchJobs',1,128],['delayMs',0,1000]]as const)if(!Number.isInteger(options[key])||options[key]<min||options[key]>max)throw new Error(`invalid runtime ${key}`);
  if(options.batchJobs>options.concurrency||options.jobs*options.pages*2>1000000||!['node','rust'].includes(options.runtime)||options.runtime==='rust'&&options.backend&&options.backend!=='node-http')throw new Error('runtime corpus/backend budget exceeded');
  const directory=options.directory??await mkdtemp(join(tmpdir(),'goodfetch-native-workflow-'));await mkdir(directory,{recursive:true});
  const fixtures=await startFixtures();const ids=new Map<string,string>();const jobs:WorkflowJob[]=[];
  for(let i=0;i<options.jobs;i++){const fixture=await fixtures.createRun({scenario:'http-catalog',seed:options.seed+i,pages:options.pages,delayMs:options.delayMs,variant:options.variant});const jobId=`job-${i}`;ids.set(jobId,fixture.id);jobs.push({jobId,task:{scenario:'http-catalog',seed:options.seed+i,pages:options.pages,actions:1,timeoutMs:30000,variant:options.variant??'normal',url:fixture.url}})}
  const token=randomBytes(24).toString('hex');const oracle=createServer(async(req,res)=>{
    try{if(req.headers.authorization!==token){res.writeHead(403);res.end();return}let input='';for await(const chunk of req){input+=chunk;if(input.length>1_000_000)throw new Error('oracle input budget')}
      const payload=JSON.parse(input);const id=ids.get(payload.jobId);if(!id)throw new Error('unknown job');const verdict=await fixtures.verify(id,{records:payload.records});res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify(verdict));
    }catch(error){res.writeHead(400);res.end(JSON.stringify({error:String(error)}))}
  });await new Promise<void>(resolve=>oracle.listen(0,'127.0.0.1',resolve));
  let closed=false;async function close(){if(closed)return;closed=true;await new Promise<void>(resolve=>oracle.close(()=>resolve()));await fixtures.close()}
  const config={...options,path:join(directory,'state.sqlite'),jobs,leaseMs:options.leaseMs??30000,timeoutMs:30000,maxBatchWaitMs:10,restartLimit:4,oracleUrl:`http://127.0.0.1:${(oracle.address() as AddressInfo).port}`,oracleToken:token};
  const configPath=join(directory,'input.json');await writeFile(configPath,JSON.stringify(config)+'\n');
  let execution=0;let coordinatorCrashUsed=false;
  async function execute():Promise<RuntimeExecution>{
    if(closed)throw new Error('fixtures closed');
    const build=JSON.parse(await readFile(join(labRoot,'.native/build.json'),'utf8'));const currentLibrary=await hashFile(join(labRoot,`.native/${sqliteLibrary}`));if(currentLibrary!==build.hashes[sqliteLibrary])throw new Error('shared SQLite artifact differs from build');
    const rustBuild=JSON.parse(await readFile(join(labRoot,'.native/workflow-build.json'),'utf8'));if(options.runtime==='rust'&&(rustBuild.sqliteSha256!==currentLibrary||await hashFile(join(labRoot,'.native/goodfetch-workflow-rust'))!==rustBuild.binarySha256))throw new Error('Rust workflow artifact differs from build');
    const label=`execution-${execution++}`;const command=options.runtime==='rust'?join(labRoot,'.native/goodfetch-workflow-rust'):process.execPath;const args=options.runtime==='rust'?[configPath]:[join(labRoot,'src/workflow-native-runner.ts'),configPath];
    const child=spawn(command,args,{cwd:labRoot,stdio:['ignore','pipe','pipe'],detached:true});const sampler=sampleRuntimeResources(child.pid!,100);const events:Record<string,unknown>[]=[];let buffer='';let stderr='';let result:RuntimeResult|undefined;let fatal:string|undefined;const began=performance.now();
    let timer:NodeJS.Timeout;
    let coordinatorKilled=false;let killedAt=0;
    function killOwned(){for(const event of events.filter(e=>e.type==='workers-started'))for(const key of ['sinkPid','acquisitionPid']){const pid=Number(event[key]);if(pid>0&&pid!==child.pid)try{process.kill(options.runtime==='node'?-pid:pid,'SIGKILL')}catch{}}try{process.kill(-child.pid!,'SIGKILL')}catch{}}
    const completion=new Promise<void>((resolve,reject)=>{
      timer=setTimeout(()=>{try{process.kill(-child.pid!,'SIGKILL')}catch{}reject(new Error('runtime campaign watchdog exceeded 180 seconds'))},180000);
      child.stdout.setEncoding('utf8');child.stdout.on('data',(chunk:string)=>{buffer+=chunk;let n:number;while((n=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,n);buffer=buffer.slice(n+1);try{const event=JSON.parse(line);if(event.type==='summary')result=event.result;else{event.observerTimeMs=performance.now();events.push(event);if(event.type==='fatal')fatal=event.error;if(options.coordinatorCrashAt&&!coordinatorCrashUsed&&event.type===options.coordinatorCrashAt){coordinatorCrashUsed=true;coordinatorKilled=true;killedAt=performance.now();killOwned()}}}catch{fatal='invalid runtime JSONL output'}}});
      child.stderr.setEncoding('utf8');child.stderr.on('data',(chunk:string)=>{stderr=(stderr+chunk).slice(-100000)});child.on('error',reject);child.on('exit',(code,signal)=>code===0&&result&&!fatal?resolve():reject(new Error(fatal??`runtime exited ${code??signal}: ${stderr}`)));
    });
    let resources:Awaited<ReturnType<typeof sampler.stop>>;
    try{await completion;}catch(error){killOwned();if(!coordinatorKilled)throw error}
    finally{clearTimeout(timer!);resources=await sampler.stop();await writeFile(join(directory,`${label}-events.jsonl`),events.map(e=>JSON.stringify(e)).join('\n')+'\n');await writeFile(join(directory,`${label}-stderr.log`),stderr)}
    const runnerWallMs=performance.now()-began;const verifyStarted=performance.now();const snapshot=readWorkflow(config.path);
    if(coordinatorKilled){
      await writeFile(join(directory,`${label}-checkpoint.json`),JSON.stringify({at:options.coordinatorCrashAt,snapshot,resources},null,2)+'\n');config.coordinatorCrashAt=undefined;await writeFile(configPath,JSON.stringify(config)+'\n');
      const resumed=await execute();const firstAck=resumed.events.find(e=>e.type==='job-acked'||e.type==='reconciled');const totalVerifiedWallMs=performance.now()-began;
      const combined={...resumed,coordinatorRestarts:1,coordinatorRecoveryMs:firstAck?Number(firstAck.observerTimeMs)-killedAt:null,runnerWallMs:runnerWallMs+resumed.runnerWallMs,totalVerifiedWallMs,jobsPerSecondIncludingStartupAndOracle:resumed.verifiedJobs*1000/totalVerifiedWallMs,events:[...events,...resumed.events],resources:{...resumed.resources,samples:resources.samples+resumed.resources.samples,unavailable:resources.unavailable||resumed.resources.unavailable,observedProcessCount:resources.observedProcessCount+resumed.resources.observedProcessCount,peakRssBytes:Math.max(resources.peakRssBytes??0,resumed.resources.peakRssBytes??0),peakFootprintBytes:Math.max(resources.peakFootprintBytes??0,resumed.resources.peakFootprintBytes??0),sampledUserCpuMs:resources.sampledUserCpuMs+resumed.resources.sampledUserCpuMs,sampledSystemCpuMs:resources.sampledSystemCpuMs+resumed.resources.sampledSystemCpuMs,sampledDiskReadBytes:resources.sampledDiskReadBytes+resumed.resources.sampledDiskReadBytes,sampledDiskWriteBytes:resources.sampledDiskWriteBytes+resumed.resources.sampledDiskWriteBytes,perProcess:[...resources.perProcess,...resumed.resources.perProcess]}};
      await writeFile(join(directory,`${label}-recovery-summary.json`),JSON.stringify(combined,null,2)+'\n');return combined;
    }
    if(snapshot.jobs.length!==jobs.length||new Set(snapshot.jobs.map(j=>j.jobId)).size!==jobs.length||snapshot.integrity[0]!=='ok'||snapshot.integrity.length!==1||snapshot.foreignKeys.length)throw new Error('independent persisted workflow structure failed');
    let verifiedJobs=0;let verifiedRecords=0;const verdicts=[];
    for(const job of snapshot.jobs){const verdict=job.state==='acked'?await fixtures.verify(ids.get(job.jobId)!,{records:job.records}):{status:'failed',reasons:[`job is ${job.state}`]};verdicts.push({jobId:job.jobId,...verdict});if(verdict.status==='verified'){verifiedJobs++;verifiedRecords+=job.records.length}}
    const finalOracleMs=performance.now()-verifyStarted;if(verifiedJobs!==result!.verifiedJobs||verifiedRecords!==result!.verifiedRecords)throw new Error('runtime reported false verified counts');if(result!.peakBufferedJobs>options.concurrency)throw new Error('result queue bound violated');
    for(const info of result!.info){const sink=options.runtime==='node'?info.sink as Record<string,unknown>:info;if(sink.sqlite!==build.sqlite||sink.sourceId!==build.sqliteSourceId||sink.libraryPath!==build.libraryPath)throw new Error('runtime SQLite engine metadata mismatch')}
    const summary={options,result:result!,verifiedJobs,verifiedRecords,resources,runnerWallMs,finalOracleMs,totalVerifiedWallMs:runnerWallMs+finalOracleMs,jobsPerSecondIncludingStartupAndOracle:verifiedJobs*1000/(runnerWallMs+finalOracleMs),sqliteSha256:currentLibrary,verdicts};
    await writeFile(join(directory,`${label}-summary.json`),JSON.stringify(summary,null,2)+'\n');await writeFile(join(directory,`${label}-persisted.json`),JSON.stringify(snapshot,null,2)+'\n');
    return {...summary,events,snapshot};
  }
  try{return {...await execute(),directory,resume:execute,close}}catch(error){await close();throw error}
}
