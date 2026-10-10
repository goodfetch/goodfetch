use crate::ledger::now;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Write,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, mpsc, oneshot},
};

fn emit(v: Value) {
    let mut output = std::io::stdout().lock();
    writeln!(output, "{v}").ok();
    output.flush().ok();
}
struct Rpc {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    sequence: u64,
    timeout: u64,
}
impl Rpc {
    fn spawn(timeout: u64) -> Result<Self, String> {
        let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .arg("sink")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            child,
            input,
            output,
            sequence: 0,
            timeout,
        })
    }
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.sequence += 1;
        let id = self.sequence;
        let line = json!({"version":1,"id":id,"method":method,"params":params}).to_string() + "\n";
        if line.len() > 128000 {
            return Err("adapter request exceeds limit".into());
        }
        let result = tokio::time::timeout(Duration::from_millis(self.timeout), async {
            self.input
                .write_all(line.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            self.input.flush().await.map_err(|e| e.to_string())?;
            let mut response = String::new();
            if self
                .output
                .read_line(&mut response)
                .await
                .map_err(|e| e.to_string())?
                == 0
            {
                return Err("adapter exited".into());
            }
            if response.len() > 8_000_000 {
                return Err("adapter response exceeds limit".into());
            }
            let value: Value = serde_json::from_str(&response).map_err(|e| e.to_string())?;
            if value["version"] != 1 || value["id"] != id {
                return Err("invalid adapter response".into());
            }
            if let Some(error) = value["error"].as_str() {
                Err(error.into())
            } else {
                Ok(value["result"].clone())
            }
        })
        .await;
        match result {
            Ok(v) => v,
            Err(_) => {
                self.child.start_kill().ok();
                Err("adapter timeout".into())
            }
        }
    }
    async fn close(&mut self) {
        self.call("close", json!({})).await.ok();
        if tokio::time::timeout(Duration::from_millis(1000), self.child.wait())
            .await
            .is_err()
        {
            self.child.start_kill().ok();
            self.child.wait().await.ok();
        }
    }
}
#[derive(Default)]
struct Stats {
    first: HashMap<String, Instant>,
    attempts: Vec<Value>,
    buffered: usize,
    peak: usize,
    reconciled: usize,
    acknowledged: std::collections::HashSet<String>,
}
struct Item {
    entry: Value,
    done: oneshot::Sender<Result<(), String>>,
}
async fn call(sink: &Arc<Mutex<Rpc>>, op: Value) -> Result<Value, String> {
    sink.lock().await.call("run", op).await
}
async fn validate(
    client: &reqwest::Client,
    c: &Value,
    id: &str,
    records: &Value,
) -> Result<Value, String> {
    client
        .post(c["oracleUrl"].as_str().ok_or("missing oracle URL")?)
        .header(
            "authorization",
            c["oracleToken"].as_str().ok_or("missing oracle token")?,
        )
        .json(&json!({"jobId":id,"records":records}))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}
async fn acquire(
    client: &reqwest::Client,
    task: &Value,
    article: &Regex,
    next: &Regex,
) -> Result<Value, String> {
    let mut url = task["url"].as_str().ok_or("missing URL")?.to_owned();
    let mut seen = std::collections::HashSet::new();
    let mut records = Vec::new();
    loop {
        if !seen.insert(url.clone())
            || seen.len() > task["pages"].as_u64().ok_or("missing page budget")? as usize
        {
            return Err("pagination exceeded contract".into());
        }
        let html = client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;
        for capture in article.captures_iter(&html) {
            records.push(json!({"id":&capture[1],"price":capture[2].parse::<i64>().map_err(|e|e.to_string())?}));
        }
        if let Some(capture) = next.captures(&html) {
            url = reqwest::Url::parse(&url)
                .map_err(|e| e.to_string())?
                .join(&capture[1])
                .map_err(|e| e.to_string())?
                .to_string()
        } else {
            break;
        }
    }
    Ok(json!(records))
}
pub async fn execute(c: Value) -> Result<Value, String> {
    let c = Arc::new(c);
    let total = Instant::now();
    let concurrency = c["concurrency"]
        .as_u64()
        .filter(|v| *v > 0 && *v <= 128)
        .ok_or("invalid concurrency")? as usize;
    let batch = c["batchJobs"]
        .as_u64()
        .filter(|v| *v > 0 && *v <= concurrency as u64)
        .ok_or("invalid batch")? as usize;
    let wait = c["maxBatchWaitMs"]
        .as_u64()
        .filter(|v| *v > 0 && *v <= 1000)
        .ok_or("invalid batch wait")?;
    let timeout = c["timeoutMs"]
        .as_u64()
        .filter(|v| *v >= 100 && *v <= 120000)
        .ok_or("invalid timeout")?;
    let jobs = c["jobs"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 10000)
        .ok_or("invalid jobs")?;
    let lease = c["leaseMs"]
        .as_u64()
        .filter(|v| *v >= 10 && *v <= 120000)
        .ok_or("invalid lease")?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(timeout))
        .build()
        .map_err(|e| e.to_string())?;
    let article = Regex::new(r#"<article data-id="([^"]+)" data-price="(\d+)""#).unwrap();
    let next = Regex::new(r#"class="next" href="([^"]+)""#).unwrap();
    let stats = Arc::new(StdMutex::new(Stats::default()));
    let faults = if let Some(f) = c["faults"].as_array() {
        f.clone()
    } else if c["fault"].is_string() {
        vec![c["fault"].clone()]
    } else {
        Vec::new()
    };
    let mut fault_index = 0;
    let mut restarts = 0;
    let mut info = Vec::new();
    let mut work_start = None;
    let mut startup_ms = 0.0;
    loop {
        let rpc = Rpc::spawn(timeout)?;
        let sink_pid = rpc.child.id();
        let sink = Arc::new(Mutex::new(rpc));
        emit(
            json!({"type":"workers-started","generation":restarts,"sinkPid":sink_pid,"coordinatorPid":std::process::id()}),
        );
        sink.lock()
            .await
            .call("prepare", json!({"path":c["path"],"leaseMs":lease}))
            .await?;
        for chunk in jobs.chunks(100) {
            call(&sink, json!({"op":"submit","jobs":chunk})).await?;
        }
        info.push(sink.lock().await.call("hello", json!({})).await?);
        if work_start.is_none() {
            startup_ms = total.elapsed().as_secs_f64() * 1000.0;
            work_start = Some(Instant::now());
        }
        let stored = call(&sink, json!({"op":"snapshot"})).await?;
        let mut expiry = 0;
        for job in stored["jobs"].as_array().unwrap() {
            let id = job["jobId"].as_str().unwrap();
            if job["state"] == "committed" {
                let verdict = validate(&client, &c, id, &job["records"]).await?;
                if verdict["status"] != "verified" {
                    return Err("persisted committed data failed oracle".into());
                }
                call(
                    &sink,
                    json!({"op":"ack","receipts":[{"jobId":id,"receipt":job["receipt"]}]}),
                )
                .await?;
                let mut s = stats.lock().unwrap();
                s.reconciled += 1;
                s.acknowledged.insert(id.into());
                emit(
                    json!({"type":"reconciled","jobId":id,"endToEndMs":s.first.get(id).map(|t|t.elapsed().as_secs_f64()*1000.0)}),
                );
            } else if job["state"] == "acked" {
                let mut s = stats.lock().unwrap();
                if s.first.contains_key(id) && s.acknowledged.insert(id.into()) {
                    s.reconciled += 1;
                    emit(
                        json!({"type":"reconciled","alreadyAcked":true,"jobId":id,"endToEndMs":s.first[id].elapsed().as_secs_f64()*1000.0}),
                    );
                }
            } else if job["state"] == "leased" {
                expiry = expiry.max(job["expires"].as_u64().unwrap_or(0));
            }
        }
        if expiry > now() {
            emit(json!({"type":"lease-expiry-wait","waitMs":expiry-now()}));
            tokio::time::sleep(Duration::from_millis(expiry - now() + 1)).await;
        }
        let failed = Arc::new(AtomicBool::new(false));
        let fault_used = Arc::new(AtomicBool::new(false));
        let fault = faults.get(fault_index).cloned().unwrap_or(Value::Null);
        let (tx, mut rx) = mpsc::channel::<Item>(concurrency);
        let mut producers = tokio::task::JoinSet::new();
        for _ in 0..concurrency {
            let (sink, stats, failed, tx, client, c, article, next) = (
                sink.clone(),
                stats.clone(),
                failed.clone(),
                tx.clone(),
                client.clone(),
                c.clone(),
                article.clone(),
                next.clone(),
            );
            producers.spawn(async move{
                while !failed.load(Ordering::SeqCst){
                    let claim=match call(&sink,json!({"op":"claim"})).await{Ok(v)=>v,Err(e)=>{failed.store(true,Ordering::SeqCst);return Err(e)}};
                    if claim["job"].is_null()&&claim["jobId"].is_null(){break}
                    let id=claim["jobId"].as_str().unwrap().to_owned();let began=Instant::now();stats.lock().unwrap().first.entry(id.clone()).or_insert(began);
                    emit(json!({"type":"job-claimed","jobId":id,"generation":restarts,"expires":claim["expires"]}));
                    if c["coordinatorCrashAt"]=="job-claimed"{tokio::time::sleep(Duration::from_millis(50)).await;}
                    let result=async{
                        let mut records=acquire(&client,&claim["task"],&article,&next).await?;
                        if c["corruptOutput"]==true{records[0]["price"]=json!(-1)}let acquisition_ms=began.elapsed().as_secs_f64()*1000.0;let validation=Instant::now();let verdict=validate(&client,&c,&id,&records).await?;let validation_ms=validation.elapsed().as_secs_f64()*1000.0;
                        if verdict["status"]!="verified"{call(&sink,json!({"op":"reject","jobId":id,"token":claim["token"],"reason":verdict["reasons"].to_string()})).await?;stats.lock().unwrap().attempts.push(json!({"jobId":id,"status":"failed","reasons":verdict["reasons"]}));return Ok(())}
                        let queued=Instant::now();let (done,wait)=oneshot::channel();{let mut s=stats.lock().unwrap();s.buffered+=1;s.peak=s.peak.max(s.buffered);if s.buffered>concurrency{return Err("bounded result queue exceeded".into())}}
                        let count=records.as_array().unwrap().len();tx.send(Item{entry:json!({"jobId":id,"token":claim["token"],"records":records}),done}).await.map_err(|_|"writer stopped".to_string())?;wait.await.map_err(|_|"writer stopped".to_string())??;
                        let mut s=stats.lock().unwrap();let attempt=json!({"jobId":id,"status":"acked","generation":restarts,"records":count,"acquisitionMs":acquisition_ms,"validationMs":validation_ms,"sinkWaitCommitAckMs":queued.elapsed().as_secs_f64()*1000.0,"endToEndMs":s.first[&id].elapsed().as_secs_f64()*1000.0});s.acknowledged.insert(id.clone());s.attempts.push(attempt.clone());let mut event=attempt;event["type"]=json!("job-acked");emit(event);Ok::<(),String>(())
                    }.await;
                    if let Err(e)=result{failed.store(true,Ordering::SeqCst);stats.lock().unwrap().attempts.push(json!({"jobId":id,"status":"aborted","error":e}));return Err(e)}
                }Ok(())
            });
        }
        drop(tx);
        let writer = {
            let (sink, stats, failed, fault_used) = (
                sink.clone(),
                stats.clone(),
                failed.clone(),
                fault_used.clone(),
            );
            let delay = c["writerDelayMs"].as_u64().unwrap_or(0);
            let checkpoint = c["coordinatorCrashAt"].clone();
            tokio::spawn(async move {
                while let Some(first) = rx.recv().await {
                    let mut items = vec![first];
                    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
                    while items.len() < batch {
                        match tokio::time::timeout_at(deadline, rx.recv()).await {
                            Ok(Some(item)) => items.push(item),
                            _ => break,
                        }
                    }
                    let result=async{
                    if delay>0{tokio::time::sleep(Duration::from_millis(delay)).await;}let used=if !fault.is_null()&&!fault_used.swap(true,Ordering::SeqCst){fault.clone()}else{Value::Null};let begin=Instant::now();
                    let commit=call(&sink,json!({"op":"commit","entries":items.iter().map(|i|i.entry.clone()).collect::<Vec<_>>(),"fault":if ["before-commit","after-commit","write-error","lock-held","lock-starved"].iter().any(|f|used==*f){used.clone()}else{Value::Null}})).await?;
                    let receipts=commit["receipts"].as_array().filter(|r|r.len()==items.len()).ok_or("invalid commit receipts")?;let mut seen=std::collections::HashSet::new();for r in receipts{if !r["receipt"].is_string()||!seen.insert(r["jobId"].clone().to_string())||!items.iter().any(|i|i.entry["jobId"]==r["jobId"]){return Err("invalid commit receipts".into())}}
                    let commit_ms=begin.elapsed().as_secs_f64()*1000.0;emit(json!({"type":"batch-committed","jobs":items.len(),"commitMs":commit_ms}));if checkpoint=="batch-committed"{tokio::time::sleep(Duration::from_millis(50)).await;}let ack_start=Instant::now();
                    let ack=call(&sink,json!({"op":"ack","receipts":receipts,"fault":if used=="during-ack"||used=="after-ack"{used}else{Value::Null}})).await?;if ack["acknowledged"]!=items.len(){return Err("invalid ACK".into())}emit(json!({"type":"batch-acked","jobs":items.len(),"commitMs":commit_ms,"ackMs":ack_start.elapsed().as_secs_f64()*1000.0}));Ok::<(),String>(())
                }.await;
                    stats.lock().unwrap().buffered -= items.len();
                    for item in items {
                        item.done.send(result.clone()).ok();
                    }
                    if let Err(e) = result {
                        failed.store(true, Ordering::SeqCst);
                        return Err(e);
                    }
                }
                Ok(())
            })
        };
        let mut error = None;
        while let Some(result) = producers.join_next().await {
            match result {
                Ok(Err(e)) => error = Some(e),
                Err(e) => error = Some(e.to_string()),
                _ => {}
            }
        }
        match writer.await {
            Ok(Err(e)) => error = Some(e),
            Err(e) => error = Some(e.to_string()),
            _ => {}
        }
        if fault_used.load(Ordering::SeqCst) {
            fault_index += 1;
        }
        sink.lock().await.close().await;
        if let Some(error) = error {
            if restarts >= c["restartLimit"].as_u64().unwrap_or(4) {
                return Err(error);
            }
            restarts += 1;
            stats.lock().unwrap().buffered = 0;
            emit(json!({"type":"restart","generation":restarts,"error":error}));
            continue;
        }
        break;
    }
    let work_ms = work_start.unwrap().elapsed().as_secs_f64() * 1000.0;
    let finalization = Instant::now();
    let mut reopened = Rpc::spawn(timeout)?;
    reopened
        .call("prepare", json!({"path":c["path"],"leaseMs":lease}))
        .await?;
    let snapshot = reopened.call("run", json!({"op":"snapshot"})).await?;
    reopened.close().await;
    let mut verdicts = Vec::new();
    let mut verified = 0;
    let mut records = 0;
    for job in snapshot["jobs"].as_array().unwrap() {
        let id = job["jobId"].as_str().unwrap();
        let verdict = if job["state"] == "acked" {
            validate(&client, &c, id, &job["records"]).await?
        } else {
            json!({"status":"failed","reasons":[format!("job is {}",job["state"])]})
        };
        if verdict["status"] == "verified" {
            verified += 1;
            records += job["records"].as_array().unwrap().len()
        }
        verdicts.push(json!({"jobId":id,"status":verdict["status"],"reasons":verdict["reasons"]}));
    }
    if snapshot["integrity"] != json!(["ok"]) || snapshot["foreignKeys"] != json!([]) {
        return Err("workflow structural integrity failed".into());
    }
    let finalize_ms = finalization.elapsed().as_secs_f64() * 1000.0;
    let s = stats.lock().unwrap();
    Ok(
        json!({"runtime":"rust","snapshot":snapshot,"verdicts":verdicts,"attempts":s.attempts,"info":info,"startupMs":startup_ms,"workMs":work_ms,"finalizationMs":finalize_ms,"totalMs":total.elapsed().as_secs_f64()*1000.0,"verifiedJobs":verified,"verifiedRecords":records,"verifiedJobsPerSecond":verified as f64*1000.0/(work_ms+finalize_ms),"peakBufferedJobs":s.peak,"restarts":restarts,"reconciledJobs":s.reconciled,"tokioWorkerThreads":1}),
    )
}
