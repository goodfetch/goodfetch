import {test} from 'node:test';
import assert from 'node:assert/strict';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {pathToFileURL} from 'node:url';
import {join} from 'node:path';
import {labRoot} from '../src/registry.ts';
test('a closed adapter leaves no referenced shutdown timer keeping its supervisor alive',async()=>{
  const script=`import {WorkerClient} from ${JSON.stringify(pathToFileURL(join(labRoot,'src/worker-client.ts')).href)};
    const worker=new WorkerClient({command:process.execPath,args:['-e',${JSON.stringify("const rl=require('node:readline').createInterface({input:process.stdin});rl.on('line',s=>{const m=JSON.parse(s);process.stdout.write(JSON.stringify({version:1,id:m.id,result:{ok:true}})+'\\n');if(m.method==='close'){rl.close();process.stdin.destroy()}})")} ]});
    await worker.call('hello',{});await worker.close();process.stdout.write(JSON.stringify(process.getActiveResourcesInfo()));`;
  const result=await promisify(execFile)(process.execPath,['--input-type=module','-e',script],{timeout:5000});
  assert.equal((JSON.parse(result.stdout) as string[]).filter(name=>name==='Timeout').length,0);
});
