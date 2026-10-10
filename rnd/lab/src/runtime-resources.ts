import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {join} from 'node:path';
import {labRoot} from './registry.ts';
import {sampleLinuxResources} from './core/metrics/linux.ts';
const exec=promisify(execFile);
type Usage={pid:number;started:string;userNs:number;systemNs:number;timebaseNumer:number;timebaseDenom:number;rss:number;footprint:number;kernelPeakFootprint:number;diskReadBytes:number;diskWriteBytes:number};
// On Linux the same sampler reads /proc: proportional memory (PSS) instead of RSS and footprint, CPU from stat.
function sampleLinuxRuntime(rootPid:number,intervalMs:number){
  const linux=sampleLinuxResources(rootPid,intervalMs);
  return {async ready(){},async stop(){const r=await linux.stop();return {intervalMs,samples:r.samples,unavailable:r.unavailable,observedProcessCount:null as number|null,peakRssBytes:null as number|null,peakFootprintBytes:null as number|null,peakPssBytes:r.peakPssBytes,sampledUserCpuMs:r.sampledUserCpuMs,sampledSystemCpuMs:r.sampledSystemCpuMs,sampledDiskReadBytes:null as number|null,sampledDiskWriteBytes:null as number|null,perProcess:[] as unknown[],baselines:[] as unknown[],method:r.method}}};
}
export function sampleRuntimeResources(rootPid:number,intervalMs=100,options:{existingRoot?:boolean}={}){
  if(process.platform==='linux')return sampleLinuxRuntime(rootPid,intervalMs) as unknown as ReturnType<typeof sampleDarwinRuntime>;
  return sampleDarwinRuntime(rootPid,intervalMs,options);
}
function sampleDarwinRuntime(rootPid:number,intervalMs=100,options:{existingRoot?:boolean}={}){
  let pending:Promise<void>|undefined;let stopped=false;let samples=0;let peakRssBytes=0;let peakFootprintBytes=0;let unavailable=false;const lifetime=new Map<string,Usage>();const baselines=new Map<string,Usage>();
  async function sample(){
    try{
      const result=await exec('ps',['-axo','pid=,ppid=,pgid='],{maxBuffer:4000000,timeout:2000});const rows=result.stdout.trim().split('\n').map(row=>row.trim().split(/\s+/).map(Number));const selected=new Set([rootPid]);let changed=true;
      while(changed){changed=false;for(const row of rows)if(selected.has(row[1]!)&&!selected.has(row[0]!)){selected.add(row[0]!);changed=true}}
      const pids=rows.filter(row=>selected.has(row[0]!)||row[2]===rootPid).map(row=>String(row[0]));if(!pids.length)return;
      const usage=JSON.parse((await exec(join(labRoot,'.native/process-probe'),pids,{maxBuffer:1000000,timeout:2000})).stdout) as Usage[];
      if(!usage.length)return;samples++;peakRssBytes=Math.max(peakRssBytes,usage.reduce((n,u)=>n+u.rss,0));peakFootprintBytes=Math.max(peakFootprintBytes,usage.reduce((n,u)=>n+u.footprint,0));
      for(const row of usage){const key=`${row.pid}:${row.started}`;if(options.existingRoot&&row.pid===rootPid&&!lifetime.has(key))baselines.set(key,row);lifetime.set(key,row)}
    }catch{unavailable=true}
  }
  function tick(){if(stopped||pending)return;pending=sample().finally(()=>{pending=undefined})}
  const timer=setInterval(tick,intervalMs);timer.unref();tick();
  return {async ready(){await pending},async stop(){stopped=true;clearInterval(timer);await pending;await sample();const usage=[...lifetime.values()];const delta=(u:Usage,key:'userNs'|'systemNs'|'diskReadBytes'|'diskWriteBytes')=>u[key]-(baselines.get(`${u.pid}:${u.started}`)?.[key]??0);return {intervalMs,samples,unavailable,observedProcessCount:usage.length,peakRssBytes:samples?peakRssBytes:null,peakFootprintBytes:samples?peakFootprintBytes:null,sampledUserCpuMs:usage.reduce((n,u)=>n+delta(u,'userNs')/1e6,0),sampledSystemCpuMs:usage.reduce((n,u)=>n+delta(u,'systemNs')/1e6,0),sampledDiskReadBytes:usage.reduce((n,u)=>n+delta(u,'diskReadBytes'),0),sampledDiskWriteBytes:usage.reduce((n,u)=>n+delta(u,'diskWriteBytes'),0),perProcess:usage,baselines:[...baselines.values()],method:'macOS proc_pid_rusage V4 with mach_timebase_info conversion; sampled root and descendants; existing coordinator counters subtract the initial baseline; new process counters use their observed lifetime; CPU/I/O can miss exited process tails; RSS sums double-count shared pages; disk counters are kernel-attributed I/O, not file bytes or a durability proof'}}};
}
