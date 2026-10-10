//! Stub engine for the binding prototype (throwaway).
//! A run generates fake pages, stores each body in a content-addressed blob dir,
//! calls the user handler (inline body or blob reference), journals each page in
//! one SQLite file (WAL, synchronous=FULL, one writer thread per file, group
//! commit), emits events and honours a cancellation token between steps.
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
pub use tokio_util::sync::CancellationToken;

pub type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub fn now_us() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as f64 / 1000.0
}

pub fn peak_rss_bytes() -> u64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    if cfg!(target_os = "macos") { ru.ru_maxrss as u64 } else { ru.ru_maxrss as u64 * 1024 }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct RunOpts {
    pub run_id: String,
    pub n_pages: u64,
    pub page_bytes: usize,
    pub db_path: String,
    pub blob_dir: String,
    /// true: body delivered inline (Buffer/bytes/JSON string); false: by reference only.
    pub inline: bool,
    pub handler_timeout_ms: u64,
    pub concurrency: usize,
    /// "default" (binding's natural delivery), "bounded" (engine queue, block when full), "drop".
    pub event_mode: String,
    pub event_cap: usize,
    pub event_batch: usize,
    pub item_events: bool,
    pub progress_every: u64,
    /// >0: no pages, just emit this many `tick` events (S1).
    pub stream_events: u64,
    pub event_bytes: usize,
    pub leases: bool,
    pub owner: String,
    pub timeline: bool,
}

impl Default for RunOpts {
    fn default() -> Self {
        RunOpts {
            run_id: "run".into(),
            n_pages: 10,
            page_bytes: 1024,
            db_path: "data/engine.db".into(),
            blob_dir: "data/blobs".into(),
            inline: false,
            handler_timeout_ms: 0,
            concurrency: 4,
            event_mode: "default".into(),
            event_cap: 1024,
            event_batch: 1,
            item_events: true,
            progress_every: 100,
            stream_events: 0,
            event_bytes: 64,
            leases: false,
            owner: "local".into(),
            timeline: false,
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Event {
    pub kind: String,
    pub run_id: String,
    pub seq: u64,
    pub idx: i64,
    pub t_us: f64,
    pub data: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct BlobRef {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

pub struct PageMsg {
    pub run_id: String,
    pub idx: u64,
    pub url: String,
    pub body: Option<Vec<u8>>,
    pub blob: BlobRef,
    pub t0_us: f64,
}

/// What a binding must provide: call the user handler, deliver events.
pub trait Host: Send + Sync + 'static {
    fn handle(&self, page: PageMsg) -> BoxFut<Result<Value, String>>;
    /// The binding's natural delivery (may be fire-and-forget or blocking).
    fn emit(&self, evs: Vec<Event>) -> BoxFut<()>;
    /// Delivery that completes only once the consumer has taken the events.
    fn emit_await(&self, evs: Vec<Event>) -> BoxFut<()> {
        self.emit(evs)
    }
}

#[derive(Serialize, Default, Debug)]
pub struct Summary {
    pub run_id: String,
    pub pages_done: u64,
    pub pages_timeout: u64,
    pub pages_error: u64,
    pub cancelled: bool,
    pub events_emitted: u64,
    pub events_dropped: u64,
    pub t_start_us: f64,
    pub t_end_us: f64,
    pub t_first_emit_us: f64,
    pub t_last_emit_us: f64,
    pub commit_p50_us: f64,
    pub commit_p99_us: f64,
    pub writer_commits: u64,
    pub writer_busy: u64,
    pub writer_lock_wait_ms: f64,
    pub deliver_p50_us: f64,
    pub body_p50_us: f64,
    pub rt_p50_us: f64,
    pub errors: Vec<String>,
    pub timeline: Vec<(u64, String, f64)>,
    pub engine_peak_rss: u64,
}

// ---------------------------------------------------------------- SQLite writer

enum WOp {
    Put { run_id: String, idx: u64, status: &'static str, sha: String, owner: String, enq: Instant, ack: oneshot::Sender<f64> },
    Requeue { owner: String, ack: oneshot::Sender<u64> },
}

#[derive(Default)]
pub struct WStats {
    pub commits: AtomicU64,
    pub busy: AtomicU64,
    pub lock_wait_us: AtomicU64,
}

pub struct Writer {
    tx: std::sync::mpsc::Sender<WOp>,
    pub stats: Arc<WStats>,
}

static WRITERS: LazyLock<Mutex<HashMap<String, Arc<Writer>>>> = LazyLock::new(Default::default);

/// One writer (thread + connection) per database file per process.
pub fn writer(db_path: &str) -> Arc<Writer> {
    let mut m = WRITERS.lock().unwrap();
    m.entry(db_path.to_string())
        .or_insert_with(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let stats = Arc::new(WStats::default());
            let (s2, p) = (stats.clone(), db_path.to_string());
            std::thread::Builder::new().name("journal-writer".into()).spawn(move || writer_thread(&p, rx, s2)).unwrap();
            Arc::new(Writer { tx, stats })
        })
        .clone()
}

impl Writer {
    pub async fn put(&self, run_id: &str, idx: u64, status: &'static str, sha: &str, owner: &str) -> f64 {
        let (ack, rx) = oneshot::channel();
        let _ = self.tx.send(WOp::Put { run_id: run_id.into(), idx, status, sha: sha.into(), owner: owner.into(), enq: Instant::now(), ack });
        rx.await.unwrap_or(-1.0)
    }
    /// Lease release: in-flight items of a dead owner become re-queueable.
    pub async fn requeue(&self, owner: &str) -> u64 {
        let (ack, rx) = oneshot::channel();
        let _ = self.tx.send(WOp::Requeue { owner: owner.into(), ack });
        rx.await.unwrap_or(0)
    }
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(e.sqlite_error_code(), Some(ErrorCode::DatabaseBusy) | Some(ErrorCode::DatabaseLocked))
}

fn retry<T>(stats: &WStats, mut f: impl FnMut() -> rusqlite::Result<T>) -> T {
    loop {
        match f() {
            Ok(v) => return v,
            Err(e) if is_busy(&e) => {
                stats.busy.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => panic!("sqlite: {e}"),
        }
    }
}

pub fn open_db(path: &str) -> Connection {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path).unwrap();
    conn.busy_timeout(Duration::from_millis(5000)).unwrap();
    conn
}

fn writer_thread(path: &str, rx: std::sync::mpsc::Receiver<WOp>, stats: Arc<WStats>) {
    let conn = open_db(path);
    retry(&stats, || conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0)));
    retry(&stats, || conn.pragma_update(None, "synchronous", "FULL"));
    retry(&stats, || {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS journal(run_id TEXT NOT NULL, idx INTEGER NOT NULL, status TEXT NOT NULL,
             sha TEXT, owner TEXT, ts REAL, PRIMARY KEY(run_id, idx))",
        )
    });
    enum Ack {
        Put(oneshot::Sender<f64>, Instant),
        Req(oneshot::Sender<u64>, u64),
    }
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while batch.len() < 512 {
            match rx.try_recv() {
                Ok(op) => batch.push(op),
                Err(_) => break,
            }
        }
        let t = Instant::now();
        retry(&stats, || conn.execute_batch("BEGIN IMMEDIATE"));
        stats.lock_wait_us.fetch_add(t.elapsed().as_micros() as u64, Ordering::Relaxed);
        let mut acks = Vec::with_capacity(batch.len());
        {
            let mut st = conn
                .prepare_cached(
                    "INSERT INTO journal(run_id, idx, status, sha, owner, ts) VALUES(?1,?2,?3,?4,?5,?6)
                     ON CONFLICT(run_id, idx) DO UPDATE SET status=excluded.status, sha=excluded.sha, owner=excluded.owner, ts=excluded.ts",
                )
                .unwrap();
            for op in batch {
                match op {
                    WOp::Put { run_id, idx, status, sha, owner, enq, ack } => {
                        st.execute(params![run_id, idx as i64, status, sha, owner, now_us()]).unwrap();
                        acks.push(Ack::Put(ack, enq));
                    }
                    WOp::Requeue { owner, ack } => {
                        let n = conn
                            .execute("UPDATE journal SET status='queued', owner=NULL WHERE owner=?1 AND status='leased'", params![owner])
                            .unwrap();
                        acks.push(Ack::Req(ack, n as u64));
                    }
                }
            }
        }
        retry(&stats, || conn.execute_batch("COMMIT"));
        stats.commits.fetch_add(1, Ordering::Relaxed);
        for a in acks {
            match a {
                Ack::Put(tx, enq) => {
                    let _ = tx.send(enq.elapsed().as_nanos() as f64 / 1000.0);
                }
                Ack::Req(tx, n) => {
                    let _ = tx.send(n);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- events

struct Emitter {
    mode: String,
    run_id: String,
    host: Arc<dyn Host>,
    tx: Mutex<Option<mpsc::Sender<Event>>>,
    pump: Mutex<Option<tokio::task::JoinHandle<()>>>,
    seq: AtomicU64,
    dropped: AtomicU64,
    first: Mutex<f64>,
    last: Mutex<f64>,
}

impl Emitter {
    fn new(o: &RunOpts, host: Arc<dyn Host>) -> Self {
        let (mut tx, mut pump) = (None, None);
        if o.event_mode != "default" {
            let (t, mut rx) = mpsc::channel::<Event>(o.event_cap.max(1));
            let (h, batch) = (host.clone(), o.event_batch.max(1));
            pump = Some(tokio::spawn(async move {
                while let Some(e) = rx.recv().await {
                    let mut b = vec![e];
                    while b.len() < batch {
                        match rx.try_recv() {
                            Ok(e) => b.push(e),
                            Err(_) => break,
                        }
                    }
                    h.emit_await(b).await;
                }
            }));
            tx = Some(t);
        }
        Emitter {
            mode: o.event_mode.clone(),
            run_id: o.run_id.clone(),
            host,
            tx: Mutex::new(tx),
            pump: Mutex::new(pump),
            seq: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            first: Mutex::new(0.0),
            last: Mutex::new(0.0),
        }
    }

    async fn emit(&self, kind: &str, idx: i64, data: String) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let ev = Event { kind: kind.into(), run_id: self.run_id.clone(), seq, idx, t_us: now_us(), data };
        if seq == 0 {
            *self.first.lock().unwrap() = ev.t_us;
        }
        if self.mode == "default" {
            self.host.emit(vec![ev]).await;
        } else {
            let tx = self.tx.lock().unwrap().clone();
            if let Some(tx) = tx {
                if self.mode == "drop" {
                    if tx.try_send(ev).is_err() {
                        self.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    let _ = tx.send(ev).await;
                }
            }
        }
        *self.last.lock().unwrap() = now_us();
    }

    async fn close(&self) {
        self.tx.lock().unwrap().take();
        let p = self.pump.lock().unwrap().take();
        if let Some(p) = p {
            let _ = p.await;
        }
    }
}

// ---------------------------------------------------------------- pages

static FILLER: LazyLock<Vec<u8>> = LazyLock::new(|| {
    let unit = b"<div class=\"item\" data-id=\"42\"><a href=\"/p?q=1&r=2\">Item eleve</a></div>\n";
    unit.iter().cycle().take(1 << 20).copied().collect()
});

fn make_page(run_id: &str, idx: u64, size: usize) -> Vec<u8> {
    let mut v = format!("<!-- run {run_id} page {idx} -->\n").into_bytes();
    while v.len() < size {
        let n = (size - v.len()).min(FILLER.len());
        v.extend_from_slice(&FILLER[..n]);
    }
    v.truncate(size.max(1));
    v
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 * p) as usize).min(v.len() - 1)]
}

#[derive(Default)]
struct Acc {
    done: u64,
    timeout: u64,
    error: u64,
    commit: Vec<f64>,
    deliver: Vec<f64>,
    body: Vec<f64>,
    rt: Vec<f64>,
    errors: Vec<String>,
    timeline: Vec<(u64, String, f64)>,
}

pub async fn run(opts: RunOpts, host: Arc<dyn Host>, cancel: CancellationToken) -> Summary {
    let t_start = now_us();
    let ev = Arc::new(Emitter::new(&opts, host.clone()));
    let mut s = Summary { run_id: opts.run_id.clone(), t_start_us: t_start, ..Default::default() };

    if opts.stream_events > 0 {
        let payload = "x".repeat(opts.event_bytes);
        for i in 0..opts.stream_events {
            if cancel.is_cancelled() {
                s.cancelled = true;
                break;
            }
            ev.emit("tick", i as i64, payload.clone()).await;
        }
        ev.close().await;
    } else {
        let w = writer(&opts.db_path);
        let _ = std::fs::create_dir_all(&opts.blob_dir);
        let next = Arc::new(AtomicU64::new(0));
        let acc = Arc::new(Mutex::new(Acc::default()));
        let opts = Arc::new(opts.clone());
        let mut tasks = Vec::new();
        for _ in 0..opts.concurrency.max(1) {
            let (o, h, c, w, n, a, e) = (opts.clone(), host.clone(), cancel.clone(), w.clone(), next.clone(), acc.clone(), ev.clone());
            tasks.push(tokio::spawn(async move { worker(o, h, c, w, n, a, e, t_start).await }));
        }
        for t in tasks {
            let _ = t.await;
        }
        ev.emit("run_done", -1, String::new()).await;
        ev.close().await;
        let mut a = std::mem::take(&mut *acc.lock().unwrap());
        s.pages_done = a.done;
        s.pages_timeout = a.timeout;
        s.pages_error = a.error;
        s.commit_p50_us = pct(&mut a.commit, 0.5);
        s.commit_p99_us = pct(&mut a.commit, 0.99);
        s.deliver_p50_us = pct(&mut a.deliver, 0.5);
        s.body_p50_us = pct(&mut a.body, 0.5);
        s.rt_p50_us = pct(&mut a.rt, 0.5);
        s.errors = a.errors;
        s.timeline = a.timeline;
        s.writer_commits = w.stats.commits.load(Ordering::Relaxed);
        s.writer_busy = w.stats.busy.load(Ordering::Relaxed);
        s.writer_lock_wait_ms = w.stats.lock_wait_us.load(Ordering::Relaxed) as f64 / 1000.0;
        s.cancelled = cancel.is_cancelled();
    }
    s.events_emitted = ev.seq.load(Ordering::Relaxed);
    s.events_dropped = ev.dropped.load(Ordering::Relaxed);
    s.t_first_emit_us = *ev.first.lock().unwrap();
    s.t_last_emit_us = *ev.last.lock().unwrap();
    s.t_end_us = now_us();
    s.engine_peak_rss = peak_rss_bytes();
    s
}

#[allow(clippy::too_many_arguments)]
async fn worker(
    o: Arc<RunOpts>,
    host: Arc<dyn Host>,
    cancel: CancellationToken,
    w: Arc<Writer>,
    next: Arc<AtomicU64>,
    acc: Arc<Mutex<Acc>>,
    ev: Arc<Emitter>,
    t_start: f64,
) {
    loop {
        if cancel.is_cancelled() {
            return;
        }
        let idx = next.fetch_add(1, Ordering::Relaxed);
        if idx >= o.n_pages {
            return;
        }
        let body = make_page(&o.run_id, idx, o.page_bytes);
        let sha = hex(&Sha256::digest(&body));
        let path = format!("{}/{}", o.blob_dir, sha);
        if !std::path::Path::new(&path).exists() {
            let tmp = format!("{path}.{}.tmp", std::process::id());
            std::fs::write(&tmp, &body).unwrap();
            std::fs::rename(&tmp, &path).unwrap();
        }
        if cancel.is_cancelled() {
            return;
        }
        if o.leases {
            w.put(&o.run_id, idx, "leased", &sha, &o.owner).await;
        }
        let size = body.len() as u64;
        let page = PageMsg {
            run_id: o.run_id.clone(),
            idx,
            url: format!("https://example.test/p/{idx}"),
            body: if o.inline { Some(body) } else { None },
            blob: BlobRef { path, size, sha256: sha.clone() },
            t0_us: now_us(),
        };
        let t0 = page.t0_us;
        let fut = host.handle(page);
        let res = tokio::select! {
            biased;
            _ = cancel.cancelled() => return, // abandon in-flight handler; lease stays until released
            r = async {
                if o.handler_timeout_ms > 0 {
                    tokio::time::timeout(Duration::from_millis(o.handler_timeout_ms), fut).await.map_err(|_| ())
                } else {
                    Ok(fut.await)
                }
            } => r,
        };
        if cancel.is_cancelled() && !matches!(res, Ok(Ok(_))) {
            return; // "client gone" during cancellation is not a handler failure: keep the lease
        }
        let t_ret = now_us();
        let status: &'static str = match &res {
            Ok(Ok(v)) => {
                let mut a = acc.lock().unwrap();
                if let (Some(te), Some(tb)) = (v.get("t_enter_us").and_then(|x| x.as_f64()), v.get("t_body_us").and_then(|x| x.as_f64())) {
                    a.deliver.push(te - t0);
                    a.body.push(tb - t0);
                }
                a.rt.push(t_ret - t0);
                "done"
            }
            Ok(Err(e)) => {
                let mut a = acc.lock().unwrap();
                if a.errors.len() < 3 {
                    a.errors.push(e.clone());
                }
                "error"
            }
            Err(()) => "timeout",
        };
        let lat = w.put(&o.run_id, idx, status, &sha, &o.owner).await;
        let done_n = {
            let mut a = acc.lock().unwrap();
            a.commit.push(lat);
            match status {
                "done" => a.done += 1,
                "error" => a.error += 1,
                _ => a.timeout += 1,
            }
            if o.timeline {
                a.timeline.push((idx, status.into(), (now_us() - t_start) / 1000.0));
            }
            a.done + a.error + a.timeout
        };
        if o.item_events {
            ev.emit("item_done", idx as i64, status.into()).await;
        }
        if o.progress_every > 0 && done_n % o.progress_every == 0 {
            ev.emit("progress", done_n as i64, String::new()).await;
        }
    }
}
