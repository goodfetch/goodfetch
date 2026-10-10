import {mkdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {randomBytes} from 'node:crypto';
import {runtimeCampaign,type RuntimeCampaign} from '../src/runtime-campaign.ts';
import {actionabilityCampaign} from '../src/actionability-campaign.ts';
import {readersCampaign} from '../src/sqlite-readers-campaign.ts';
import {runCampaign,type Campaign} from '../src/campaign.ts';
import {labRoot} from '../src/registry.ts';
import {environment,copySourceSnapshot,sourceFingerprint} from '../src/environment.ts';
const id=`qualification-integrated-${new Date().toISOString().replace(/[:.]/g,'-')}-${randomBytes(3).toString('hex')}`;
const directory=join(labRoot,'results',id);await mkdir(directory,{recursive:true});const env=await environment();await copySourceSnapshot(directory,env.sourceHashes);
const native:RuntimeCampaign={name:'runtime-native-sustained',blocks:3,seed:7711,cells:[1,4,16].flatMap(concurrency=>[1,4].filter(batchJobs=>batchJobs<=concurrency).flatMap(batchJobs=>['node','rust'].map(runtime=>({runtime:runtime as 'node'|'rust',concurrency,batchJobs,jobs:concurrency*192,pages:16,delayMs:4}))))};
const backends:RuntimeCampaign={name:'runtime-http-backends',blocks:3,seed:7711,cells:[4,16].flatMap(concurrency=>[
  {runtime:'node' as const,concurrency,batchJobs:4,jobs:concurrency*96,pages:16,delayMs:4},
  {runtime:'rust' as const,concurrency,batchJobs:4,jobs:concurrency*96,pages:16,delayMs:4},
  {runtime:'node' as const,backend:'spider-http' as const,concurrency,batchJobs:4,jobs:concurrency*96,pages:16,delayMs:4},
  {runtime:'node' as const,backend:'crawlee-http-pooled' as const,poolWake:'notify' as const,concurrency,batchJobs:4,jobs:concurrency*96,pages:16,delayMs:4}
])};
const recovery:RuntimeCampaign={name:'runtime-recovery',blocks:3,seed:8811,cells:['node','rust'].flatMap(runtime=>[
  ...(['before-commit','after-commit','during-ack','after-ack'] as const).map(fault=>({runtime:runtime as 'node'|'rust',fault,jobs:24,pages:4,delayMs:2,concurrency:4,batchJobs:2,leaseMs:1000})),
  ...(['job-claimed','batch-committed'] as const).map(coordinatorCrashAt=>({runtime:runtime as 'node'|'rust',coordinatorCrashAt,jobs:24,pages:4,delayMs:2,concurrency:4,batchJobs:2,leaseMs:1000})),
  {runtime:runtime as 'node'|'rust',faults:['before-commit','during-ack','after-ack'],writerDelayMs:20,jobs:24,pages:4,delayMs:2,concurrency:4,batchJobs:2,leaseMs:1000}
])};
const transport:Campaign={name:'http-transport',backends:['node-http','spider-http','crawlee-http','crawlee-http-pooled'],scenarios:['http-catalog'],seed:4411,blocks:3,iterations:15,warmup:1,concurrency:4,mode:'closed',timeoutMs:15000,maxDurationMs:120000,pages:4,actions:1,delayMs:2,poolWake:'notify',variants:['normal','http-redirect','http-gzip','http-chunked','http-large']};
const offered=[20,100,250].map(ratePerSecond=>({name:`http-open-${ratePerSecond}`,backends:['node-http','spider-http','crawlee-http','crawlee-http-pooled'],scenarios:['http-catalog'],seed:4411,blocks:3,iterations:ratePerSecond*12,warmup:2,concurrency:4,mode:'open',ratePerSecond,timeoutMs:15000,maxDurationMs:30000,pages:8,actions:1,delayMs:3,poolWake:'notify'} as Campaign));
const steps:{name:string;directory?:string;status:string;error?:string;verified?:number;total?:number}[]=[];
await writeFile(join(directory,'manifest.json'),JSON.stringify({id,environment:env,native,backends,recovery,transport,offered,actionability:{blocks:3},readers:{blocks:3},methodology:'all campaigns run sequentially; frozen source snapshots and per-campaign manifests; comparator failures remain visible; finite corpus and local Mac limitations explicit'},null,2)+'\n');
async function step(name:string,run:()=>Promise<{directory:string;cells?:Record<string,unknown>[];rows?:{status:string}[];sourceChanges?:string[];binaryChanges?:string[];binaryChanged?:boolean}>){
  process.stderr.write(`START ${name}\n`);try{const result=await run();const cells=result.cells??result.rows??[];const changed=!!result.sourceChanges?.length||!!result.binaryChanges?.length||result.binaryChanged;
    steps.push({name,directory:result.directory,status:changed?'invalid-provenance':'executed',verified:cells.filter(c=>c.status==='verified').length,total:cells.length});
  }catch(error){steps.push({name,status:'failed',error:String(error)})}
  await writeFile(join(directory,'summary.json'),JSON.stringify({steps},null,2)+'\n');process.stderr.write(`FINISH ${name}: ${steps.at(-1)!.status}\n`);
}
for(const config of [native,backends,recovery]){await writeFile(join(directory,`${config.name}.json`),JSON.stringify(config,null,2)+'\n');await step(config.name,()=>runtimeCampaign(config))}
await step('browser-actionability',()=>actionabilityCampaign(3));
await step('sqlite-readers',()=>readersCampaign(3));
for(const config of [transport,...offered]){await writeFile(join(directory,`${config.name}.json`),JSON.stringify(config,null,2)+'\n');await step(config.name,()=>runCampaign(config))}
const sourceChanges=Object.entries(await sourceFingerprint()).filter(([file,hash])=>env.sourceHashes[file]!==hash).map(([file])=>file);
await writeFile(join(directory,'summary.json'),JSON.stringify({steps,sourceChanges},null,2)+'\n');process.stdout.write(JSON.stringify({directory,steps,sourceChanges})+'\n');if(steps.some(s=>s.status!=='executed')||sourceChanges.length)process.exitCode=1;
