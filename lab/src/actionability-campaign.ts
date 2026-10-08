import {mkdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {randomBytes} from 'node:crypto';
import {startFixtures,type RunSpec} from '../fixtures/server.ts';
import {WorkerClient} from './worker-client.ts';
import {commandFor,labRoot,type BackendName} from './registry.ts';
import {environment,sourceFingerprint,copySourceSnapshot} from './environment.ts';
import {launchLightpanda,lightpandaPath,type HostedBrowser} from './core/browser-host.ts';
import {shuffle} from './statistics.ts';

// The matrix lane runs one action per cell with a fresh adapter: the actionability variants, or the primitive actions.
export type ActionabilityConfig={blocks:number;seed:number;backends:BackendName[];variants:NonNullable<RunSpec['variant']>[];policies:('native'|'guarded')[];timeoutMs:number;outerTimeoutMs:number;scenario?:'actionability'|'primitives'|'navigation'|'http-catalog';respectRobots?:boolean;insecureTls?:boolean;browserEngine?:'chromium'|'lightpanda'};
export const defaultActionability=(blocks=3,policies:('native'|'guarded')[]=['native','guarded']):ActionabilityConfig=>({blocks,seed:4242,backends:['playwright','puppeteer','stagehand','stagehand-batch','crawlee-playwright','crawlee-stagehand'],variants:['normal','overlay','delayed','hidden','moving','rerender','never-enabled','never-uncovered','ambiguous'],policies,timeoutMs:800,outerTimeoutMs:8000});
export type ActionabilityCell={backend:BackendName;variant:NonNullable<RunSpec['variant']>;policy:'native'|'guarded';scenario:'actionability'|'primitives'|'navigation'|'http-catalog'};
export function actionabilityCombinations(config:ActionabilityConfig):ActionabilityCell[]{const scenario=config.scenario??'actionability';return config.backends.flatMap(backend=>config.variants.flatMap(variant=>config.policies.map(policy=>({backend,variant,policy,scenario}))))}
// One actionability cell: fresh adapter and fixtures, one action, effect and refusal checked by the private oracle.
export async function runActionabilityCell(config:ActionabilityConfig,cell:ActionabilityCell,block:number,executablePath:string):Promise<Record<string,unknown>>{
  // With browserEngine lightpanda, every driver connects to a fresh Lightpanda server instead of launching Chromium.
  const engine:HostedBrowser|undefined=config.browserEngine==='lightpanda'?await launchLightpanda(lightpandaPath(labRoot)):undefined;
  const fixtures=await startFixtures();const w=new WorkerClient(commandFor(cell.backend,config.outerTimeoutMs,executablePath));let startupMs=0;const started=performance.now();let info:unknown;
  try{
    info=await w.call('hello',{});await w.call('prepare',{executablePath,headless:true,respectRobots:config.respectRobots??false,insecureTls:config.insecureTls??false,...(engine?{cdpEndpoint:engine.wsEndpoint}:{})},45000);startupMs=performance.now()-started;
    // A catalog run spans three pages, so an incident on the second one is reached mid-crawl.
    const pages=cell.scenario==='http-catalog'?3:1;
    const fixture=await fixtures.createRun({scenario:cell.scenario,seed:config.seed+block,pages,variant:cell.variant});const began=performance.now();
    let row:Record<string,unknown>;
    try{const output=await w.call('run',{scenario:cell.scenario,seed:config.seed+block,pages,actions:1,timeoutMs:config.timeoutMs,variant:cell.variant,actionPolicy:cell.policy,respectRobots:config.respectRobots??false,insecureTls:config.insecureTls??false,url:fixture.url}) as Record<string,unknown>;// A driver without the capability says so; it is unsupported, not failed.
      const verdict=output.status==='unsupported'?{status:'unsupported',reasons:[String(output.reason??'unsupported')]}:await fixtures.verify(fixture.id,output);row={...cell,block,...verdict,taskMs:performance.now()-began,metrics:output.metrics,...fixtures.snapshot(fixture.id)}}
    catch(error){const verdict=fixtures.expectsRefusal(fixture.id)?await fixtures.verify(fixture.id,{error:String(error)}):{status:'failed',reasons:[String(error)]};row={...cell,block,...verdict,taskMs:performance.now()-began,...fixtures.snapshot(fixture.id)}}
    return {...row,startupMs,info};
  }catch(error){return {...cell,block,status:'startup-failed',reasons:[String(error)],startupMs:performance.now()-started,info}}
  finally{await w.close();await fixtures.close();await engine?.close()}
}
export async function actionabilityCampaign(blocks=3,policies:('native'|'guarded')[]=['native','guarded'],config:ActionabilityConfig=defaultActionability(blocks,policies)){
  const id=`browser-actionability-${new Date().toISOString().replace(/[:.]/g,'-')}-${randomBytes(3).toString('hex')}`;const directory=join(labRoot,'results',id);await mkdir(directory,{recursive:true});
  const env=await environment();await copySourceSnapshot(directory,env.sourceHashes);
  const cells:Record<string,unknown>[]=[];const manifest={id,blocks:config.blocks,backends:config.backends,variants:config.variants,policies:config.policies,environment:env,timeoutMs:config.timeoutMs,outerTimeoutMs:config.outerTimeoutMs,methodology:'fresh adapter per cell; same Chromium and viewport; native actions do not use common readiness waits; guarded actions add the same bounded strictness/visibility/enabled/stability/hit-test checks with two stable intervals sampled at least 16ms apart; every effect independently verified; negatives must reject without effects; failures remain visible; deterministic mode without model calls'};
  await writeFile(join(directory,'manifest.json'),JSON.stringify(manifest,null,2)+'\n');
  for(let block=0;block<config.blocks;block++)for(const cell of shuffle(actionabilityCombinations(config),config.seed+block)){
    cells.push(await runActionabilityCell(config,cell,block,env.browser.executablePath));
    await writeFile(join(directory,'summary.json'),JSON.stringify({cells},null,2)+'\n');process.stderr.write(`${cell.backend}/${cell.policy}/${cell.variant}/b${block}: ${cells.at(-1)!.status}\n`);
  }
  const sourceChanges=Object.entries(await sourceFingerprint()).filter(([file,hash])=>env.sourceHashes[file]!==hash).map(([file])=>file);
  await writeFile(join(directory,'summary.json'),JSON.stringify({cells,sourceChanges},null,2)+'\n');return {directory,cells,sourceChanges};
}
if(process.argv[1]?.endsWith('actionability-campaign.ts')){
  const policy=process.argv[3];if(policy!==undefined&&policy!=='native'&&policy!=='guarded')throw new Error('invalid action policy');
  process.stdout.write(JSON.stringify(await actionabilityCampaign(Number(process.argv[2]??3),policy?[policy]:undefined))+'\n');
}
