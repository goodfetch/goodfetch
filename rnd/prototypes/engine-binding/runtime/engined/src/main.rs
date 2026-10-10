//! engined: the stub engine behind newline-delimited JSON-RPC 2.0 (throwaway).
//!   engined stdio            sidecar: one SDK process spawns it, talks over stdin/stdout
//!   engined serve <sock>     daemon: several SDK processes connect over a Unix socket
//!   engined inspect <db> <interval_ms>   read-only `status`-like poller until stdin closes
//!   engined check <db>       integrity_check + row counts per run/status
//! Client->engine: initialize, run (long request), $/cancelRequest.
//! Engine->client: handle (request per page), events (notification), $/cancelRequest.
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engine_core::{self as ec, BoxFut, CancellationToken, Event, Host, PageMsg, RunOpts};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::{mpsc, oneshot};

const PROTOCOL: &str = "1.0";

enum Out {
    Line(Vec<u8>),
    Flush(Vec<u8>, oneshot::Sender<()>),
}

struct Sess {
    out: mpsc::UnboundedSender<Out>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    next_id: AtomicU64,
    runs: Mutex<HashMap<String, CancellationToken>>,
    owner: String,
}

fn line(v: &Value) -> Vec<u8> {
    let mut b = serde_json::to_vec(v).unwrap();
    b.push(b'\n');
    b
}

impl Sess {
    fn send(&self, v: &Value) {
        let _ = self.out.send(Out::Line(line(v)));
    }
}

struct RpcHost {
    s: Arc<Sess>,
}

/// Dropped before the reply arrived (timeout or run cancelled): tell the client.
struct CancelGuard {
    s: Arc<Sess>,
    id: u64,
    armed: bool,
}
impl Drop for CancelGuard {
    fn drop(&mut self) {
        if self.armed {
            self.s.pending.lock().unwrap().remove(&self.id);
            self.s.send(&json!({"jsonrpc": "2.0", "method": "$/cancelRequest", "params": {"id": self.id}}));
        }
    }
}

impl Host for RpcHost {
    fn handle(&self, p: PageMsg) -> BoxFut<Result<Value, String>> {
        let s = self.s.clone();
        Box::pin(async move {
            let id = s.next_id.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = oneshot::channel();
            s.pending.lock().unwrap().insert(id, tx);
            let mut params = json!({"run_id": p.run_id, "idx": p.idx, "url": p.url, "blob": p.blob, "t0_us": p.t0_us});
            if let Some(b) = p.body {
                params["body"] = Value::String(String::from_utf8(b).unwrap_or_default());
            }
            s.send(&json!({"jsonrpc": "2.0", "id": id, "method": "handle", "params": params}));
            let mut g = CancelGuard { s: s.clone(), id, armed: true };
            let r = rx.await;
            g.armed = false;
            match r {
                Ok(mut m) => {
                    if m.get("result").is_some() {
                        Ok(m["result"].take())
                    } else {
                        let e = &m["error"];
                        Err(format!("{}\n{}", e["message"].as_str().unwrap_or("?"), e["data"]["stack"].as_str().unwrap_or("")))
                    }
                }
                Err(_) => Err("client gone".into()),
            }
        })
    }
    fn emit(&self, evs: Vec<Event>) -> BoxFut<()> {
        // Default: unbounded outgoing queue, engine never waits.
        self.s.send(&json!({"jsonrpc": "2.0", "method": "events", "params": evs}));
        Box::pin(async {})
    }
    fn emit_await(&self, evs: Vec<Event>) -> BoxFut<()> {
        // Completes once written and flushed to the socket: kernel-buffer backpressure.
        let (tx, rx) = oneshot::channel();
        let _ = self.s.out.send(Out::Flush(line(&json!({"jsonrpc": "2.0", "method": "events", "params": evs})), tx));
        Box::pin(async move {
            let _ = rx.await;
        })
    }
}

async fn write_loop<W: AsyncWrite + Unpin>(w: W, mut rx: mpsc::UnboundedReceiver<Out>) {
    let mut w = BufWriter::with_capacity(1 << 16, w);
    while let Some(m) = rx.recv().await {
        let ok = match m {
            Out::Line(b) => w.write_all(&b).await.is_ok(),
            Out::Flush(b, ack) => {
                let ok = w.write_all(&b).await.is_ok() && w.flush().await.is_ok();
                let _ = ack.send(());
                ok
            }
        };
        if !ok {
            break;
        }
        if rx.is_empty() && w.flush().await.is_err() {
            break;
        }
    }
}

async fn session<R: AsyncRead + Unpin, W: AsyncWrite + Unpin + Send + 'static>(r: R, w: W, owner: String) {
    let (tx, rx) = mpsc::unbounded_channel();
    let wt = tokio::spawn(write_loop(w, rx));
    let s = Arc::new(Sess { out: tx, pending: Default::default(), next_id: AtomicU64::new(1), runs: Default::default(), owner: owner.clone() });
    let mut tasks = tokio::task::JoinSet::new();
    let mut lease_dbs = HashSet::new();
    let mut reader = BufReader::with_capacity(1 << 20, r);
    let mut buf = Vec::with_capacity(1 << 16);
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            _ => {}
        }
        let Ok(mut msg) = serde_json::from_slice::<Value>(&buf) else { continue };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        match msg.get("method").and_then(|m| m.as_str()).map(str::to_owned).as_deref() {
            Some("initialize") => {
                let pv = msg["params"]["protocolVersion"].as_str().unwrap_or("").to_owned();
                if pv.split('.').next() == PROTOCOL.split('.').next() {
                    s.send(&json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": PROTOCOL, "engineVersion": env!("CARGO_PKG_VERSION"), "pid": std::process::id(), "owner": owner}}));
                } else {
                    s.send(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32002, "message": format!("protocol version mismatch: client {pv}, engine {PROTOCOL}"), "data": {"engineProtocol": PROTOCOL}}}));
                }
            }
            Some("run") => {
                let mut opts: RunOpts = serde_json::from_value(msg["params"].take()).unwrap_or_default();
                if opts.leases {
                    opts.owner = owner.clone();
                    lease_dbs.insert(opts.db_path.clone());
                }
                let tok = CancellationToken::new();
                s.runs.lock().unwrap().insert(id.to_string(), tok.clone());
                let s2 = s.clone();
                tasks.spawn(async move {
                    let host = Arc::new(RpcHost { s: s2.clone() });
                    let sum = ec::run(opts, host, tok).await;
                    s2.runs.lock().unwrap().remove(&id.to_string());
                    s2.send(&json!({"jsonrpc": "2.0", "id": id, "result": sum}));
                });
            }
            Some("$/cancelRequest") => {
                if let Some(t) = s.runs.lock().unwrap().get(&msg["params"]["id"].to_string()) {
                    t.cancel();
                }
            }
            Some(_) => s.send(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}})),
            None => {
                if let Some(tx) = id.as_u64().and_then(|i| s.pending.lock().unwrap().remove(&i)) {
                    let _ = tx.send(msg);
                }
            }
        }
    }
    // Client gone (EOF): stop its runs, fail its pending handler calls, release its leases.
    let t = Instant::now();
    for tok in s.runs.lock().unwrap().values() {
        tok.cancel();
    }
    s.pending.lock().unwrap().clear();
    while tasks.join_next().await.is_some() {}
    let mut requeued = 0;
    for db in &lease_dbs {
        requeued += ec::writer(db).requeue(&owner).await;
    }
    eprintln!("[engined] session {owner} closed; requeued {requeued} leased items in {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    wt.abort();
}

fn inspect(db: &str, interval_ms: u64) {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut b);
        s2.store(true, Ordering::Relaxed);
    });
    let mut conn = None;
    let (mut lat, mut errors, mut last) = (Vec::new(), 0u64, Value::Null);
    while !stop.load(Ordering::Relaxed) {
        if conn.is_none() {
            conn = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok();
            if let Some(c) = &conn {
                c.busy_timeout(Duration::from_millis(5000)).unwrap();
            }
        }
        if let Some(c) = &conn {
            let t = Instant::now();
            let r: rusqlite::Result<Vec<(String, i64)>> = c
                .prepare_cached("SELECT status, count(*) FROM journal GROUP BY status")
                .and_then(|mut st| st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect());
            match r {
                Ok(rows) => {
                    lat.push(t.elapsed().as_secs_f64() * 1000.0);
                    last = json!(rows);
                }
                Err(_) => errors += 1,
            }
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| if lat.is_empty() { 0.0 } else { lat[((lat.len() as f64 * q) as usize).min(lat.len() - 1)] };
    println!("{}", json!({"queries": lat.len(), "p50_ms": p(0.5), "p99_ms": p(0.99), "max_ms": p(1.0), "errors": errors, "last": last}));
}

fn check(db: &str) {
    let c = ec::open_db(db);
    let integrity: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
    let mut st = c.prepare("SELECT run_id, status, count(*) FROM journal GROUP BY 1, 2").unwrap();
    let counts: Vec<(String, String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(|r| r.unwrap()).collect();
    let mut st = c.prepare("SELECT run_id, idx FROM journal WHERE status='done'").unwrap();
    let done: Vec<(String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    println!("{}", json!({"integrity": integrity, "counts": counts, "done": done}));
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("stdio") => session(tokio::io::stdin(), tokio::io::stdout(), format!("sidecar-{}", std::process::id())).await,
        Some("serve") => {
            let _ = std::fs::remove_file(&args[2]);
            let l = tokio::net::UnixListener::bind(&args[2]).unwrap();
            eprintln!("[engined] daemon pid {} listening on {}", std::process::id(), args[2]);
            let mut n = 0;
            loop {
                let (st, _) = l.accept().await.unwrap();
                n += 1;
                let (r, w) = st.into_split();
                tokio::spawn(session(r, w, format!("conn-{n}")));
            }
        }
        Some("inspect") => inspect(&args[2], args.get(3).and_then(|s| s.parse().ok()).unwrap_or(100)),
        Some("check") => check(&args[2]),
        _ => eprintln!("usage: engined stdio | serve <sock> | inspect <db> [ms] | check <db>"),
    }
}
