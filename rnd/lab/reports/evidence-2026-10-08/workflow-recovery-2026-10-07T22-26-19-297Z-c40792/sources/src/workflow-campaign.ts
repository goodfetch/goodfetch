import {appendFile,mkdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {randomBytes} from 'node:crypto';
import {cpus,arch,release,totalmem} from 'node:os';
import {startFixtures} from '../fixtures/server.ts';
import {executeWorkflow,readWorkflow,type WorkflowJob} from './workflow.ts';
import {labRoot,type BackendName} from './registry.ts';
import {sourceFingerprint,hashFile,copySourceSnapshot} from './environment.ts';
import {shuffle,quantiles} from './statistics.ts';

export type WorkflowCampaign={name:string;backends:BackendName[];blocks:number;jobs:number;pages:number;concurrency:number[];batchJobs:number[];maxBatchWaitMs:number;leaseMs:number;timeoutMs:number;delayMs:number;seed:number;faults:('none'|'before-commit'|'after-commit')[]};
export function validateWorkflowCampaign(c:WorkflowCampaign){
  if(!c||!/^[a-z0-9-]{1,80}$/.test(c.name))throw new Error('invalid workflow campaign');
  for(const [values,allowed]of [[c.backends,['node-http','spider-http','crawlee-http','crawlee-http-pooled']],[c.faults,['none','before-commit','after-commit']]]as const)if(!Array.isArray(values)||!values.length||new Set(values).size!==values.length||!values.every(v=>(allowed as readonly string[]).includes(v)))throw new Error('invalid workflow backends/faults');
  for(const [key,min,max]of [['blocks',1,20],['jobs',1,1000],['pages',1,1000],['maxBatchWaitMs',1,1000],['leaseMs',10,120000],['timeoutMs',100,120000],['delayMs',0,1000],['seed',0,2**32-1]]as const)if(!Number.isInteger(c[key])||c[key]<min||c[key]>max)throw new Error(`invalid workflow ${key}`);
  for(const values of [c.concurrency,c.batchJobs])if(!Array.isArray(values)||!values.length||new Set(values).size!==values.length||!values.every(v=>Number.isInteger(v)&&v>0&&v<=128))throw new Error('invalid workflow concurrency/batches');
  if(!c.concurrency.some(n=>c.batchJobs.some(b=>b<=n)))throw new Error('no valid bounded batch configuration');
  if(c.jobs*c.pages*2>1000000||c.blocks*c.backends.length*c.concurrency.length*c.batchJobs.length*c.faults.length>256)throw new Error('workflow corpus/matrix budget exceeded');
}
export async function runWorkflowCampaign(config:WorkflowCampaign,outputRoot=join(labRoot,'results')){
  validateWorkflowCampaign(config);
  const directory=join(outputRoot,`${config.name}-${new Date().toISOString().replace(/[:.]/g,'-')}-${randomBytes(3).toString('hex')}`);await mkdir(directory,{recursive:true});
  const sourceHashes=await sourceFingerprint();
  await copySourceSnapshot(directory,sourceHashes);
  const spiderBinary=config.backends.includes('spider-http')?await hashFile(join(labRoot,'adapters/rust/target/http/release/automan-lab-spider')):null;
  await writeFile(join(directory,'manifest.json'),JSON.stringify({schemaVersion:1,startedAt:new Date().toISOString(),config,environment:{node:process.version,nodeExecutable:process.execPath,arch:arch(),os:release(),cpu:cpus()[0]?.model,totalMemoryBytes:totalmem(),sourceHashes,spiderBinary},methodology:{workflow:'HTTP acquisition + extraction → private oracle → bounded batch writer → atomic results/job commit → durable ack',durability:'Node SQLite WAL FULL; separate commit and ack transactions',backpressure:'at most concurrency unacknowledged jobs; one batch in flight; batchJobs <= concurrency',timing:'startup separated; work includes acquisition, validation, queue wait, commit, ack, recovery and worker drain; independent final reopen separately; no warmup removed',rss:'coordinator including fixtures/oracle + owned descendants; sampled every 250 ms',faults:'writer SIGKILL inside transaction or after COMMIT before response; coordinator stays alive; reacquisition only for idempotent HTTP reads',limits:['prototype is not the final runtime architecture','wall-clock leases, no heartbeat or migration','not a browser interaction or power-loss recovery proof','machine not reserved; exploratory paired blocks']}},null,2)+'\n');
  const cells:Record<string,unknown>[]=[];
  const combinations=config.backends.flatMap(backend=>config.concurrency.flatMap(concurrency=>config.batchJobs.filter(batchJobs=>batchJobs<=concurrency).flatMap(batchJobs=>config.faults.map(fault=>({backend,concurrency,batchJobs,fault})))));
  for(let block=0;block<config.blocks;block++)for(const cell of shuffle(combinations,config.seed+block)){
    const label=`${cell.backend}-c${cell.concurrency}-b${cell.batchJobs}-${cell.fault}-r${block}`;
    const cellDirectory=join(directory,label);await mkdir(cellDirectory,{recursive:true});const path=join(cellDirectory,'state.sqlite');
    const fixtures=await startFixtures();let flush=Promise.resolve();
    try{
      const jobs:WorkflowJob[]=[];const ids=new Map<string,string>();
      for(let i=0;i<config.jobs;i++){const jobId=`job-${i}`;const seed=config.seed+block*100003+i;const fixture=await fixtures.createRun({scenario:'http-catalog',seed,pages:config.pages,delayMs:config.delayMs});ids.set(jobId,fixture.id);jobs.push({jobId,task:{scenario:'http-catalog',seed,pages:config.pages,actions:1,timeoutMs:config.timeoutMs,variant:'normal',url:fixture.url}})}
      const result=await executeWorkflow({path,backend:cell.backend,concurrency:cell.concurrency,batchJobs:cell.batchJobs,maxBatchWaitMs:config.maxBatchWaitMs,leaseMs:config.leaseMs,timeoutMs:config.timeoutMs,fault:cell.fault==='none'?undefined:cell.fault,restartLimit:cell.fault==='none'?0:1},jobs,async(id,records)=>fixtures.verify(ids.get(id)!,{records}),{onEvent:async row=>{flush=flush.then(()=>appendFile(join(cellDirectory,'events.jsonl'),JSON.stringify(row)+'\n'));await flush}});
      await writeFile(join(cellDirectory,'persisted.json'),JSON.stringify({snapshot:result.snapshot,verdicts:result.verdicts},null,2)+'\n');
      await writeFile(join(cellDirectory,'attempts.jsonl'),result.attempts.map(a=>JSON.stringify(a)).join('\n')+'\n');
      const acknowledged=result.events.filter(e=>e.type==='job-acked'||e.type==='reconciled');
      const batches=result.events.filter(e=>e.type==='batch-acked');const successful=result.attempts.filter(a=>a.status==='acked');
      cells.push({...cell,block,status:result.verifiedJobs===config.jobs&&result.verifiedRecords===config.jobs*config.pages*2?'verified':'failed',admittedJobs:config.jobs,verifiedJobs:result.verifiedJobs,verifiedRecords:result.verifiedRecords,startupMs:result.startupMs,workMs:result.workMs,finalizationMs:result.finalizationMs,totalMs:result.totalMs,verifiedJobsPerSecond:result.verifiedJobsPerSecond,jobsPerSecondIncludingStartup:result.verifiedJobs*1000/result.totalMs,peakBufferedJobs:result.peakBufferedJobs,restarts:result.restarts,reconciledJobs:result.reconciledJobs,info:result.info,resources:result.resources,ackLatencyMs:quantiles(acknowledged.filter(e=>typeof e.endToEndMs==='number').map(e=>Number(e.endToEndMs))),acquisitionAndExtractionMs:quantiles(successful.map(a=>Number(a.acquisitionMs))),validationMs:quantiles(successful.map(a=>Number(a.validationMs))),sinkWaitCommitAckMs:quantiles(successful.map(a=>Number(a.sinkWaitCommitAckMs))),commitRpcMs:quantiles(batches.map(e=>Number(e.commitMs))),ackRpcMs:quantiles(batches.map(e=>Number(e.ackMs))),batchJobCounts:batches.map(e=>e.jobs),abortedAttempts:result.attempts.filter(a=>a.status==='aborted').length,failedAttempts:result.attempts.filter(a=>a.status==='failed').length});
    }catch(error){await flush;let snapshot:unknown;try{snapshot=readWorkflow(path)}catch{}await writeFile(join(cellDirectory,'failed-state.json'),JSON.stringify({snapshot,error:error instanceof Error?error.message:String(error)},null,2)+'\n');cells.push({...cell,block,status:'failed',admittedJobs:config.jobs,verifiedJobs:0,error:error instanceof Error?error.message:String(error)});
    }finally{await fixtures.close()}
    await writeFile(join(directory,'summary.json'),JSON.stringify({cells},null,2)+'\n');process.stderr.write(`${label}: ${cells.at(-1)!.status}\n`);
  }
  const sourceChanges=Object.entries(await sourceFingerprint()).filter(([p,h])=>sourceHashes[p]!==h).map(([p])=>p);
  const binaryChanged=spiderBinary!==null&&spiderBinary!==await hashFile(join(labRoot,'adapters/rust/target/http/release/automan-lab-spider'));
  await writeFile(join(directory,'summary.json'),JSON.stringify({cells,sourceChanges,binaryChanged},null,2)+'\n');
  const esc=(v:unknown)=>String(v??'—').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
  const rows=cells.map(c=>{const latency=c.ackLatencyMs as ReturnType<typeof quantiles>|undefined;return `<tr>${[c.backend,c.concurrency,c.batchJobs,c.fault,c.block,c.status,`${c.verifiedJobs}/${c.admittedJobs}`,typeof c.verifiedJobsPerSecond==='number'?c.verifiedJobsPerSecond.toFixed(1):null,latency?.p95?.toFixed(2),latency?.n,c.peakBufferedJobs,c.restarts,c.reconciledJobs].map(v=>`<td>${esc(v)}</td>`).join('')}</tr>`}).join('');
  await writeFile(join(directory,'report.html'),`<!doctype html><html lang="fr"><meta charset="utf-8"><title>Automan workflow durable</title><style>body{font:15px system-ui;margin:32px;background:#f8fafc}table{border-collapse:collapse;background:white}td,th{padding:10px;border-bottom:1px solid #ddd;text-align:left}</style><h1>Acquisition → validation → commit → ack</h1><p>Débit utile vérifié après réouverture. Démarrage séparé ; drain, reprise et contrôle final inclus au dénominateur. Latence depuis la première prise de lease jusqu'à l'ack, y compris la reprise. RSS du coordinateur et descendants. Crash du writer ; aucune preuve de reprise du coordinateur ni de coupure électrique. Résultats exploratoires sur un host partagé.</p><table><thead><tr>${['Backend','C','Batch jobs','Crash','Bloc','État','Jobs vérifiés','Jobs/s','p95 ack ms','n latences','Queue max','Reprises','Commits réconciliés'].map(h=>`<th>${h}</th>`).join('')}</tr></thead><tbody>${rows}</tbody></table><p><a href="manifest.json">Manifest</a> · <a href="summary.json">Mesures détaillées</a></p></html>\n`);
  return {directory,cells,sourceChanges,binaryChanged};
}
