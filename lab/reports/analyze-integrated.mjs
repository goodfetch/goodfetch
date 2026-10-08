// Reproducible analysis of preserved evidence; does not execute a benchmark.
import {readFile,writeFile,readdir} from 'node:fs/promises';
import {resolve,join,relative,basename} from 'node:path';
import {createHash} from 'node:crypto';
const root=resolve(import.meta.dirname,'..');
const suite=resolve(process.argv[2]??'');
if(!process.argv[2]||!suite.startsWith(join(root,'results')+'/'))throw Error('Pass a suite directory under lab/results');
const read=async path=>JSON.parse(await readFile(path,'utf8'));
const q=values=>{const v=values.filter(Number.isFinite).sort((a,b)=>a-b);const at=p=>v.length?v[Math.ceil(p*v.length)-1]:null;return {n:v.length,p50:at(.5),p95:at(.95),p99:at(.99),min:v[0]??null,max:v.at(-1)??null}};
const sum=(rows,f)=>rows.reduce((s,r)=>s+Number(f(r)??0),0);
const group=(rows,key)=>{const g=new Map();for(const r of rows){const k=key(r);if(!g.has(k))g.set(k,[]);g.get(k).push(r)}return [...g]};
const status=rows=>Object.fromEntries(group(rows,r=>r.status).map(([k,v])=>[k,v.length]));
const lines=async path=>(await readFile(path,'utf8')).split('\n').filter(Boolean).map(line=>JSON.parse(line));
const ledger={};
async function runtime(directory,summary){
  const cells=summary.cells;
  const out=[];
  for(const [key,rows]of group(cells,r=>[r.runtime,r.backend??'native',r.concurrency,r.batchJobs,r.fault??(r.faults?.join('+')??r.coordinatorCrashAt??'none'),r.writerDelayMs??0].join('/'))){
    const good=rows.filter(r=>r.status==='verified');const raw=[];const batchSizes=[];const commits=[];const ackTransactions=[];
    for(const cell of good){
      const cellDir=join(directory,`cell-${cell.index}-b${cell.block}`);
      const events=[];for(const file of (await readdir(cellDir)).filter(f=>/^execution-\d+-events.jsonl$/.test(f)))events.push(...await lines(join(cellDir,file)));
      raw.push(...events.filter(e=>['job-acked','reconciled'].includes(e.type)&&Number.isFinite(e.endToEndMs)).map(e=>e.endToEndMs));
      for(const event of events.filter(e=>e.type==='batch-acked')){batchSizes.push(event.jobs);commits.push(event.commitMs);ackTransactions.push(event.ackMs)}
    }
    const work=sum(good,r=>r.workMs+r.finalizationMs),wall=sum(good,r=>r.totalVerifiedWallMs),jobs=sum(good,r=>r.verifiedJobs),cpu=sum(good,r=>r.resources.sampledUserCpuMs+r.resources.sampledSystemCpuMs);
    out.push({key,config:rows[0]&&Object.fromEntries(['runtime','backend','concurrency','batchJobs','jobs','pages','delayMs','fault','faults','writerDelayMs','coordinatorCrashAt'].filter(k=>rows[0][k]!==undefined).map(k=>[k,rows[0][k]])),statuses:status(rows),jobs,records:sum(good,r=>r.verifiedRecords),workAndFinalizationMs:work,completeWallMs:wall,weightedJobsPerSecond:work?jobs*1000/work:null,completeJobsPerSecond:wall?jobs*1000/wall:null,blockJobsPerSecond:q(good.map(r=>r.verifiedJobsPerSecond)),cpuMsPerJob:jobs?cpu/jobs:null,cpuTotalMs:cpu,peakFootprintMiB:q(good.map(r=>r.resources.peakFootprintBytes/1048576)),peakRssMiB:q(good.map(r=>r.resources.peakRssBytes/1048576)),sampleCounts:good.map(r=>r.resources.samples),observedProcessCounts:good.map(r=>r.resources.observedProcessCount),diskWriteBytesPerJob:jobs?sum(good,r=>r.resources.sampledDiskWriteBytes)/jobs:null,ackLatencyMs:q(raw),batchJobs:q(batchSizes),commitMs:q(commits),ackTransactionMs:q(ackTransactions),coordinatorRecoveryMs:q(good.map(r=>r.coordinatorRecoveryMs)),restarts:sum(good,r=>r.restarts),coordinatorRestarts:sum(good,r=>r.coordinatorRestarts),maxBufferedJobs:Math.max(0,...good.map(r=>r.peakBufferedJobs)),blocks:good.map(r=>r.block),failures:rows.filter(r=>r.status!=='verified').map(r=>({block:r.block,error:r.error}))});
  }
  const paired=[];
  for(const rust of out.filter(g=>g.config.runtime==='rust'&&!g.config.backend)){
    const node=out.find(g=>g.config.runtime==='node'&&!g.config.backend&&g.config.concurrency===rust.config.concurrency&&g.config.batchJobs===rust.config.batchJobs&&g.config.fault===rust.config.fault&&JSON.stringify(g.config.faults)===JSON.stringify(rust.config.faults)&&g.config.coordinatorCrashAt===rust.config.coordinatorCrashAt);
    if(!node)continue;
    const rs=cells.filter(c=>c.runtime==='rust'&&!c.backend&&c.concurrency===rust.config.concurrency&&c.batchJobs===rust.config.batchJobs&&c.status==='verified'&&c.fault===rust.config.fault&&JSON.stringify(c.faults)===JSON.stringify(rust.config.faults)&&c.coordinatorCrashAt===rust.config.coordinatorCrashAt);
    const ns=cells.filter(c=>c.runtime==='node'&&!c.backend&&c.concurrency===rust.config.concurrency&&c.batchJobs===rust.config.batchJobs&&c.status==='verified'&&c.fault===rust.config.fault&&JSON.stringify(c.faults)===JSON.stringify(rust.config.faults)&&c.coordinatorCrashAt===rust.config.coordinatorCrashAt);
    paired.push({rustKey:rust.key,nodeKey:node.key,weightedRateRatio:rust.weightedJobsPerSecond/node.weightedJobsPerSecond,completeRateRatio:rust.completeJobsPerSecond/node.completeJobsPerSecond,cpuMsPerJobRatio:rust.cpuMsPerJob/node.cpuMsPerJob,pairedBlocks:rs.flatMap(r=>{const n=ns.find(n=>n.block===r.block);return n?[{block:r.block,rateRatio:r.verifiedJobsPerSecond/n.verifiedJobsPerSecond,completeRateRatio:r.jobsPerSecondIncludingStartupAndOracle/n.jobsPerSecondIncludingStartupAndOracle}]:[]})});
  }
  return {statuses:status(cells),jobs:sum(cells,r=>r.verifiedJobs),records:sum(cells,r=>r.verifiedRecords),groups:out,pairedNativeComparisons:paired};
}
async function sqlite(directory,summary){
  const out=[];
  for(const [key,rows]of group(summary.cells,r=>r.config?[r.config.engine,r.config.writers,r.config.readerMode,r.config.batchRows].join('/'):[r.engine,r.writers,r.readerMode,r.batchRows].join('/'))){
    const good=rows.filter(r=>r.status==='verified');const acks=[];
    for(const r of good)acks.push(...(await lines(join(directory,r.directory,'acks.jsonl'))).map(r=>r.rpcMs));
    const count=sum(good,r=>r.verdict.persistedRows),write=sum(good,r=>r.writeMs);
    out.push({key,statuses:status(rows),rows:count,weightedRowsPerSecond:write?count*1000/write:null,writingWindowMs:q(good.map(r=>r.writeMs)),completeRowsPerSecond:count*1000/sum(good,r=>r.totalMs),ackLatencyMs:q(acks),passiveCheckpointMs:q(good.map(r=>r.passiveCheckpointMs)),truncateCheckpointMs:q(good.map(r=>r.truncateCheckpointMs)),walMiB:q(good.map(r=>r.walBeforeReleaseBytes/1048576)),checkpoints:good.map(r=>({block:r.block,snapshot:r.oldSnapshotRows,passive:r.passiveCheckpoint,truncate:r.truncateCheckpoint,walAfter:r.walAfterTruncateBytes,readerSnapshots:r.readerSnapshots})),cpuMsPerRow:count?sum(good,r=>r.resources.sampledUserCpuMs+r.resources.sampledSystemCpuMs)/count:null,peakFootprintMiB:q(good.map(r=>r.resources.peakFootprintBytes/1048576)),failures:rows.filter(r=>r.status!=='verified').map(r=>({block:r.block,error:r.error}))});
  }
  return {statuses:status(summary.cells),rows:sum(summary.cells,r=>r.verdict?.persistedRows),groups:out};
}
function browser(summary){
  return {statuses:status(summary.cells),groups:group(summary.cells,r=>r.backend+'/'+r.policy).map(([key,rows])=>({key,statuses:status(rows),variants:group(rows,r=>r.variant).map(([variant,rs])=>({variant,statuses:status(rs),taskMs:q(rs.filter(r=>r.status==='verified').map(r=>r.taskMs)),startupMs:q(rs.map(r=>r.startupMs)),clickMs:q(rs.filter(r=>r.status==='verified').map(r=>r.metrics?.nativeClickMs)),rows:rs.map(r=>({block:r.block,status:r.status,reasons:r.reasons,metrics:r.metrics,events:r.events,effects:r.effects,effectDetails:r.effectDetails}))}))}))};
}
async function crawl(directory,summary){
  const raw=await lines(join(directory,'attempts.jsonl'));
  const measured=raw.filter(r=>r.phase==='measure');
  return {summary,statuses:status(raw),measureStatuses:status(measured),rawCount:raw.length,measureCount:measured.length,measurementGroups:group(measured,r=>r.backend+'/'+r.scenario).map(([key,rows])=>({key,statuses:status(rows),successLatencyMs:q(rows.filter(r=>r.status==='verified').map(r=>r.elapsedMs)),startedLatencyMs:q(rows.filter(r=>r.elapsedMs!==null).map(r=>r.elapsedMs)),admissionMs:q(rows.filter(r=>r.queueMs!==null).map(r=>r.queueMs))}))};
}
let suiteSummary;try{suiteSummary=await read(join(suite,'summary.json'))}catch{suiteSummary={steps:[]}}
for(const step of suiteSummary.steps){
  if(!step.directory){ledger[step.name]={status:step.status,error:step.error};continue}
  const directory=step.directory;const summary=await read(join(directory,'summary.json'));
  const result=step.name.startsWith('runtime-')?await runtime(directory,summary):step.name==='sqlite-readers'?await sqlite(directory,summary):step.name==='browser-actionability'?browser(summary):await crawl(directory,summary);
  ledger[step.name]={directory:relative(root,directory),provenance:{sourceChanges:summary.sourceChanges??[],binaryChanges:summary.binaryChanges??[],binaryChanged:summary.binaryChanged??false},...result};
}
const additional=process.argv.slice(3);
const additionalSuites=[];
for(const directory of additional){
  const full=resolve(directory);const summary=await read(join(full,'summary.json'));const name=basename(full);
  if(summary.steps){additionalSuites.push(full);continue}
  const result=name.startsWith('runtime-')?await runtime(full,summary):name.startsWith('sqlite-readers-')?await sqlite(full,summary):name.startsWith('browser-actionability-')?browser(summary):await crawl(full,summary);
  ledger[name]={directory:relative(root,full),provenance:{sourceChanges:summary.sourceChanges??[],binaryChanges:summary.binaryChanges??[],binaryChanged:summary.binaryChanged??false},...result};
}
const result={suite:relative(root,suite),analyzedAt:new Date().toISOString(),suiteSummary,quantiles:'nearest rank from raw events, never averaged block percentiles',runtimeWeighting:'sum independently verified jobs / sum selected durations; complete wall includes process startup and external oracle; recovery timing uses outer complete wall',resourceCaveat:'sampled process families; endpoint counters may miss tails; peak footprint is per-cell sampled maximum; no CPU language isolation',campaigns:ledger};
await writeFile(join(root,'reports','integrated-analysis.json'),JSON.stringify(result,null,2)+'\n');
const catalogue=[];
const selected=[suite,...suiteSummary.steps.filter(s=>s.directory).map(s=>s.directory),...additional.map(d=>resolve(d))];
for(const directory of selected){
  async function walk(path){for(const entry of await readdir(path,{withFileTypes:true})){const full=join(path,entry.name);if(entry.isDirectory()){await walk(full);continue}if(!entry.isFile()||entry.name==='input.json'||/\.sqlite(?:-wal|-shm)?$/.test(entry.name)||entry.name.endsWith('-persisted.json'))continue;const bytes=await readFile(full);catalogue.push({path:relative(root,full),bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')})}}
  await walk(directory);
}
await writeFile(join(root,'reports','integrated-evidence-catalog.json'),JSON.stringify({suite:relative(root,suite),files:catalogue,totalBytes:sum(catalogue,r=>r.bytes),excluded:'SQLite databases/sidecars, private live oracle input credentials, bulky per-record persisted exports; retained in original results directories'},null,2)+'\n');
const byPath=new Map(catalogue.map(f=>[f.path,f]));const snapshotAudits=[];
for(const directory of selected){
  const manifest=await read(join(directory,'manifest.json'));const hashes=manifest.environment?.sourceHashes??{};const mismatches=[];let checked=0;
  for(const [file,hash]of Object.entries(hashes)){
    // The acquisition campaigns use the suite's identical frozen source snapshot.
    const own=byPath.get(relative(root,join(directory,'sources',file)));
    const common=byPath.get(relative(root,join(suite,'sources',file)));
    const matching=additionalSuites.map(s=>byPath.get(relative(root,join(s,'sources',file)))).find(f=>f?.sha256===hash);
    const actual=own?.sha256===hash?own:common?.sha256===hash?common:matching;if(actual?.sha256!==hash)mismatches.push(file);else checked++;
  }
  snapshotAudits.push({directory:relative(root,directory),checked,mismatches});
}
await writeFile(join(root,'reports','integrated-source-audit.json'),JSON.stringify({snapshots:snapshotAudits,method:'compare SHA-256 of every preserved source against each manifest; acquisition campaigns refer to a suite snapshot with an identical SHA-256 for each file'},null,2)+'\n');
console.log(JSON.stringify({campaigns:Object.keys(ledger),files:catalogue.length,bytes:sum(catalogue,r=>r.bytes),snapshotMismatches:sum(snapshotAudits,r=>r.mismatches.length)}));
if(snapshotAudits.some(a=>a.mismatches.length))process.exitCode=1;
