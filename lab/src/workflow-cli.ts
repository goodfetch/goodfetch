import {readFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {runWorkflowCampaign,type WorkflowCampaign} from './workflow-campaign.ts';
try{
  const config=JSON.parse(await readFile(resolve(process.argv[2]??'campaigns/workflow-throughput.json'),'utf8')) as WorkflowCampaign;
  const result=await runWorkflowCampaign(config);console.log(result.directory);
  if(result.cells.some(c=>c.status==='failed')||result.sourceChanges.length||result.binaryChanged)process.exitCode=1;
}catch(error){console.error(error instanceof Error?error.message:error);process.exitCode=1}
