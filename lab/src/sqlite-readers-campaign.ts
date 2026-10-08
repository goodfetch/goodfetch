import {mkdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {randomBytes} from 'node:crypto';
import {runReaderExperiment,type ReaderConfig} from './sqlite-readers.ts';
import {labRoot} from './registry.ts';
import {environment,sourceFingerprint,copySourceSnapshot} from './environment.ts';
import {sharedBuildMetadata} from './storage-engines.ts';
import {shuffle} from './statistics.ts';
export async function readersCampaign(blocks=3){
  const id=`sqlite-readers-${new Date().toISOString().replace(/[:.]/g,'-')}-${randomBytes(3).toString('hex')}`;const directory=join(labRoot,'results',id);await mkdir(directory,{recursive:true});
  const env=await environment();const build=await sharedBuildMetadata();await copySourceSnapshot(directory,env.sourceHashes);
  const matrix:Omit<ReaderConfig,'seed'|'directory'>[]=['node-shared','rust-sync','go-sync'].flatMap(engine=>[1,4].flatMap(writers=>['none','short','held'].flatMap(readerMode=>[100,1000].map(batchRows=>({engine:engine as ReaderConfig['engine'],writers,readerMode:readerMode as ReaderConfig['readerMode'],batchRows,batches:32000/batchRows,payloadBytes:512})))));
  await writeFile(join(directory,'manifest.json'),JSON.stringify({id,blocks,matrix,environment:env,build,methodology:'same SQLite shared artifact; WAL FULL; eight producers; real writer processes on one DB; reader none, short 2ms transactions every 10ms, or snapshot held from before first insert; PASSIVE checkpoint while held, TRUNCATE after release; checkpoint timings and WAL bytes explicit; every record independently reopened and verified; startup/drain/oracle separate; short writing windows are not sustained throughput evidence'},null,2)+'\n');
  const cells:Record<string,unknown>[]=[];
  for(let block=0;block<blocks;block++)for(const index of shuffle(matrix.map((_,i)=>i),8877+block)){
    const config=matrix[index]!;try{const run=await runReaderExperiment({...config,seed:8877+block*100003,directory:join(directory,`cell-${index}-b${block}`)});cells.push({...run,directory:`cell-${index}-b${block}`,block,index,status:run.verdict.status})}catch(error){cells.push({...config,block,index,status:'failed',error:String(error)})}
    await writeFile(join(directory,'summary.json'),JSON.stringify({cells},null,2)+'\n');process.stderr.write(`sqlite/${config.engine}/W${config.writers}/${config.readerMode}/B${config.batchRows}/b${block}: ${cells.at(-1)!.status}\n`);
  }
  const sourceChanges=Object.entries(await sourceFingerprint()).filter(([file,hash])=>env.sourceHashes[file]!==hash).map(([file])=>file);const current=await sharedBuildMetadata();const binaryChanged=current.hashes['libautoman_sqlite.dylib']!==build.hashes['libautoman_sqlite.dylib'];
  await writeFile(join(directory,'summary.json'),JSON.stringify({cells,sourceChanges,binaryChanged},null,2)+'\n');return {directory,cells,sourceChanges,binaryChanged};
}
if(process.argv[1]?.endsWith('sqlite-readers-campaign.ts')){const result=await readersCampaign(Number(process.argv[2]??3));process.stdout.write(JSON.stringify({directory:result.directory,verified:result.cells.filter(c=>c.status==='verified').length,total:result.cells.length,sourceChanges:result.sourceChanges,binaryChanged:result.binaryChanged})+'\n');if(result.cells.some(c=>c.status!=='verified')||result.sourceChanges.length||result.binaryChanged)process.exitCode=1}
