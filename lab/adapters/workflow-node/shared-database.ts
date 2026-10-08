import {createRequire} from 'node:module';
import {join} from 'node:path';
import {labRoot} from '../../src/registry.ts';
type Native={open:(path:string,format:string,busy:number)=>unknown;query:(handle:unknown,sql:string,params:string)=>string;exec:(handle:unknown,sql:string)=>void;info:(handle:unknown)=>string;close:(handle:unknown)=>void};
const native=createRequire(import.meta.url)(join(labRoot,'.native/sqlite.node')) as Native;
export class SharedDatabase {
  private handle:unknown;
  constructor(path:string){this.handle=native.open(path,'sqlite-wal-full',5000)}
  exec(sql:string){native.exec(this.handle,sql)}
  info(){return JSON.parse(native.info(this.handle)) as Record<string,unknown>}
  prepare(sql:string){
    const all=(...params:unknown[])=>JSON.parse(native.query(this.handle,sql,JSON.stringify(params))) as Record<string,unknown>[];
    return {all,get:(...params:unknown[])=>all(...params)[0],run:(...params:unknown[])=>{all(...params)}};
  }
  close(){native.close(this.handle)}
}
