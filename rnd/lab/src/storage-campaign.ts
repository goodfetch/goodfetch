import { mkdir, appendFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { randomBytes } from 'node:crypto';
import { cpus, arch, release, totalmem } from 'node:os';
import { WorkerClient } from './worker-client.ts';
import { labRoot } from './registry.ts';
import { schedule } from './scheduler.ts';
import { summarize, shuffle, quantiles } from './statistics.ts';
import { sampleProcessGroup } from './resources.ts';
import { hashFile, sourceFingerprint, copySourceSnapshot } from './environment.ts';
import {storageEngines,storageCommand,isSharedEngine,sharedBuildMetadata,checkSharedInfo,type StorageEngine} from './storage-engines.ts';
import { storageFormats, verifyStorage, type StorageFormat, type StorageBatch } from './storage.ts';

export type StorageConfig = { name: string; engines: StorageEngine[]; formats: StorageFormat[]; blocks: number; batches: number; batchSizes: number[]; payloadBytes: number; writers: number[]; producers: number; timeoutMs: number; maxDurationMs: number; busyTimeoutMs: number; seed: number; killAfterAck?: boolean };
export function validateStorageConfig(c: StorageConfig) {
  if (!c || !/^[a-z0-9-]{1,80}$/.test(c.name)) throw new Error('invalid storage campaign');
  for (const [values, allowed] of [[c.engines,storageEngines], [c.formats,storageFormats]] as const) if (!Array.isArray(values) || !values.length || !values.every(v => (allowed as readonly string[]).includes(v)) || new Set(values).size !== values.length) throw new Error('invalid storage engines/formats');
  for (const [key, min, max] of [['blocks',1,20],['batches',1,10000],['payloadBytes',0,65536],['producers',1,128],['timeoutMs',100,120000],['maxDurationMs',100,3600000],['busyTimeoutMs',0,60000],['seed',0,2**32-1]] as const) if (!Number.isInteger(c[key]) || c[key] < min || c[key] > max) throw new Error(`invalid ${key}`);
  for (const [values, max] of [[c.batchSizes,10000],[c.writers,16]] as const) if (!Array.isArray(values) || !values.length || !values.every(v => Number.isInteger(v) && v > 0 && v <= max) || new Set(values).size !== values.length) throw new Error('invalid batch sizes/writers');
  if (c.batches * Math.max(...c.batchSizes) * (c.payloadBytes + 128) > 64 * 1024 * 1024) throw new Error('storage oracle corpus exceeds 64 MiB budget; split campaign');
  if (c.engines.length*c.formats.length*c.batchSizes.length*c.writers.length*c.blocks > 256) throw new Error('storage matrix exceeds 256 cells');
  if (c.killAfterAck !== undefined && typeof c.killAfterAck !== 'boolean') throw new Error('invalid crash probe');
}
type Attempt = { index: number; batch: StorageBatch; ack: 'acknowledged'|'error'|'rejected'; elapsedMs: number|null; queueMs: number; error?: string; metrics?: Record<string,number> };
export async function runStorageCampaign(config: StorageConfig, outputRoot = join(labRoot,'results')) {
  validateStorageConfig(config);
  const directory = join(outputRoot, `${config.name}-${new Date().toISOString().replace(/[:.]/g,'-')}-${randomBytes(3).toString('hex')}`);
  await mkdir(directory,{recursive:true});
  const sourceHashes=await sourceFingerprint();
  await copySourceSnapshot(directory,sourceHashes);
  const sharedBuild=config.engines.some(isSharedEngine)?await sharedBuildMetadata():null;
  await writeFile(join(directory,'manifest.json'),JSON.stringify({schemaVersion:1,config,startedAt:new Date().toISOString(),environment:{sharedBuild,node:process.version,nodeExecutable:process.execPath,arch:arch(),os:release(),cpu:cpus()[0]?.model,totalMemoryBytes:totalmem(),lockHash:await hashFile(join(labRoot,'adapters/storage-rust/Cargo.lock')),writerBinarySha256:config.engines.includes('rust')?await hashFile(join(labRoot,'adapters/rust/target/http/release/automan-lab-storage')):null,sourceHashes},methodology:{oneConnectionPerWriterProcess:true,oneSharedSinkPerCell:true,freshStorePerCell:true,admission:'closed loop; bounded producers',sync:'JSONL durable = file fsync; JSON snapshot = fsync+rename+directory fsync; SQLite explicit PRAGMAs',timing:'startup excluded; ack IPC+encoding+sink included; drain/reopen/oracle separately',killAfterAck:'SIGKILL after all acknowledgements; process crash only; no power-loss proof',scope:'pipeline with common JSON encoding, then native sink; not a pure language comparison'}},null,2)+'\n');
  const cells: Record<string,unknown>[] = [];
  const shared:StorageCellShared={sharedBuild};
  for(let block=0;block<config.blocks;block++) for(const cell of shuffle(storageCombinations(config),config.seed+block)) {
    cells.push(await runStorageCell(config,cell,block,directory,shared));
    await writeFile(join(directory,'summary.json'),JSON.stringify({cells,limitations:['exploratory local measurements','SQLite versions/builds may differ; read info','OS page cache and physical power-loss are uncontrolled','JSON snapshot replaces the whole growing document; separate workload','no scraping included in this storage lane']},null,2)+'\n');
    process.stderr.write(`${cell.engine}-${cell.format}-b${cell.batchSize}-w${cell.writers}-r${block}: ${cells.at(-1)!.status}\n`);
  }
  if(sharedBuild){const endingBuild=await sharedBuildMetadata();if(JSON.stringify(endingBuild.hashes)!==JSON.stringify(sharedBuild.hashes))throw new Error('shared binaries changed during campaign; measurements invalid')}
  const sourceChanges=Object.entries(await sourceFingerprint()).filter(([path,hash])=>sourceHashes[path]!==hash).map(([path])=>path);
  await writeFile(join(directory,'summary.json'),JSON.stringify({cells,sourceChanges,limitations:['exploratory local measurements','SQLite versions/builds may differ; read info','OS page cache and physical power-loss are uncontrolled','JSON snapshot replaces the whole growing document; separate workload','no scraping included in this storage lane',...(sourceChanges.length?['source changed during campaign; rerun before interpreting performance']:[])]},null,2)+'\n');
  await writeStorageReport(directory,cells);
  return {directory,cells,sourceChanges};
}
export type StorageCell={engine:StorageEngine;format:StorageFormat;batchSize:number;writers:number};
export type StorageCellShared={sharedBuild:Awaited<ReturnType<typeof sharedBuildMetadata>>|null;commonIdentity?:string};
export function storageCombinations(config:StorageConfig):StorageCell[]{return config.engines.flatMap(engine=>config.formats.flatMap(format=>config.batchSizes.flatMap(batchSize=>config.writers.map(writers=>({engine,format,batchSize,writers})))))}
// One storage cell: fresh sink, real writer processes, acknowledgements, independent re-read oracle.
export async function runStorageCell(config:StorageConfig,cell:StorageCell,block:number,directory:string,shared:StorageCellShared):Promise<Record<string,unknown>>{
  const label=`${cell.engine}-${cell.format}-b${cell.batchSize}-w${cell.writers}-r${block}`;
    if(cell.format==='json-snapshot-durable' && cell.writers>1) return {...cell,block,status:'unsupported',reason:'whole-document replacement requires a single writer or interprocess coordination'};
    if(['postgres-copy','duckdb-table','parquet-batch'].includes(cell.format)&&cell.engine!=='node')return {...cell,block,status:'unsupported',reason:'the additional sinks are implemented in the Node storage worker'};
    if(cell.format==='duckdb-table'&&cell.writers>1)return {...cell,block,status:'unsupported',reason:'a DuckDB database file has one writing process'};
    if(isSharedEngine(cell.engine)&&!cell.format.startsWith('sqlite'))return {...cell,block,status:'unsupported',reason:'shared synchronous bindings support SQLite only'};
    const cellDirectory=join(directory,label); await mkdir(cellDirectory,{recursive:true});
    const path=join(cellDirectory,cell.format.startsWith('sqlite')?'records.sqlite':cell.format.startsWith('jsonl')?'records.jsonl':cell.format==='duckdb-table'?'records.duckdb':cell.format==='parquet-batch'?'parts':cell.format==='postgres-copy'?'postgres':'records.json');
    const workers: WorkerClient[]=[]; const samplers: ReturnType<typeof sampleProcessGroup>[]=[];
    const attempts: Attempt[]=[]; let flush=Promise.resolve(); const info: unknown[]=[];
    let finalized=false;
    try {
    let startupError:string|null=null; const startupStart=performance.now();
    try {
      for(let i=0;i<cell.writers;i++) { const worker=new WorkerClient(storageCommand(cell.engine,config.timeoutMs)); workers.push(worker); samplers.push(sampleProcessGroup(worker.pid));
        await worker.call('hello',{}); await worker.call('prepare',{path,format:cell.format,busyTimeoutMs:config.busyTimeoutMs}); const metadata=await worker.call('hello',{}) as Record<string,unknown>;if(isSharedEngine(cell.engine)){await checkSharedInfo(metadata,shared.sharedBuild!,cell.format,config.busyTimeoutMs);const identity=JSON.stringify([metadata.sourceId,metadata.compileOptions]);if(shared.commonIdentity&&identity!==shared.commonIdentity)throw new Error('SQLite source/options differ across shared runtimes');shared.commonIdentity=identity}info.push(metadata); }
    } catch(error) { startupError=error instanceof Error?error.message:String(error); }
    const startupMs=performance.now()-startupStart;
    const batch=(index:number):StorageBatch=>({batchId:`b${index}`,seed:config.seed+block*100003+index,rows:cell.batchSize,payloadBytes:config.payloadBytes});
    async function record(row:Attempt) { attempts.push(row); flush=flush.then(()=>appendFile(join(cellDirectory,'attempts.jsonl'),JSON.stringify(row)+'\n')); await flush; }
    const experimentStart=performance.now();
    try {
      if(!startupError) await schedule({mode:'closed',iterations:config.batches,concurrency:config.producers,maxDurationMs:config.maxDurationMs},async(index,queueMs)=>{
        const began=performance.now(); const input=batch(index);
        try { const result=await workers[index%workers.length]!.call('run',input) as {status:string;rows:number;metrics:Record<string,number>};
          if(result.status!=='ok'||result.rows!==cell.batchSize||!result.metrics||Object.values(result.metrics).some(v=>typeof v!=='number'||!Number.isFinite(v)||v<0)) throw new Error('invalid storage acknowledgement');
          await record({index,batch:input,ack:'acknowledged',elapsedMs:performance.now()-began,queueMs,metrics:result.metrics});
        } catch(error) { await record({index,batch:input,ack:'error',elapsedMs:performance.now()-began,queueMs,error:error instanceof Error?error.message:String(error)}); }
      },(index,queueMs)=>record({index,batch:batch(index),ack:'rejected',elapsedMs:null,queueMs}));
    } finally { await flush; }
    const writeMs=performance.now()-experimentStart; const finalizationStart=performance.now();
    if(config.killAfterAck) for(const worker of workers) { try { process.kill(process.platform==='win32'?worker.pid:-worker.pid,'SIGKILL'); } catch {} }
    await Promise.all(workers.map(worker=>worker.close()));
    finalized=true;
    const acknowledged=attempts.filter(a=>a.ack==='acknowledged');
    const verdict=await verifyStorage(path,cell.format,acknowledged.map(a=>a.batch));
    const samples=attempts.map(a=>({status:a.ack==='rejected'?'rejected' as const:a.ack==='acknowledged'&&verdict.status==='verified'?'verified' as const:'failed' as const,elapsedMs:a.elapsedMs}));
    const summary=summarize(samples);
    const verifiedRows=summary.verified*cell.batchSize;
    return {...cell,block,status:startupError||summary.failed||summary.rejected||verdict.status==='failed'?'failed':'verified',startupMs,startupError,info,writeMs,finalizationMs:performance.now()-finalizationStart,verdict,measurement:summary,serializationMs:quantiles(acknowledged.map(a=>a.metrics!.serializationMs!)),verifiedRowsPerSecond:verdict.status==='verified'&&writeMs>0?verifiedRows*1000/writeMs:null,resources:samplers.map(s=>s.stop()),stderr:workers.map(w=>w.stderr)};
    } finally { if(!finalized) await Promise.allSettled(workers.map(worker=>worker.close())); for(const sampler of samplers) sampler.stop(); }

}

async function writeStorageReport(directory:string,cells:Record<string,unknown>[]) {
  const esc=(v:unknown)=>String(v).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
  const rows=cells.map(c=>{const m=c.measurement as ReturnType<typeof summarize>|undefined; const info=c.info as {sqlite?:string}[]|undefined; return `<tr>${[c.engine,c.format,c.format && String(c.format).startsWith('sqlite') ? info?.[0]?.sqlite??'—':'—',c.batchSize,c.writers,c.block,c.status,m?`${m.verified}/${m.attempts}`:'—',typeof c.verifiedRowsPerSecond==='number'?c.verifiedRowsPerSecond.toFixed(1):'—',m?.successLatencyMs.p95?.toFixed(2)??'—'].map(v=>`<td>${esc(v)}</td>`).join('')}</tr>`}).join('');
  await writeFile(join(directory,'report.html'),`<!doctype html><html lang="en"><meta charset="utf-8"><title>Automan storage</title><style>body{font:15px system-ui;margin:32px;background:#f8fafc}table{border-collapse:collapse;background:white}td,th{padding:10px;border-bottom:1px solid #ddd;text-align:left}</style><h1>Writes: JSON / JSONL / SQLite</h1><p>Exploratory results per block. Throughput of verified rows, after close and an independent oracle. Durability contracts differ; no proof of survival after a power cut. Several producers share one sink; W counts the real writer processes. The node-shared, rust-sync and go-sync engines load the same SQLite artifact, checked by SHA-256, source ID, options and PRAGMAs. The other tracks have their own builds. Bindings, allocations and encoders still differ: no figure proves an advantage of the language alone.</p><table><thead><tr>${['Runtime','Contract','SQLite','Batch','W','Block','Status','Verified batches','Rows/s','p95 ack ms'].map(h=>`<th>${h}</th>`).join('')}</tr></thead><tbody>${rows}</tbody></table><p><a href="manifest.json">Manifest</a> · <a href="summary.json">Details, PRAGMAs, oracle and resources</a></p></html>\n`);
}
