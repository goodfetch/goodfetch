import {readFile} from 'node:fs/promises';
import {join} from 'node:path';
import {executeWorkflow,type WorkflowJob,type WorkflowConfig} from './workflow.ts';
import {labRoot} from './registry.ts';
const config=JSON.parse(await readFile(process.argv[2]!,'utf8')) as WorkflowConfig&{jobs:WorkflowJob[];oracleUrl:string;oracleToken:string;corruptOutput?:boolean;coordinatorCrashAt?:string};
try{
  const result=await executeWorkflow({...config,backend:config.backend??'node-http'},config.jobs,async(jobId,records)=>{
    if(config.corruptOutput&&records[0])records[0].price=-1;
    const response=await fetch(config.oracleUrl,{method:'POST',headers:{authorization:config.oracleToken,'content-type':'application/json'},body:JSON.stringify({jobId,records}),signal:AbortSignal.timeout(config.timeoutMs)});if(!response.ok)throw new Error(`oracle HTTP ${response.status}`);
    return await response.json() as {status:'verified'|'failed';reasons:string[]};
  },{nativeAcquisition:!config.backend||config.backend==='node-http',sampleResources:false,retainEvents:false,sinkCommand:{command:process.execPath,args:[join(labRoot,'adapters/workflow-node/worker.ts')],cwd:labRoot,env:{...process.env,LAB_SHARED_SQLITE:'1'},timeoutMs:config.timeoutMs},onEvent:async row=>{process.stdout.write(JSON.stringify(row)+'\n');if(row.type===config.coordinatorCrashAt)await new Promise(resolve=>setTimeout(resolve,50))}});
  process.stdout.write(JSON.stringify({type:'summary',result:{...result,events:undefined,runtime:'node'}})+'\n');
}catch(error){process.stdout.write(JSON.stringify({type:'fatal',error:error instanceof Error?error.message:String(error)})+'\n');process.exitCode=1}
