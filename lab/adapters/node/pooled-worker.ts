import { CheerioCrawler, Configuration, MemoryStorage, log } from 'crawlee';
import { serve, type Adapter, type Task, type AdapterOutput } from '../../src/worker-server.ts';

log.setLevel(log.LEVELS.ERROR);
type Job = { records:{id:string;price:number}[]; pages:number; maxPages:number; resolve:(value:AdapterOutput)=>void; reject:(error:Error)=>void; start:number };
const jobs = new Map<string,Job>(); let crawler:CheerioCrawler|undefined, running:Promise<unknown>|undefined, fatal:Error|undefined;
let poolWake:'poll'|'notify'='poll';
let poolReady=false;
const adapter:Adapter<Task,{concurrency?:number;poolWake?:'poll'|'notify';respectRobots?:boolean}> = {
  info(){return {backend:'crawlee-http-pooled',protocol:1,capabilities:['http-catalog'],versions:{crawlee:'3.18.2'},poolWake,lifecycle:'one keepAlive crawler and request queue per block; tasks share its bounded pool'}},
  async prepare(input){
    if(input.poolWake!==undefined&&!['poll','notify'].includes(input.poolWake))throw new Error('invalid pool wake mode');poolWake=input.poolWake??'poll';
    crawler=new CheerioCrawler({keepAlive:true,respectRobotsTxtFile:input.respectRobots===true,minConcurrency:input.concurrency??1,maxConcurrency:input.concurrency??1,maxRequestRetries:0,
      requestHandler:async({request,$,enqueueLinks})=>{
        // The pool object exists before crawler.run finishes initialization.
        // A completed start into a request handler proves it can accept notify.
        poolReady=true;
        const runId=String(request.userData.runId); const job=jobs.get(runId); if(!job)throw new Error('unknown crawl job');
        if(++job.pages>job.maxPages)throw new Error('pagination budget exceeded');
        $('article[data-id]').each((_,el)=>{job.records.push({id:$(el).attr('data-id')!,price:Number($(el).attr('data-price'))})});
        if($('a.next').length)await enqueueLinks({selector:'a.next',userData:{runId}});
        else {jobs.delete(runId);job.resolve({status:'ok',records:job.records,metrics:{taskMs:performance.now()-job.start,pages:job.pages}})}
      },failedRequestHandler:async({request})=>{const id=String(request.userData.runId);jobs.get(id)?.reject(new Error(`crawl failed ${request.url}`));jobs.delete(id)}
    },new Configuration({persistStorage:false,storageClient:new MemoryStorage({persistStorage:false})}));
    running=crawler.run().catch(error=>{fatal=error;for(const job of jobs.values())job.reject(error);jobs.clear()});
  },
  async run(task){
    if(task.scenario!=='http-catalog')return {status:'unsupported'};
    if(!crawler||fatal)throw fatal??new Error('not prepared');
    const runId=new URL(task.url).pathname.split('/')[2]!;
    return await new Promise<AdapterOutput>((resolve,reject)=>{
      jobs.set(runId,{records:[],pages:0,maxPages:task.pages,resolve,reject,start:performance.now()});
      void crawler!.addRequests([{url:task.url,userData:{runId}}]).then(async()=>{if(poolWake==='notify'&&poolReady)await crawler!.autoscaledPool?.notify()}).catch(error=>{jobs.delete(runId);reject(error)});
    });
  },
  async close(){crawler?.stop('lab block complete');await running;}
};
serve(adapter);
