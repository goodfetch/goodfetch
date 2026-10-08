import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {randomBytes} from 'node:crypto';
import {runRuntime,type RuntimeConfig} from './workflow-runtime.ts';
import {labRoot} from './registry.ts';
import {environment,sourceFingerprint,copySourceSnapshot,hashFile} from './environment.ts';
import {shuffle,quantiles} from './statistics.ts';
export type RuntimeCampaign={name:string;blocks:number;cells:Omit<RuntimeConfig,'directory'|'seed'>[];seed:number};
// One native workflow cell (Node or Rust coordinator); the returned record is the legacy summary cell.
export async function runRuntimeCell(config:RuntimeCampaign,index:number,block:number,directory:string):Promise<Record<string,unknown>>{
  const cell=config.cells[index]!;const cellDir=join(directory,`cell-${index}-b${block}`);const began=performance.now();
  try{const run=await runRuntime({...cell,seed:config.seed+block*100003,directory:cellDir});try{
    const latency=run.events.filter(e=>(e.type==='job-acked'||e.type==='reconciled')&&typeof e.endToEndMs==='number');const batches=run.events.filter(e=>e.type==='batch-acked');
    const base={...cell,index,block,status:run.verifiedJobs===cell.jobs?'verified':'failed',verifiedJobs:run.verifiedJobs,verifiedRecords:run.verifiedRecords,startupMs:run.result.startupMs,workMs:run.result.workMs,finalizationMs:run.result.finalizationMs,totalVerifiedWallMs:run.totalVerifiedWallMs,verifiedJobsPerSecond:run.verifiedJobs*1000/(run.result.workMs+run.result.finalizationMs),jobsPerSecondIncludingStartupAndOracle:run.jobsPerSecondIncludingStartupAndOracle,resources:run.resources,peakBufferedJobs:run.result.peakBufferedJobs,restarts:run.result.restarts,coordinatorRestarts:run.coordinatorRestarts??0,coordinatorRecoveryMs:run.coordinatorRecoveryMs??null,reconciledJobs:run.result.reconciledJobs,ackLatencyMs:quantiles(latency.map(e=>Number(e.endToEndMs))),batchJobCounts:batches.map(e=>e.jobs),commitMs:quantiles(batches.map(e=>Number(e.commitMs))),ackMs:quantiles(batches.map(e=>Number(e.ackMs))),info:run.result.info};
    await writeFile(join(cellDir,'cell.json'),JSON.stringify(base,null,2)+'\n');return base;
  }finally{await run.close()}}catch(error){return {...cell,index,block,status:'failed',error:String(error),elapsedMs:performance.now()-began}}
}
export async function runtimeCampaign(config:RuntimeCampaign){
  const id=`${config.name}-${new Date().toISOString().replace(/[:.]/g,'-')}-${randomBytes(3).toString('hex')}`;const directory=join(labRoot,'results',id);await mkdir(directory,{recursive:true});
  const env=await environment();const binaries:Record<string,string>={};for(const file of ['libautoman_sqlite.dylib','sqlite.node','automan-workflow-rust','process-probe'])binaries[file]=await hashFile(join(labRoot,'.native',file));
  await copySourceSnapshot(directory,env.sourceHashes);const nativeBuild=JSON.parse(await readFile(join(labRoot,'.native/build.json'),'utf8'));const rustBuild=JSON.parse(await readFile(join(labRoot,'.native/workflow-build.json'),'utf8'));
  const manifest={id,config,environment:env,binaries,nativeBuild,rustBuild,methodology:'randomized paired blocks; same generated corpus by seed; one coordinator event thread and dedicated sink process for native Node/Rust; common C SQL binding and same SQLite artifact; Rust reqwest and Node fetch fixture-specific regex parsers; private HTTP oracle per acquired job; independent SQLite re-open after process exit; bounded in-memory results but finite durable backlog; full wall time includes startup/drain/oracle; sampled macOS process-family CPU/RSS/I/O with limitations in each resource record; backend adapters use their declared lifecycle; no cloud/model calls'};
  await writeFile(join(directory,'manifest.json'),JSON.stringify(manifest,null,2)+'\n');const cells:Record<string,unknown>[]=[];
  for(let block=0;block<config.blocks;block++)for(const index of shuffle(config.cells.map((_,i)=>i),config.seed+block)){
    const cell=config.cells[index]!;cells.push(await runRuntimeCell(config,index,block,directory));
    await writeFile(join(directory,'summary.json'),JSON.stringify({cells},null,2)+'\n');process.stderr.write(`${config.name}/${cell.runtime}/${cell.backend??'native'}/C${cell.concurrency}/B${cell.batchJobs}/b${block}: ${cells.at(-1)!.status}\n`);
  }
  const sourceChanges=Object.entries(await sourceFingerprint()).filter(([file,hash])=>env.sourceHashes[file]!==hash).map(([file])=>file);const binaryChanges=[];for(const [file,hash]of Object.entries(binaries))if(await hashFile(join(labRoot,'.native',file))!==hash)binaryChanges.push(file);
  await writeFile(join(directory,'summary.json'),JSON.stringify({cells,sourceChanges,binaryChanges},null,2)+'\n');return {directory,cells,sourceChanges,binaryChanges};
}
if(process.argv[1]?.endsWith('runtime-campaign.ts')){const config=JSON.parse(await readFile(process.argv[2]!,'utf8')) as RuntimeCampaign;const result=await runtimeCampaign(config);process.stdout.write(JSON.stringify({directory:result.directory,verified:result.cells.filter(c=>c.status==='verified').length,total:result.cells.length,sourceChanges:result.sourceChanges,binaryChanges:result.binaryChanges})+'\n');if(result.cells.some(c=>c.status!=='verified')||result.sourceChanges.length||result.binaryChanges.length)process.exitCode=1}
