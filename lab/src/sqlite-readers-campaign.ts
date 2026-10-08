import {mkdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {randomBytes} from 'node:crypto';
import {runReaderExperiment,type ReaderConfig} from './sqlite-readers.ts';
import {labRoot} from './registry.ts';
import {environment,sourceFingerprint,copySourceSnapshot} from './environment.ts';
import {sharedBuildMetadata} from './storage-engines.ts';
import {shuffle} from './statistics.ts';
export type ReadersConfig={blocks:number;seed:number;engines:ReaderConfig['engine'][];writers:number[];readerModes:ReaderConfig['readerMode'][];batchRows:number[];totalRows:number;payloadBytes:number};
export const defaultReaders=(blocks=3):ReadersConfig=>({blocks,seed:8877,engines:['node-shared','rust-sync','go-sync'],writers:[1,4],readerModes:['none','short','held'],batchRows:[100,1000],totalRows:32000,payloadBytes:512});
export function readersMatrix(config:ReadersConfig):Omit<ReaderConfig,'seed'|'directory'>[]{return config.engines.flatMap(engine=>config.writers.flatMap(writers=>config.readerModes.flatMap(readerMode=>config.batchRows.map(batchRows=>({engine,writers,readerMode,batchRows,batches:config.totalRows/batchRows,payloadBytes:config.payloadBytes})))))}
// One readers/checkpoint cell on the shared SQLite artifact.
export async function runReadersCell(config:ReadersConfig,index:number,block:number,directory:string):Promise<Record<string,unknown>>{
  const cell=readersMatrix(config)[index]!;
  try{const run=await runReaderExperiment({...cell,seed:config.seed+block*100003,directory:join(directory,`cell-${index}-b${block}`)});return {...run,directory:`cell-${index}-b${block}`,block,index,status:run.verdict.status}}catch(error){return {...cell,block,index,status:'failed',error:String(error)}}
}
export async function readersCampaign(blocks=3,config:ReadersConfig=defaultReaders(blocks)){
  const id=`sqlite-readers-${new Date().toISOString().replace(/[:.]/g,'-')}-${randomBytes(3).toString('hex')}`;const directory=join(labRoot,'results',id);await mkdir(directory,{recursive:true});
  const env=await environment();const build=await sharedBuildMetadata();await copySourceSnapshot(directory,env.sourceHashes);
  const matrix=readersMatrix(config);
  await writeFile(join(directory,'manifest.json'),JSON.stringify({id,blocks:config.blocks,matrix,environment:env,build,methodology:'same SQLite shared artifact; WAL FULL; eight producers; real writer processes on one DB; reader none, short 2ms transactions every 10ms, or snapshot held from before first insert; PASSIVE checkpoint while held, TRUNCATE after release; checkpoint timings and WAL bytes explicit; every record independently reopened and verified; startup/drain/oracle separate; short writing windows are not sustained throughput evidence'},null,2)+'\n');
  const cells:Record<string,unknown>[]=[];
  for(let block=0;block<config.blocks;block++)for(const index of shuffle(matrix.map((_,i)=>i),config.seed+block)){
    const cell=matrix[index]!;cells.push(await runReadersCell(config,index,block,directory));
    await writeFile(join(directory,'summary.json'),JSON.stringify({cells},null,2)+'\n');process.stderr.write(`sqlite/${cell.engine}/W${cell.writers}/${cell.readerMode}/B${cell.batchRows}/b${block}: ${cells.at(-1)!.status}\n`);
  }
  const sourceChanges=Object.entries(await sourceFingerprint()).filter(([file,hash])=>env.sourceHashes[file]!==hash).map(([file])=>file);const current=await sharedBuildMetadata();const binaryChanged=current.hashes['libautoman_sqlite.dylib']!==build.hashes['libautoman_sqlite.dylib'];
  await writeFile(join(directory,'summary.json'),JSON.stringify({cells,sourceChanges,binaryChanged},null,2)+'\n');return {directory,cells,sourceChanges,binaryChanged};
}
if(process.argv[1]?.endsWith('sqlite-readers-campaign.ts')){const result=await readersCampaign(Number(process.argv[2]??3));process.stdout.write(JSON.stringify({directory:result.directory,verified:result.cells.filter(c=>c.status==='verified').length,total:result.cells.length,sourceChanges:result.sourceChanges,binaryChanged:result.binaryChanged})+'\n');if(result.cells.some(c=>c.status!=='verified')||result.sourceChanges.length||result.binaryChanged)process.exitCode=1}
