import { DatabaseSync } from 'node:sqlite';
import { createHash, randomUUID } from 'node:crypto';
import { serve, type Adapter, type Task } from '../../src/worker-server.ts';

type RecordItem={id:string;price:number};
type Entry={jobId:string;token:string;records:RecordItem[]};
type Request={op:'submit';jobs:{jobId:string;task:Task}[]}|{op:'claim'}|{op:'commit';entries:Entry[];fault?:'before-commit'|'after-commit'}|{op:'ack';receipts:{jobId:string;receipt:string}[]}|{op:'reject';jobId:string;token:string;reason:string}|{op:'snapshot'};
type Job={jobId:string;task:string;state:string;token:string|null;expires:number;attempts:number;receipt:string|null;digest:string|null};
let db:DatabaseSync|undefined;let leaseMs=1000;
function database(){if(!db)throw new Error('not prepared');return db}
function transaction<T>(work:()=>T):T{const d=database();d.exec('BEGIN IMMEDIATE');try{const result=work();d.exec('COMMIT');return result}catch(error){d.exec('ROLLBACK');throw error}}
function getJob(id:string){return database().prepare('SELECT * FROM jobs WHERE jobId=?').get(id) as Job|undefined}
function canonical(records:RecordItem[]){
  if(!Array.isArray(records)||!records.length||records.length>10000||records.some(r=>!r||typeof r.id!=='string'||!r.id.length||r.id.length>128||!Number.isSafeInteger(r.price)))throw new Error('invalid records');
  if(new Set(records.map(r=>r.id)).size!==records.length)throw new Error('duplicate record ID');
  const normalized=records.map(r=>({id:r.id,price:r.price})).sort((a,b)=>a.id.localeCompare(b.id));
  return {records:normalized,digest:createHash('sha256').update(JSON.stringify(normalized)).digest('hex')};
}
const adapter:Adapter<Request,{path:string;leaseMs:number}>= {
  info(){return {backend:'workflow-node',protocol:1,sqlite:db?.prepare('SELECT sqlite_version() v').get()?.v,pragmas:db?Object.fromEntries(['journal_mode','synchronous','busy_timeout','foreign_keys','wal_autocheckpoint','page_size'].map(p=>[p,Object.values(db!.prepare(`PRAGMA ${p}`).get()!)[0]])):null,contract:'WAL FULL; jobs and results committed atomically; lease tokens fence stale commits; receipt replay is idempotent'}},
  async prepare(options){
    if(db)throw new Error('already prepared');if(!Number.isInteger(options.leaseMs)||options.leaseMs<10||options.leaseMs>120000)throw new Error('invalid lease duration');leaseMs=options.leaseMs;
    db=new DatabaseSync(options.path);db.exec('PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON; PRAGMA wal_autocheckpoint=1000; CREATE TABLE IF NOT EXISTS jobs(jobId TEXT PRIMARY KEY,task TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN (\'pending\',\'leased\',\'committed\',\'acked\',\'failed\')),token TEXT,expires INTEGER NOT NULL DEFAULT 0,attempts INTEGER NOT NULL DEFAULT 0,receipt TEXT,digest TEXT,error TEXT); CREATE TABLE IF NOT EXISTS results(jobId TEXT NOT NULL REFERENCES jobs(jobId),id TEXT NOT NULL,price INTEGER NOT NULL,PRIMARY KEY(jobId,id));');
  },
  async run(request){
    const d=database();
    if(request.op==='submit'){
      if(!Array.isArray(request.jobs)||!request.jobs.length||request.jobs.length>10000)throw new Error('invalid jobs');
      return transaction(()=>{for(const job of request.jobs){if(!/^[a-zA-Z0-9_-]{1,128}$/.test(job.jobId)||!job.task||job.task.scenario!=='http-catalog'||typeof job.task.url!=='string')throw new Error('invalid job');const task=JSON.stringify(job.task);const existing=getJob(job.jobId);if(existing&&existing.task!==task)throw new Error('job ID conflicts with a different task');if(!existing)d.prepare("INSERT INTO jobs(jobId,task,state) VALUES(?,?,'pending')").run(job.jobId,task)}return {submitted:request.jobs.length}});
    }
    if(request.op==='claim')return transaction(()=>{
      const job=d.prepare("SELECT * FROM jobs WHERE state='pending' OR (state='leased' AND expires<=?) ORDER BY rowid LIMIT 1").get(Date.now()) as Job|undefined;
      if(!job)return {job:null};const token=randomUUID();const expires=Date.now()+leaseMs;
      d.prepare("UPDATE jobs SET state='leased',token=?,expires=?,attempts=attempts+1 WHERE jobId=?").run(token,expires,job.jobId);
      return {jobId:job.jobId,task:JSON.parse(job.task),token,expires};
    });
    if(request.op==='commit'){
      if(!Array.isArray(request.entries)||!request.entries.length||request.entries.length>128||new Set(request.entries.map(e=>e.jobId)).size!==request.entries.length)throw new Error('invalid commit batch');
      if(request.fault!==undefined&&!['before-commit','after-commit'].includes(request.fault))throw new Error('invalid fault');
      const prepared=request.entries.map(entry=>({...entry,...canonical(entry.records)}));
      const receipts=transaction(()=>prepared.map(entry=>{
        const job=getJob(entry.jobId);if(!job||job.token!==entry.token)throw new Error('stale lease token');
        if(job.state==='committed'||job.state==='acked'){if(job.digest!==entry.digest)throw new Error('replay conflicts with different records');return {jobId:job.jobId,receipt:job.receipt!}}
        if(job.state!=='leased'||job.expires<=Date.now())throw new Error('expired lease');
        const insert=d.prepare('INSERT INTO results(jobId,id,price) VALUES(?,?,?)');for(const record of entry.records)insert.run(job.jobId,record.id,record.price);
        const receipt=randomUUID();d.prepare("UPDATE jobs SET state='committed',receipt=?,digest=? WHERE jobId=?").run(receipt,entry.digest,job.jobId);
        if(request.fault==='before-commit')process.kill(process.pid,'SIGKILL');
        return {jobId:job.jobId,receipt};
      }));
      if(request.fault==='after-commit')process.kill(process.pid,'SIGKILL');
      return {receipts};
    }
    if(request.op==='ack'){
      if(!Array.isArray(request.receipts)||!request.receipts.length||request.receipts.length>128)throw new Error('invalid acknowledgements');
      return transaction(()=>{for(const ack of request.receipts){const job=getJob(ack.jobId);if(!job||job.receipt!==ack.receipt||!['committed','acked'].includes(job.state))throw new Error('unknown committed receipt');d.prepare("UPDATE jobs SET state='acked' WHERE jobId=?").run(job.jobId)}return {acknowledged:request.receipts.length}});
    }
    if(request.op==='reject')return transaction(()=>{const job=getJob(request.jobId);if(!job||job.state!=='leased'||job.token!==request.token)throw new Error('stale lease token');d.prepare("UPDATE jobs SET state='failed',error=? WHERE jobId=?").run(request.reason.slice(0,2000),job.jobId);return {rejected:job.jobId}});
    if(request.op==='snapshot'){
      const integrity=d.prepare('PRAGMA integrity_check').all();const foreignKeys=d.prepare('PRAGMA foreign_key_check').all();
      const jobs=(d.prepare('SELECT * FROM jobs ORDER BY rowid').all() as Job[]).map(job=>({...job,task:JSON.parse(job.task),records:d.prepare('SELECT id,price FROM results WHERE jobId=? ORDER BY id').all(job.jobId)}));
      return {jobs,integrity:integrity.map(r=>Object.values(r)[0]),foreignKeys};
    }
    throw new Error('unknown workflow operation');
  },
  async close(){db?.close();db=undefined}
};
serve(adapter);
