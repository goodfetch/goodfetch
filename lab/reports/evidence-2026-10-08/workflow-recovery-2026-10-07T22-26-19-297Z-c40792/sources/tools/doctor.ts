import { access } from 'node:fs/promises';
import { environment } from '../src/environment.ts';
import { commandFor, backendNames, labRoot } from '../src/registry.ts';
import { join } from 'node:path';
import { WorkerClient } from '../src/worker-client.ts';
import {storageEngines,storageCommand,isSharedEngine,sharedBuildMetadata} from '../src/storage-engines.ts';
try {
  const env=await environment(); console.log(JSON.stringify({...env,sourceHashes:{files:Object.keys(env.sourceHashes).length}}, null, 2));
  for (const backend of backendNames) {
    const spec = commandFor(backend, 15_000, env.browser.executablePath);
    try {
      await access(spec.command);
      const worker = new WorkerClient(spec); try { console.log(JSON.stringify({ backend, info: await worker.call('hello', {}) })); } finally { await worker.close(); }
    } catch (error) { console.log(JSON.stringify({ backend, unavailable: error instanceof Error ? error.message : String(error) })); process.exitCode=1; }
  }
  for(const backend of storageEngines) { if(isSharedEngine(backend)&&!(process.platform==='darwin'&&process.arch==='arm64')){console.log(JSON.stringify({backend,unsupported:'shared native build currently qualified on macOS arm64'}));continue}if(isSharedEngine(backend))await sharedBuildMetadata();const worker=new WorkerClient(storageCommand(backend,10000));
    try {console.log(JSON.stringify({backend,info:await worker.call('hello',{})}));} finally {await worker.close();} }
} catch (error) { console.error(error); process.exitCode = 1; }
