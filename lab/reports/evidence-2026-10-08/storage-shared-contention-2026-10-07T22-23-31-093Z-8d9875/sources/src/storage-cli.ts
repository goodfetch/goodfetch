import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { runStorageCampaign, type StorageConfig } from './storage-campaign.ts';
try { const config=JSON.parse(await readFile(resolve(process.argv[2]??'campaigns/storage-smoke.json'),'utf8')) as StorageConfig;
  const result=await runStorageCampaign(config); console.log(result.directory); if(result.cells.some(cell=>cell.status==='failed')||result.sourceChanges.length) process.exitCode=1;
} catch(error) { console.error(error instanceof Error?error.message:error); process.exitCode=1; }
