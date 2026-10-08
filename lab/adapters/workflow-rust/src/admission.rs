// Open-arrival admission and multi-domain scheduling (D-durable-runtime, D-scheduling). A job is acknowledged only once its
// admission is committed to the journal; a full queue rejects it explicitly with a retry hint. The dispatcher never runs more
// than the per-domain limit at once and, under the fair policy, gives each domain its own bounded queue and its turn.
use crate::ledger::{Database, now, open_journal};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Notify, oneshot};

#[derive(Clone, Copy, PartialEq)]
enum Policy { FifoGlobal, FairDomain }
struct Config { policy: Policy, capacity: usize, domain_capacity: usize, workers: usize, domain_limit: usize, timeout_ms: u64 }
struct Job { id: String, domain: String, url: String }
// Jobs admitted (committed) and waiting; `reserved` counts admissions still being committed, so the bound holds before the ack.
#[derive(Default)]
struct State { global: VecDeque<Job>, domains: HashMap<String, VecDeque<Job>>, order: Vec<String>, cursor: usize, queued: usize, queued_by: HashMap<String, usize>, reserved: usize, reserved_by: HashMap<String, usize>, running: usize, running_by: HashMap<String, usize>, peak_queued: usize, peak_running_by: HashMap<String, usize>, admitted: u64, rejected: u64, done: u64, failed: u64 }
enum Write_ { Admit { id: String, domain: String, url: String }, Finish { id: String, records: Value, error: Option<String> } }
type Batch = (Write_, oneshot::Sender<Result<(), String>>);

fn emit(response: Value) { let mut out = std::io::stdout().lock(); writeln!(out, "{response}").ok(); out.flush().ok(); }

// The journal writer: one thread owns the connection and commits whatever arrived while the previous commit ran, in one transaction.
fn writer(db: Database, rx: mpsc::Receiver<Batch>, batch_max: usize) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while batch.len() < batch_max { match rx.try_recv() { Ok(next) => batch.push(next), Err(_) => break } }
        let result = db.tx(|| {
            for (op, _) in &batch {
                match op {
                    Write_::Admit { id, domain, url } => { db.query("INSERT INTO admitted(jobId,domain,url,state,admittedAt) VALUES(?,?,?,'queued',?)", json!([id, domain, url, now()]))?; }
                    Write_::Finish { id, records, error } => {
                        db.query("UPDATE admitted SET state=?,finishedAt=?,error=? WHERE jobId=? AND state='queued'", json!([if error.is_some() { "failed" } else { "done" }, now(), error, id]))?;
                        for record in records.as_array().into_iter().flatten() { db.query("INSERT INTO results(jobId,id,price) VALUES(?,?,?)", json!([id, record["id"], record["price"]]))?; }
                    }
                }
            }
            Ok(())
        });
        for (_, reply) in batch { let _ = reply.send(result.clone()); }
    }
}

async fn journal(tx: &mpsc::Sender<Batch>, op: Write_) -> Result<(), String> {
    let (reply, wait) = oneshot::channel();
    tx.send((op, reply)).map_err(|_| "journal writer stopped".to_string())?;
    wait.await.map_err(|_| "journal writer stopped".to_string())?
}

fn admit_or_reject(state: &mut State, config: &Config, domain: &str) -> bool {
    let pending = state.queued + state.reserved; let pending_domain = state.queued_by.get(domain).copied().unwrap_or(0) + state.reserved_by.get(domain).copied().unwrap_or(0);
    if pending >= config.capacity || (config.policy == Policy::FairDomain && pending_domain >= config.domain_capacity) { state.rejected += 1; return false; }
    state.reserved += 1; *state.reserved_by.entry(domain.to_owned()).or_default() += 1; true
}

// The next job a free worker may run: the oldest whose domain has room (FIFO), or the next domain in turn with work and room (fair).
fn next_job(state: &mut State, config: &Config) -> Option<Job> {
    if state.running >= config.workers { return None; }
    let has_room = |s: &State, d: &str| s.running_by.get(d).copied().unwrap_or(0) < config.domain_limit;
    let job = match config.policy {
        Policy::FifoGlobal => { let index = state.global.iter().position(|j| has_room(state, &j.domain))?; state.global.remove(index) }
        Policy::FairDomain => {
            let n = state.order.len(); let mut found = None;
            for step in 0..n { let d = state.order[(state.cursor + step) % n].clone(); if has_room(state, &d) && state.domains.get(&d).is_some_and(|q| !q.is_empty()) { state.cursor = (state.cursor + step + 1) % n; found = state.domains.get_mut(&d).unwrap().pop_front(); break; } }
            found
        }
    }?;
    state.queued -= 1; *state.queued_by.get_mut(&job.domain).unwrap() -= 1;
    state.running += 1; let running = state.running_by.entry(job.domain.clone()).or_default(); *running += 1;
    let peak = state.peak_running_by.entry(job.domain.clone()).or_default(); *peak = (*peak).max(*running);
    Some(job)
}

async fn fetch(client: &reqwest::Client, url: &str, article: &Regex) -> Result<Value, String> {
    let html = client.get(url).send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    let records: Result<Vec<Value>, String> = article.captures_iter(&html).map(|c| Ok(json!({"id": &c[1], "price": c[2].parse::<i64>().map_err(|e| e.to_string())?}))).collect();
    let records = records?; if records.is_empty() { return Err("no records".into()); } Ok(json!(records))
}

fn dispatch(state: Arc<Mutex<State>>, config: Arc<Config>, wake: Arc<Notify>, tx: mpsc::Sender<Batch>, client: reqwest::Client, article: Regex) {
    tokio::spawn(async move {
        loop {
            let job = { let mut s = state.lock().unwrap(); next_job(&mut s, &config) };
            let Some(job) = job else { wake.notified().await; continue; };
            let (state, wake, tx, client, article) = (state.clone(), wake.clone(), tx.clone(), client.clone(), article.clone());
            tokio::spawn(async move {
                let outcome = fetch(&client, &job.url, &article).await;
                let (records, error) = match outcome { Ok(r) => (r, None), Err(e) => (json!([]), Some(e)) };
                let failed = error.is_some(); let committed = journal(&tx, Write_::Finish { id: job.id.clone(), records, error }).await;
                { let mut s = state.lock().unwrap(); s.running -= 1; *s.running_by.get_mut(&job.domain).unwrap() -= 1; if failed || committed.is_err() { s.failed += 1 } else { s.done += 1 } }
                wake.notify_one();
            });
        }
    });
}

pub fn serve() {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime");
    runtime.block_on(async {
        let state = Arc::new(Mutex::new(State::default())); let wake = Arc::new(Notify::new());
        let mut setup: Option<(Arc<Config>, mpsc::Sender<Batch>, String)> = None;
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(m) = serde_json::from_str::<Value>(&line) else { break };
            if m["version"] != 1 || !m["id"].is_u64() { break; }
            let id = m["id"].clone(); let method = m["method"].as_str().unwrap_or("").to_owned(); let p = m["params"].clone();
            let reply = move |result: Result<Value, String>| emit(match result { Ok(v) => json!({"version":1,"id":id,"result":v}), Err(e) => json!({"version":1,"id":id,"error":e}) });
            match method.as_str() {
                "hello" => reply(Ok(json!({"backend":"admission-rust","protocol":1,"policies":["fifo-global","fair-domain"]}))),
                "prepare" => {
                    let result = (|| -> Result<Value, String> {
                        if setup.is_some() { return Err("already prepared".into()); }
                        let int = |k: &str, min: u64, max: u64| p[k].as_u64().filter(|v| *v >= min && *v <= max).ok_or(format!("invalid {k}"));
                        let policy = match p["policy"].as_str() { Some("fifo-global") => Policy::FifoGlobal, Some("fair-domain") => Policy::FairDomain, _ => return Err("invalid policy".into()) };
                        let config = Arc::new(Config { policy, capacity: int("capacity", 1, 100_000)? as usize, domain_capacity: int("domainCapacity", 1, 100_000)? as usize, workers: int("workers", 1, 512)? as usize, domain_limit: int("domainLimit", 1, 512)? as usize, timeout_ms: int("timeoutMs", 100, 120_000)? });
                        let path = p["path"].as_str().ok_or("missing path")?.to_owned(); let batch_max = int("batchMax", 1, 4096)? as usize;
                        // The connection lives on the writer thread, which opens it and creates the journal tables.
                        let (tx, rx) = mpsc::channel::<Batch>(); let (ready_tx, ready) = mpsc::channel::<Result<(), String>>(); let journal_path = path.clone();
                        std::thread::spawn(move || {
                            let opened = open_journal(&journal_path).and_then(|db| { db.exec("CREATE TABLE IF NOT EXISTS admitted(jobId TEXT PRIMARY KEY,domain TEXT NOT NULL,url TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('queued','done','failed')),admittedAt INTEGER NOT NULL,finishedAt INTEGER,error TEXT);CREATE TABLE IF NOT EXISTS results(jobId TEXT NOT NULL REFERENCES admitted(jobId),id TEXT NOT NULL,price INTEGER NOT NULL,PRIMARY KEY(jobId,id));")?; Ok(db) });
                            match opened { Ok(db) => { let _ = ready_tx.send(Ok(())); writer(db, rx, batch_max) } Err(e) => { let _ = ready_tx.send(Err(e)); } }
                        });
                        ready.recv().map_err(|_| "journal writer did not start".to_string())??;
                        let client = reqwest::Client::builder().timeout(Duration::from_millis(config.timeout_ms)).pool_max_idle_per_host(config.workers).build().map_err(|e| e.to_string())?;
                        dispatch(state.clone(), config.clone(), wake.clone(), tx.clone(), client, Regex::new(r#"<article data-id="([^"]+)" data-price="(\d+)""#).unwrap());
                        setup = Some((config, tx, path)); Ok(json!({"ready":true}))
                    })();
                    reply(result)
                }
                "run" => {
                    let Some((config, tx, path)) = setup.clone() else { reply(Err("not prepared".into())); continue };
                    let (state, wake) = (state.clone(), wake.clone());
                    tokio::spawn(async move {
                        let result = async {
                            match p["op"].as_str() {
                                // Admission: reserve a place under the bound, commit, then queue and acknowledge.
                                Some("submit") => {
                                    let job = Job { id: p["job"]["jobId"].as_str().ok_or("missing jobId")?.to_owned(), domain: p["job"]["domain"].as_str().ok_or("missing domain")?.to_owned(), url: p["job"]["url"].as_str().ok_or("missing url")?.to_owned() };
                                    if !admit_or_reject(&mut state.lock().unwrap(), &config, &job.domain) { return Ok(json!({"admitted":false,"code":"E_CAPACITY","retryAfterMs":100})); }
                                    let committed = journal(&tx, Write_::Admit { id: job.id.clone(), domain: job.domain.clone(), url: job.url.clone() }).await;
                                    let mut s = state.lock().unwrap(); s.reserved -= 1; *s.reserved_by.get_mut(&job.domain).unwrap() -= 1;
                                    if let Err(e) = committed { s.rejected += 1; return Ok(json!({"admitted":false,"code":"E_JOURNAL","error":e})); }
                                    s.admitted += 1; s.queued += 1; *s.queued_by.entry(job.domain.clone()).or_default() += 1; s.peak_queued = s.peak_queued.max(s.queued);
                                    if !s.order.contains(&job.domain) { s.order.push(job.domain.clone()); }
                                    match config.policy { Policy::FifoGlobal => s.global.push_back(job), Policy::FairDomain => s.domains.entry(job.domain.clone()).or_default().push_back(job) }
                                    drop(s); wake.notify_one(); Ok(json!({"admitted":true}))
                                }
                                Some("drain") => {
                                    let deadline = std::time::Instant::now() + Duration::from_millis(p["timeoutMs"].as_u64().unwrap_or(30_000));
                                    loop { { let s = state.lock().unwrap(); if s.queued + s.reserved + s.running == 0 { break; } } if std::time::Instant::now() > deadline { return Err("drain timeout".to_string()); } tokio::time::sleep(Duration::from_millis(5)).await; }
                                    let s = state.lock().unwrap();
                                    Ok(json!({"admitted":s.admitted,"rejected":s.rejected,"done":s.done,"failed":s.failed,"peakQueued":s.peak_queued,"peakRunningByDomain":s.peak_running_by}))
                                }
                                Some("snapshot") => {
                                    let db = open_journal(&path)?;
                                    let mut jobs = db.query("SELECT * FROM admitted ORDER BY rowid", json!([]))?;
                                    for job in jobs.as_array_mut().unwrap() { job["records"] = db.query("SELECT id,price FROM results WHERE jobId=? ORDER BY id", json!([job["jobId"]]))?; }
                                    Ok(json!({"jobs":jobs,"integrity":db.query("PRAGMA integrity_check", json!([]))?}))
                                }
                                _ => Err("unknown op".to_string()),
                            }
                        }.await;
                        reply(result)
                    });
                }
                "close" => { reply(Ok(json!({"closed":true}))); break; }
                _ => reply(Err("unknown method".into())),
            }
        }
    });
    runtime.shutdown_timeout(Duration::from_millis(500));
}
