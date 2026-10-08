import type {Task} from './worker-server.ts';
export class NativeHttp {
  readonly pid=process.pid;
  async call(method:string,params:unknown){
    if(method==='hello')return {backend:'native-node-http',runtime:process.version,lifecycle:'HTTP acquisition in coordinator; dedicated sink process'};
    if(method==='prepare')return {ready:true};
    const task=params as Task;if(method!=='run')throw new Error('unknown native acquisition operation');
    const records:{id:string;price:number}[]=[];const visited=new Set<string>();let url:string|null=task.url;
    while(url){
      if(visited.has(url)||visited.size>=task.pages)throw new Error('pagination exceeded contract');visited.add(url);
      const response:globalThis.Response=await fetch(url,{signal:AbortSignal.timeout(task.timeoutMs)});if(!response.ok)throw new Error(`HTTP ${response.status}`);
      const html:string=await response.text();for(const match of html.matchAll(/<article data-id="([^"]+)" data-price="(\d+)"/g))records.push({id:match[1]!,price:Number(match[2])});
      const next:RegExpMatchArray|null=html.match(/class="next" href="([^"]+)"/);url=next?new URL(next[1]!,url).href:null;
    }
    return {status:'ok',records};
  }
  async close(){}
}
