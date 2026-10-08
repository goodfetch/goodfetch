import {test} from 'node:test';
import assert from 'node:assert/strict';
import {startFixtures} from '../../fixtures/server.ts';
import {WorkerClient} from '../../src/worker-client.ts';
import {commandFor} from '../../src/registry.ts';
import {browserPath} from '../../src/environment.ts';

test('native Playwright waits for a covered target and refuses a permanently disabled target',async()=>{
  const fixtures=await startFixtures();const w=new WorkerClient(commandFor('playwright',10000,browserPath()));
  try{
    await w.call('prepare',{executablePath:browserPath(),headless:true});
    for(const variant of ['overlay','never-enabled'] as const){const fixture=await fixtures.createRun({scenario:'actionability',seed:42,variant});
      const output=await w.call('run',{scenario:'actionability',seed:42,pages:1,actions:1,url:fixture.url,timeoutMs:800,variant,actionPolicy:'native'}) as Record<string,unknown>;
      assert.deepEqual(await fixtures.verify(fixture.id,output),{status:'verified',reasons:[]});
    }
  }finally{await w.close();await fixtures.close()}
});
test('the action oracle rejects a premature trusted effect even when an adapter reports success',async()=>{
  const fixtures=await startFixtures();try{const fixture=await fixtures.createRun({scenario:'actionability',seed:42,variant:'moving'});
    await fetch(`${fixtures.origin}/runs/${fixture.id}/effect`,{method:'POST',body:JSON.stringify({target:'target',phaseReady:false,trusted:true})});
    assert.equal((await fixtures.verify(fixture.id,{metrics:{nativeRejected:0}})).status,'failed');
  }finally{await fixtures.close()}
});
test('guarded Stagehand clicks wait for a moving target before producing their trusted effect',async()=>{
  const fixtures=await startFixtures();
  try{
    for(const backend of ['stagehand','stagehand-batch'] as const){
      const w=new WorkerClient(commandFor(backend,15000,browserPath()));
      try{
        await w.call('prepare',{executablePath:browserPath(),headless:true});
        for(let iteration=0;iteration<8;iteration++){
          const fixture=await fixtures.createRun({scenario:'actionability',seed:4200+iteration,variant:'moving'});
          const output=await w.call('run',{scenario:'actionability',seed:4200+iteration,pages:1,actions:1,url:fixture.url,timeoutMs:1500,variant:'moving',actionPolicy:'guarded'}) as Record<string,unknown>;
          const verdict=await fixtures.verify(fixture.id,output);
          assert.equal(verdict.status,'verified',JSON.stringify({backend,iteration,verdict,output,observed:fixtures.snapshot(fixture.id)}));
        }
      }finally{await w.close()}
    }
  }finally{await fixtures.close()}
});
