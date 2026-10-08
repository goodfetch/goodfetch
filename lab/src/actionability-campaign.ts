import {mkdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {randomBytes} from 'node:crypto';
import {startFixtures,type RunSpec} from '../fixtures/server.ts';
import {WorkerClient} from './worker-client.ts';
import {commandFor,labRoot,type BackendName} from './registry.ts';
import {environment,sourceFingerprint,copySourceSnapshot} from './environment.ts';
import {shuffle} from './statistics.ts';

export async function actionabilityCampaign(blocks=3,policies:('native'|'guarded')[]=['native','guarded']){
  const id=`browser-actionability-${new Date().toISOString().replace(/[:.]/g,'-')}-${randomBytes(3).toString('hex')}`;const directory=join(labRoot,'results',id);await mkdir(directory,{recursive:true});
  const env=await environment();await copySourceSnapshot(directory,env.sourceHashes);
  const backends:BackendName[]=['playwright','puppeteer','stagehand','stagehand-batch','crawlee-playwright','crawlee-stagehand'];
  const variants:NonNullable<RunSpec['variant']>[]=['normal','overlay','delayed','hidden','moving','rerender','never-enabled','never-uncovered','ambiguous'];
  const cells:Record<string,unknown>[]=[];const manifest={id,blocks,backends,variants,policies,environment:env,timeoutMs:800,outerTimeoutMs:8000,methodology:'fresh adapter per cell; same Chromium and viewport; native actions do not use common readiness waits; guarded actions add the same bounded strictness/visibility/enabled/stability/hit-test checks with two stable intervals sampled at least 16ms apart; every effect independently verified; negatives must reject without effects; failures remain visible; deterministic mode without model calls'};
  await writeFile(join(directory,'manifest.json'),JSON.stringify(manifest,null,2)+'\n');
  for(let block=0;block<blocks;block++)for(const cell of shuffle(backends.flatMap(backend=>variants.flatMap(variant=>policies.map(policy=>({backend,variant,policy})))),4242+block)){
    const fixtures=await startFixtures();const w=new WorkerClient(commandFor(cell.backend,8000,env.browser.executablePath));let startupMs=0;const started=performance.now();let info:unknown;
    try{
      info=await w.call('hello',{});await w.call('prepare',{executablePath:env.browser.executablePath,headless:true},45000);startupMs=performance.now()-started;
      const fixture=await fixtures.createRun({scenario:'actionability',seed:4242+block,variant:cell.variant});const began=performance.now();
      let row:Record<string,unknown>;
      try{const output=await w.call('run',{scenario:'actionability',seed:4242+block,pages:1,actions:1,timeoutMs:800,variant:cell.variant,actionPolicy:cell.policy,url:fixture.url}) as Record<string,unknown>;const verdict=await fixtures.verify(fixture.id,output);row={...cell,block,...verdict,taskMs:performance.now()-began,metrics:output.metrics,...fixtures.snapshot(fixture.id)}}
      catch(error){row={...cell,block,status:'failed',reasons:[String(error)],taskMs:performance.now()-began,...fixtures.snapshot(fixture.id)}}
      cells.push({...row,startupMs,info});
    }catch(error){cells.push({...cell,block,status:'startup-failed',reasons:[String(error)],startupMs:performance.now()-started,info})}
    finally{await w.close();await fixtures.close()}
    await writeFile(join(directory,'summary.json'),JSON.stringify({cells},null,2)+'\n');process.stderr.write(`${cell.backend}/${cell.policy}/${cell.variant}/b${block}: ${cells.at(-1)!.status}\n`);
  }
  const sourceChanges=Object.entries(await sourceFingerprint()).filter(([file,hash])=>env.sourceHashes[file]!==hash).map(([file])=>file);
  await writeFile(join(directory,'summary.json'),JSON.stringify({cells,sourceChanges},null,2)+'\n');return {directory,cells,sourceChanges};
}
if(process.argv[1]?.endsWith('actionability-campaign.ts')){
  const policy=process.argv[3];if(policy!==undefined&&policy!=='native'&&policy!=='guarded')throw new Error('invalid action policy');
  process.stdout.write(JSON.stringify(await actionabilityCampaign(Number(process.argv[2]??3),policy?[policy]:undefined))+'\n');
}
