import { DatabaseSync } from 'node:sqlite';
import { join } from 'node:path';
import { WorkerClient, type WorkerCommand } from './worker-client.ts';
import { commandFor, labRoot, type BackendName } from './registry.ts';
import { sampleProcessGroup } from './resources.ts';
import type { Task, AdapterOutput } from './worker-server.ts';

export type WorkflowConfig={path:string;backend:BackendName;concurrency:number;batchJobs:number;maxBatchWaitMs:number;leaseMs:number;timeoutMs:number;fault?:'before-commit'|'after-commit';restartLimit?:number};
export type WorkflowJob={jobId:string;task:Task};
export type PersistedJob=WorkflowJob&{state:string;token:string|null;expires:number;attempts:number;receipt:string|null;records:{id:string;price:number}[];error?:string};
type Snapshot={jobs:PersistedJob[];integrity:unknown[];foreignKeys:unknown[]};
type Claim={jobId:string;task:Task;token:string;expires:number};
type Entry={jobId:string;token:string;records:{id:string;price:number}[]};
type Receipt={jobId:string;receipt:string};
type Validate=(jobId:string,records:{id:string;price:number}[])=>Promise<{status:'verified'|'failed';reasons:string[]}>;
export function readWorkflow(path:string):Snapshot{
  const db=new DatabaseSync(path,{readOnly:true});try{
    const jobs=db.prepare('SELECT * FROM jobs ORDER BY rowid').all().map(row=>({...row,task:JSON.parse(String(row.task)),records:db.prepare('SELECT id,price FROM results WHERE jobId=? ORDER BY id').all(row.jobId!).map(r=>({id:String(r.id),price:Number(r.price)}))})) as unknown as PersistedJob[];
    return {jobs,integrity:db.prepare('PRAGMA integrity_check').all().map(r=>Object.values(r)[0]),foreignKeys:db.prepare('PRAGMA foreign_key_check').all()};
  }finally{db.close()}
}
class CommitQueue{
  queue:{entry:Entry;resolve:(receipt:Receipt)=>void;reject:(error:Error)=>void}[]=[];
  busy=false;failed:Error|undefined;timer:NodeJS.Timeout|undefined;peak=0;inflight=0;
  private config:WorkflowConfig;private sink:WorkerClient;private fault:()=>WorkflowConfig['fault'];private event:(row:Record<string,unknown>)=>Promise<void>;
  constructor(config:WorkflowConfig,sink:WorkerClient,fault:()=>WorkflowConfig['fault'],event:(row:Record<string,unknown>)=>Promise<void>){this.config=config;this.sink=sink;this.fault=fault;this.event=event}
  push(entry:Entry){if(this.failed)return Promise.reject(this.failed);if(this.queue.length+this.inflight>=this.config.concurrency)return Promise.reject(new Error('bounded result queue exceeded'));
    const promise=new Promise<Receipt>((resolve,reject)=>this.queue.push({entry,resolve,reject}));this.peak=Math.max(this.peak,this.queue.length+this.inflight);this.schedule();return promise;
  }
  private schedule(){if(this.failed||this.busy||!this.queue.length)return;if(this.queue.length>=this.config.batchJobs){void this.flush();return}if(!this.timer)this.timer=setTimeout(()=>{this.timer=undefined;void this.flush()},this.config.maxBatchWaitMs)}
  abort(error:Error){this.failed=error;if(this.timer){clearTimeout(this.timer);this.timer=undefined}for(const item of this.queue.splice(0))item.reject(error)}
  private async flush(){
    if(this.failed||this.busy||!this.queue.length)return;if(this.timer){clearTimeout(this.timer);this.timer=undefined}this.busy=true;const batch=this.queue.splice(0,this.config.batchJobs);this.inflight=batch.length;const began=performance.now();
    try{const commit=await this.sink.call('run',{op:'commit',entries:batch.map(x=>x.entry),fault:this.fault()}) as {receipts:Receipt[]};const commitMs=performance.now()-began;
      if(!Array.isArray(commit.receipts)||commit.receipts.length!==batch.length||new Set(commit.receipts.map(r=>r.jobId)).size!==batch.length||commit.receipts.some(r=>!batch.some(x=>x.entry.jobId===r.jobId)||typeof r.receipt!=='string'))throw new Error('invalid commit receipt');
      await this.event({type:'batch-committed',jobs:batch.length,commitMs});
      const ackBegan=performance.now();const ack=await this.sink.call('run',{op:'ack',receipts:commit.receipts}) as {acknowledged:number};if(ack.acknowledged!==batch.length)throw new Error('invalid workflow acknowledgement');
      await this.event({type:'batch-acked',jobs:batch.length,commitMs,ackMs:performance.now()-ackBegan});for(const item of batch)item.resolve(commit.receipts.find(r=>r.jobId===item.entry.jobId)!);
    }catch(error){const e=error instanceof Error?error:new Error(String(error));for(const item of batch)item.reject(e);this.abort(e)}finally{this.inflight=0;this.busy=false;this.schedule()}
  }
}
export async function executeWorkflow(config:WorkflowConfig,jobs:WorkflowJob[],validate:Validate,options:{command?:WorkerCommand;onEvent?:(row:Record<string,unknown>)=>Promise<void>}={}){
  if(!['node-http','crawlee-http','crawlee-http-pooled','spider-http'].includes(config.backend))throw new Error('workflow prototype supports HTTP backends only');
  for(const [key,min,max]of [['concurrency',1,128],['batchJobs',1,128],['maxBatchWaitMs',1,1000],['leaseMs',10,120000],['timeoutMs',100,120000]]as const)if(!Number.isInteger(config[key])||config[key]<min||config[key]>max)throw new Error(`invalid workflow ${key}`);
  if(config.batchJobs>config.concurrency||!jobs.length||jobs.length>10000||jobs.length*Math.max(...jobs.map(j=>j.task.pages))*2>1000000)throw new Error('workflow queue/corpus budget exceeded');
  if(config.fault!==undefined&&!['before-commit','after-commit'].includes(config.fault))throw new Error('invalid workflow fault');
  if(config.restartLimit!==undefined&&(!Number.isInteger(config.restartLimit)||config.restartLimit<0||config.restartLimit>4))throw new Error('invalid restart budget');
  const events:Record<string,unknown>[]=[];const info:unknown[]=[];async function event(row:Record<string,unknown>){events.push(row);await options.onEvent?.(row)}
  const resourceSampler=sampleProcessGroup(process.pid);const started=performance.now();const attempts:Record<string,unknown>[]=[];const firstAdmission=new Map<string,number>();let workStarted=0;let startupMs=0;let peakBufferedJobs=0;let restarts=0;let reconciledJobs=0;let faultUsed=false;
  try{
    for(;;){
      const sink=new WorkerClient({command:process.execPath,args:[join(labRoot,'adapters/workflow-node/worker.ts')],cwd:labRoot,timeoutMs:config.timeoutMs});
      const acquisition=new WorkerClient(options.command??commandFor(config.backend,config.timeoutMs));let queue:CommitQueue|undefined;
      try{
        await event({type:'workers-started',generation:restarts,sinkPid:sink.pid,acquisitionPid:acquisition.pid});
        await sink.call('prepare',{path:config.path,leaseMs:config.leaseMs});await sink.call('run',{op:'submit',jobs});
        await acquisition.call('hello',{});await acquisition.call('prepare',{concurrency:config.concurrency});
        info.push({generation:restarts,sink:await sink.call('hello',{}),acquisition:await acquisition.call('hello',{})});
        if(!workStarted){startupMs=performance.now()-started;workStarted=performance.now()}
        const stored=await sink.call('run',{op:'snapshot'}) as Snapshot;
        for(const job of stored.jobs.filter(j=>j.state==='committed')){const verdict=await validate(job.jobId,job.records);if(verdict.status!=='verified')throw new Error(`persisted committed data failed oracle: ${verdict.reasons.join('; ')}`);await sink.call('run',{op:'ack',receipts:[{jobId:job.jobId,receipt:job.receipt}]});reconciledJobs++;await event({type:'reconciled',jobId:job.jobId,endToEndMs:firstAdmission.has(job.jobId)?performance.now()-firstAdmission.get(job.jobId)!:null})}
        const leased=stored.jobs.filter(j=>j.state==='leased');if(leased.length){const until=Math.max(...leased.map(j=>j.expires));await event({type:'lease-expiry-wait',leasedJobs:leased.length,waitMs:Math.max(0,until-Date.now())});while(Date.now()<until)await new Promise(resolve=>setTimeout(resolve,Math.min(50,until-Date.now())))}
        let restartError:Error|undefined;
        queue=new CommitQueue(config,sink,()=>{if(config.fault&&!faultUsed){faultUsed=true;return config.fault}return undefined},event);
        const commitQueue=queue;
        async function producer(){
          while(!restartError){
            let claim:Claim|{job:null};try{claim=await sink.call('run',{op:'claim'}) as Claim|{job:null}}catch(error){restartError=error instanceof Error?error:new Error(String(error));commitQueue.abort(restartError);break}
            if('job'in claim)break;const began=performance.now();if(!firstAdmission.has(claim.jobId))firstAdmission.set(claim.jobId,began);
            await event({type:'job-claimed',jobId:claim.jobId,generation:restarts,expires:claim.expires});
            let acquisitionMs=0,validationMs=0;
            try{
              const output=await acquisition.call('run',claim.task) as AdapterOutput;acquisitionMs=performance.now()-began;
              const validationStart=performance.now();const verdict=output.status==='ok'&&Array.isArray(output.records)?await validate(claim.jobId,output.records):{status:'failed',reasons:['invalid acquisition output']};validationMs=performance.now()-validationStart;
              if(verdict.status!=='verified'){await sink.call('run',{op:'reject',jobId:claim.jobId,token:claim.token,reason:verdict.reasons.join('; ')});attempts.push({jobId:claim.jobId,generation:restarts,status:'failed',acquisitionMs,validationMs,reasons:verdict.reasons});continue}
              const queued=performance.now();await commitQueue.push({jobId:claim.jobId,token:claim.token,records:output.records!});
              const attempt={jobId:claim.jobId,generation:restarts,status:'acked',acquisitionMs,validationMs,sinkWaitCommitAckMs:performance.now()-queued,endToEndMs:performance.now()-firstAdmission.get(claim.jobId)!,records:output.records!.length};attempts.push(attempt);await event({type:'job-acked',...attempt});
            }catch(error){const message=error instanceof Error?error.message:String(error);attempts.push({jobId:claim.jobId,generation:restarts,status:'aborted',acquisitionMs,validationMs,error:message});restartError=error instanceof Error?error:new Error(message);commitQueue.abort(restartError)}
          }
        }
        await Promise.all(Array.from({length:config.concurrency},()=>producer()));peakBufferedJobs=Math.max(peakBufferedJobs,queue.peak);
        if(restartError){if(restarts>=(config.restartLimit??2))throw restartError;restarts++;await event({type:'restart',generation:restarts,error:restartError.message});continue}
        break;
      }finally{queue?.abort(new Error('generation closed'));await Promise.all([sink.close(),acquisition.close()])}
    }
    const workMs=performance.now()-workStarted;const finalizationStart=performance.now();const snapshot=readWorkflow(config.path);const verdicts:{jobId:string;status:'verified'|'failed';reasons:string[]}[]=[];
    for(const job of snapshot.jobs){const verdict=job.state==='acked'?await validate(job.jobId,job.records):{status:'failed' as const,reasons:[`job is ${job.state}`]};verdicts.push({jobId:job.jobId,...verdict})}
    if(snapshot.integrity.length!==1||snapshot.integrity[0]!=='ok'||snapshot.foreignKeys.length)throw new Error('workflow structural integrity failed');
    const finalizationMs=performance.now()-finalizationStart;const verifiedJobs=verdicts.filter(v=>v.status==='verified').length;const verifiedRecords=snapshot.jobs.filter(j=>verdicts.find(v=>v.jobId===j.jobId)?.status==='verified').reduce((n,j)=>n+j.records.length,0);
    return {snapshot,verdicts,attempts,events,info,startupMs,workMs,finalizationMs,totalMs:performance.now()-started,verifiedJobs,verifiedRecords,verifiedJobsPerSecond:verifiedJobs*1000/(workMs+finalizationMs),peakBufferedJobs,restarts,reconciledJobs,resources:resourceSampler.stop()};
  }finally{resourceSampler.stop()}
}
