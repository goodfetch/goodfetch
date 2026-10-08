import {test} from 'node:test';
import assert from 'node:assert/strict';
import {WorkerClient} from '../../src/worker-client.ts';
import {commandFor} from '../../src/registry.ts';
import {startFixtures} from '../../fixtures/server.ts';

test('the Crawlee pool declares notification mode and preserves exact extraction across idle admissions',async()=>{
  const fixtures=await startFixtures();const worker=new WorkerClient(commandFor('crawlee-http-pooled',15000));
  try{
    await worker.call('prepare',{concurrency:1,poolWake:'notify'});const info=await worker.call('hello',{}) as {poolWake:string};assert.equal(info.poolWake,'notify');
    for(let i=0;i<2;i++){const fixture=await fixtures.createRun({scenario:'http-catalog',seed:42,pages:2});const result=await worker.call('run',{scenario:'http-catalog',seed:42,pages:2,actions:1,variant:'normal',url:fixture.url,timeoutMs:15000}) as {records:{id:string;price:number}[]};assert.equal((await fixtures.verify(fixture.id,result)).status,'verified');await new Promise(resolve=>setTimeout(resolve,20))}
  }finally{await worker.close();await fixtures.close()}
});
