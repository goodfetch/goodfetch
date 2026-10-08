import {createRequire} from 'node:module';
import {join} from 'node:path';
import {labRoot} from '../../src/registry.ts';
import {serve,type Adapter} from '../../src/worker-server.ts';
import type {StorageBatch} from '../../src/storage.ts';
type Native={open(path:string,format:string,busy:number):object;begin(db:object):void;insert(db:object,id:string,value:number,payload:string):void;commit(db:object):void;rollback(db:object):void;close(db:object):void;info(db?:object):string};
const native=createRequire(import.meta.url)(join(labRoot,'.native/sqlite.node')) as Native;
let db:object|undefined;
const adapter:Adapter<StorageBatch,{path:string;format:string;busyTimeoutMs:number}>={
  info(){return {backend:'storage-node-shared',runtime:process.version,binding:'synchronous Node-API; one common C call per inserted row',...JSON.parse(db?native.info(db):native.info())}},
  async prepare(input){if(db)throw new Error('already prepared');db=native.open(input.path,input.format,input.busyTimeoutMs)},
  async run(batch){
    if(!db)throw new Error('not prepared');const began=performance.now();const payload='x'.repeat(batch.payloadBytes);
    const rows=Array.from({length:batch.rows},(_,i)=>({id:`${batch.batchId}:${i}`,value:batch.seed+i,payload}));const json=rows.map(r=>JSON.stringify(r)).join('\n')+'\n';const serializationMs=performance.now()-began;
    native.begin(db);try{for(const r of rows)native.insert(db,r.id,r.value,r.payload);native.commit(db)}catch(error){try{native.rollback(db)}catch{}throw error}
    return {status:'ok',rows:rows.length,metrics:{serializationMs,writerAckMs:performance.now()-began,encodedBytes:Buffer.byteLength(json)}};
  },
  async close(){if(db){native.close(db);db=undefined}}
};serve(adapter);
