import { sqliteLibrary } from './registry.ts';
import {mkdtemp,mkdir,writeFile,stat} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {WorkerClient} from './worker-client.ts';
import {storageCommand,sharedBuildMetadata,checkSharedInfo,type StorageEngine} from './storage-engines.ts';
import {verifyStorage,type StorageBatch} from './storage.ts';
import {SharedDatabase} from '../adapters/workflow-node/shared-database.ts';
import {sampleRuntimeResources} from './runtime-resources.ts';
import {quantiles} from './statistics.ts';
export type ReaderConfig={engine:Extract<StorageEngine,'node-shared'|'rust-sync'|'go-sync'>;writers:number;readerMode:'none'|'short'|'held';batchRows:number;batches:number;payloadBytes:number;seed:number;directory?:string};
export async function runReaderExperiment(config:ReaderConfig){
  const directory=config.directory??await mkdtemp(join(tmpdir(),'goodfetch-readers-'));await mkdir(directory,{recursive:true});const path=join(directory,'records.sqlite');
  const build=await sharedBuildMetadata();const started=performance.now();const workers=Array.from({length:config.writers},()=>new WorkerClient(storageCommand(config.engine,30000)));let control:SharedDatabase|undefined;let reader:SharedDatabase|undefined;
  const batches:StorageBatch[]=Array.from({length:config.batches},(_,i)=>({batchId:`batch-${i}`,seed:config.seed+i*10007,rows:config.batchRows,payloadBytes:config.payloadBytes}));let reading:Promise<void>|undefined;let stopped=false;let readerTimer:NodeJS.Timeout|undefined;const reads:{elapsedMs:number;rows:number}[]=[];const acks:{batchId:string;elapsedMs:number;rpcMs:number;metrics:unknown}[]=[];
  const sampler=sampleRuntimeResources(process.pid,100,{existingRoot:true});await sampler.ready();
  try{
    // Establish schema and WAL before concurrent connections upgrade a new DB.
    control=new SharedDatabase(path);
    await Promise.all(workers.map(async w=>{await w.call('prepare',{path,format:'sqlite-wal-full',busyTimeoutMs:5000});await checkSharedInfo(await w.call('hello',{}) as Record<string,unknown>,build,'sqlite-wal-full',5000)}));
    reader=config.readerMode==='none'?undefined:new SharedDatabase(path);let oldSnapshotRows:number|null=null;
    if(config.readerMode==='held'){reader!.exec('BEGIN');oldSnapshotRows=Number(reader!.prepare('SELECT COUNT(*) n FROM records').get()!.n)}
    const startupMs=performance.now()-started;const writeBegan=performance.now();
    if(config.readerMode==='short'){
      const tick=()=>{if(reading||stopped)return;reading=(async()=>{reader!.exec('BEGIN');const count=Number(reader!.prepare('SELECT COUNT(*) n FROM records').get()!.n);reads.push({elapsedMs:performance.now()-writeBegan,rows:count});await new Promise(resolve=>setTimeout(resolve,2));reader!.exec('COMMIT')})().finally(()=>{reading=undefined})};readerTimer=setInterval(tick,10);tick();
    }
    let next=0;
    await Promise.all(Array.from({length:8},(_,producer)=>async()=>{while(next<batches.length){const batch=batches[next++]!;const began=performance.now();const output=await workers[producer%workers.length]!.call('run',batch) as {status:string;rows:number;metrics:unknown};if(output.status!=='ok'||output.rows!==batch.rows)throw new Error('invalid writer acknowledgement');acks.push({batchId:batch.batchId,elapsedMs:performance.now()-writeBegan,rpcMs:performance.now()-began,metrics:output.metrics})}}).map(producer=>producer()));
    const writeMs=performance.now()-writeBegan;stopped=true;if(readerTimer)clearInterval(readerTimer);await reading;
    if(config.readerMode==='held')oldSnapshotRows=Number(reader!.prepare('SELECT COUNT(*) n FROM records').get()!.n);
    const walBeforeReleaseBytes=await stat(`${path}-wal`).then(s=>s.size).catch(()=>0);
    const checkpointBegan=performance.now();const passiveCheckpoint=control.prepare('PRAGMA wal_checkpoint(PASSIVE)').get()!;const passiveCheckpointMs=performance.now()-checkpointBegan;
    if(config.readerMode==='held')reader!.exec('COMMIT');reader?.close();reader=undefined;
    const truncateBegan=performance.now();const truncateCheckpoint=control.prepare('PRAGMA wal_checkpoint(TRUNCATE)').get()!;const truncateCheckpointMs=performance.now()-truncateBegan;const walAfterTruncateBytes=await stat(`${path}-wal`).then(s=>s.size).catch(()=>0);
    const drainBegan=performance.now();control.close();control=undefined;await Promise.all(workers.map(w=>w.close()));const drainMs=performance.now()-drainBegan;const resources=await sampler.stop();
    const oracleBegan=performance.now();const verdict=await verifyStorage(path,'sqlite-wal-full',batches);const oracleMs=performance.now()-oracleBegan;
    if(config.readerMode==='held'&&(oldSnapshotRows!==0||Number(passiveCheckpoint.log)<=Number(passiveCheckpoint.checkpointed)))throw new Error('held snapshot did not prevent checkpoint progress');
    if(truncateCheckpoint.busy!==0||truncateCheckpoint.log!==0||truncateCheckpoint.checkpointed!==0||walAfterTruncateBytes!==0)throw new Error('checkpoint did not truncate after readers released');
    if(reads.some((r,i)=>r.rows>config.batches*config.batchRows||(i>0&&r.rows<reads[i-1]!.rows)))throw new Error('non-monotonic reader publication');
    const result={config,buildSha256:build.hashes[sqliteLibrary],verdict,startupMs,writeMs,passiveCheckpointMs,truncateCheckpointMs,drainMs,oracleMs,totalMs:performance.now()-started,verifiedRowsPerSecond:verdict.status==='verified'?verdict.persistedRows*1000/writeMs:0,ackLatencyMs:quantiles(acks.map(a=>a.rpcMs)),oldSnapshotRows,passiveCheckpoint,truncateCheckpoint,walBeforeReleaseBytes,walAfterTruncateBytes,resources,readerSnapshots:reads.length};
    await writeFile(join(directory,'summary.json'),JSON.stringify(result,null,2)+'\n');await writeFile(join(directory,'acks.jsonl'),acks.map(a=>JSON.stringify(a)).join('\n')+'\n');await writeFile(join(directory,'reads.jsonl'),reads.map(r=>JSON.stringify(r)).join('\n')+'\n');return {...result,directory};
  }finally{stopped=true;if(readerTimer)clearInterval(readerTimer);await reading?.catch(()=>{});if(reader){try{if(config.readerMode==='held')reader.exec('ROLLBACK')}catch{}reader.close()}control?.close();await Promise.all(workers.map(w=>w.close()));await sampler.stop()}
}
