import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {randomBytes} from 'node:crypto';
import {runtimeCampaign,type RuntimeCampaign} from '../src/runtime-campaign.ts';
import {actionabilityCampaign} from '../src/actionability-campaign.ts';
import {readersCampaign} from '../src/sqlite-readers-campaign.ts';
import {runCampaign,type Campaign} from '../src/campaign.ts';
import {labRoot} from '../src/registry.ts';
import {environment,sourceFingerprint,copySourceSnapshot} from '../src/environment.ts';
const id=`qualification-current-${new Date().toISOString().replace(/[:.]/g,'-')}-${randomBytes(3).toString('hex')}`;
const directory=join(labRoot,'results',id);await mkdir(directory,{recursive:true});
const env=await environment();await copySourceSnapshot(directory,env.sourceHashes);
const native=JSON.parse(await readFile(join(labRoot,'campaigns/runtime-native-qualified.json'),'utf8')) as RuntimeCampaign;
const recovery=JSON.parse(await readFile(join(labRoot,'campaigns/runtime-recovery-qualified.json'),'utf8')) as RuntimeCampaign;
const transport=JSON.parse(await readFile(join(labRoot,'campaigns/spider-http-transport-supported.json'),'utf8')) as Campaign;
const steps:{name:string;directory?:string;expected:number;verified?:number;total?:number;status:string;error?:string}[]=[];
await writeFile(join(directory,'manifest.json'),JSON.stringify({id,environment:env,native,recovery,transport,browser:{blocks:3,policies:['guarded']},readers:{blocks:3},methodology:'sequential requalification after public red-green regressions; original campaigns and pre-Arc Rust binary retained; Spider redirect policy unchanged'},null,2)+'\n');
async function step(name:string,expected:number,run:()=>Promise<{directory:string;cells?:Record<string,unknown>[];rows?:{status:string}[];sourceChanges?:string[];binaryChanges?:string[];binaryChanged?:boolean}>){
  process.stderr.write(`START ${name}\n`);
  try{
    const result=await run();const rows=result.cells??result.rows??[];const verified=rows.filter(r=>r.status==='verified').length;
    const changed=!!result.sourceChanges?.length||!!result.binaryChanges?.length||result.binaryChanged;
    steps.push({name,directory:result.directory,expected,verified,total:rows.length,status:changed?'invalid-provenance':verified===expected&&rows.length===expected?'qualified':'recorded-failures'});
  }catch(error){steps.push({name,expected,status:'failed',error:String(error)})}
  await writeFile(join(directory,'summary.json'),JSON.stringify({steps},null,2)+'\n');process.stderr.write(`FINISH ${name}: ${steps.at(-1)!.status}\n`);
}
await step('runtime-native-qualified',30,()=>runtimeCampaign(native));
await step('runtime-recovery-qualified',42,()=>runtimeCampaign(recovery));
await step('browser-actionability-guarded',162,()=>actionabilityCampaign(3,['guarded']));
await step('sqlite-readers-qualified',108,()=>readersCampaign(3));
await step('spider-http-transport-supported',60,()=>runCampaign(transport));
const sourceChanges=Object.entries(await sourceFingerprint()).filter(([file,hash])=>env.sourceHashes[file]!==hash).map(([file])=>file);
await writeFile(join(directory,'summary.json'),JSON.stringify({steps,sourceChanges},null,2)+'\n');process.stdout.write(JSON.stringify({directory,steps,sourceChanges})+'\n');
if(sourceChanges.length||steps.some(s=>s.status!=='qualified'))process.exitCode=1;
