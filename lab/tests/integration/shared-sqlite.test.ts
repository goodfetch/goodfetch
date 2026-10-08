import { sqliteLibrary } from '../../src/registry.ts';
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {WorkerClient} from '../../src/worker-client.ts';
import {storageCommand,sharedBuildMetadata,checkSharedInfo} from '../../src/storage-engines.ts';
import {verifyStorage} from '../../src/storage.ts';

test('Node, synchronous Rust and Go use one SQLite artifact and roll back a duplicate batch',async()=>{
  const build=await sharedBuildMetadata();
  const infos:{libraryPath:string;sqlite:string;sourceId:string;compileOptions:unknown}[]=[];
  for(const engine of ['node-shared','rust-sync','go-sync'] as const){
    const path=join(await mkdtemp(join(tmpdir(),`automan-${engine}-`)),'records.sqlite');const w=new WorkerClient(storageCommand(engine,10000));
    try{await w.call('prepare',{path,format:'sqlite-wal-full',busyTimeoutMs:5000});const info=await w.call('hello',{}) as typeof infos[number];infos.push(info);assert.equal(info.sqlite,'3.53.4');await checkSharedInfo(info,build,'sqlite-wal-full',5000);await assert.rejects(checkSharedInfo({...info,synchronous:1},build,'sqlite-wal-full',5000),/contract differs/);
      const batch={batchId:'b0',seed:7,rows:2,payloadBytes:3};await w.call('run',batch);await assert.rejects(w.call('run',batch),/UNIQUE|constraint/);
      await assert.rejects(checkSharedInfo(info,{...build,hashes:{...build.hashes,[sqliteLibrary]:'0'.repeat(64)}},'sqlite-wal-full',5000),/artifact changed/);
    }finally{await w.close()}
    const verdict=await verifyStorage(path,'sqlite-wal-full',[{batchId:'b0',seed:7,rows:2,payloadBytes:3}]);assert.equal(verdict.status,'verified',verdict.reasons.join('; '));
  }
  for(const info of infos.slice(1)){assert.equal(info.libraryPath,infos[0]!.libraryPath);assert.equal(info.sourceId,infos[0]!.sourceId);assert.deepEqual(info.compileOptions,infos[0]!.compileOptions)}
});
