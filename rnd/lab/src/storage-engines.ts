import { sqliteLibrary } from './registry.ts';
import {join} from 'node:path';
import {labRoot} from './registry.ts';
import type {WorkerCommand} from './worker-client.ts';
import {readFile,realpath} from 'node:fs/promises';
import {hashFile} from './environment.ts';
export const storageEngines=['node','rust','node-shared','rust-sync','go-sync'] as const;
export type StorageEngine=typeof storageEngines[number];
export const isSharedEngine=(engine:StorageEngine)=>['node-shared','rust-sync','go-sync'].includes(engine);
export function storageCommand(engine:StorageEngine,timeoutMs:number):WorkerCommand{
  if(engine==='node')return {command:process.execPath,args:[join(labRoot,'adapters/storage-node/worker.ts')],cwd:labRoot,timeoutMs};
  if(engine==='rust')return {command:join(labRoot,'adapters/rust/target/http/release/automan-lab-storage'),args:[],cwd:labRoot,timeoutMs};
  if(engine==='node-shared')return {command:process.execPath,args:[join(labRoot,'adapters/storage-shared-node/worker.ts')],cwd:labRoot,timeoutMs};
  return {command:join(labRoot,'.native',engine==='rust-sync'?'automan-storage-rust-sync':'automan-storage-go-sync'),args:[],cwd:labRoot,timeoutMs};
}
export async function sharedBuildMetadata(){
  const build=JSON.parse(await readFile(join(labRoot,'.native/build.json'),'utf8')) as {sqlite:string;libraryPath:string;hashes:Record<string,string>;inputHashes:Record<string,string>;[key:string]:unknown};
  if(build.sqlite!=='3.53.4'||await realpath(build.libraryPath)!==await realpath(join(labRoot,`.native/${sqliteLibrary}`)))throw new Error('unexpected shared SQLite build');
  for(const file of [sqliteLibrary,'sqlite.node','automan-storage-rust-sync','automan-storage-go-sync'])if(await hashFile(join(labRoot,'.native',file))!==build.hashes[file])throw new Error(`shared artifact changed: ${file}; rebuild before measuring`);
  if(!build.inputHashes)throw new Error('shared build lacks source provenance; rebuild before measuring');
  for(const [file,hash]of Object.entries(build.inputHashes))if(await hashFile(join(labRoot,file))!==hash)throw new Error(`native source changed: ${file}; rebuild before measuring`);
  return build;
}
export async function checkSharedInfo(info:Record<string,unknown>,build:Awaited<ReturnType<typeof sharedBuildMetadata>>,format:string,busyTimeoutMs:number){
  if(info.sqlite!==build.sqlite||typeof info.libraryPath!=='string'||await realpath(info.libraryPath)!==await realpath(build.libraryPath)||info.sourceId!==build.sqliteSourceId||!Array.isArray(info.compileOptions))throw new Error('loaded SQLite artifact differs');
  if(await hashFile(info.libraryPath)!==build.hashes[sqliteLibrary])throw new Error('loaded SQLite artifact changed during campaign');
  if(info.journal_mode!==(format.includes('wal')?'wal':'delete')||info.synchronous!==(format.includes('normal')?1:2)||info.busy_timeout!==busyTimeoutMs||info.wal_autocheckpoint!==1000||info.page_size!==4096)throw new Error('shared SQLite effective contract differs');
}
