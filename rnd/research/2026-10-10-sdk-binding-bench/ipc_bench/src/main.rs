//! Engine-side JSON-RPC-over-stdio benchmark: a Tokio "engine" spawns a handler host
//! (Node or Python), sends one request per simulated page (HTML payload) as
//! newline-delimited JSON-RPC 2.0, and waits for the handler's response.
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{oneshot, Mutex};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

fn make_html(len: usize) -> String {
    let unit = "<div class=\"item\" data-id=\"42\"><a href=\"/p?q=1&r=2\">Item \u{e9}l\u{e8}ve</a></div>\n";
    let mut s = String::with_capacity(len + unit.len());
    while s.len() < len {
        s.push_str(unit);
    }
    while s.len() > len {
        s.pop();
    }
    s
}

fn pct(v: &mut [u128], p: usize) -> u128 {
    v.sort_unstable();
    v[(v.len() * p) / 100]
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let label = args[1].clone();
    let cmd: Vec<String> = args[2..].to_vec();

    let spawn_t0 = Instant::now();
    let mut child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn handler host");
    let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let stdout = child.stdout.take().unwrap();
    let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

    let p2 = pending.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::with_capacity(1 << 20, stdout);
        let mut line = Vec::with_capacity(1 << 16);
        loop {
            line.clear();
            let n = reader.read_until(b'\n', &mut line).await.unwrap();
            if n == 0 {
                break;
            }
            let v: Value = serde_json::from_slice(&line).unwrap();
            let id = v["id"].as_u64().unwrap();
            if let Some(tx) = p2.lock().await.remove(&id) {
                let _ = tx.send(v);
            }
        }
    });

    let next_id = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let call = {
        let stdin = stdin.clone();
        let pending = pending.clone();
        let next_id = next_id.clone();
        move |html: Arc<String>| {
            let stdin = stdin.clone();
            let pending = pending.clone();
            let id = next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            async move {
                let (tx, rx) = oneshot::channel();
                pending.lock().await.insert(id, tx);
                let mut buf = serde_json::to_vec(&json!({
                    "jsonrpc": "2.0", "id": id, "method": "handle",
                    "params": {"url": "https://example.test/p", "html": html.as_str()}
                }))
                .unwrap();
                buf.push(b'\n');
                stdin.lock().await.write_all(&buf).await.unwrap();
                rx.await.unwrap()
            }
        }
    };

    // Startup: time until the first (empty) round trip completes.
    let _ = call(Arc::new(String::new())).await;
    let startup_ms = spawn_t0.elapsed().as_secs_f64() * 1000.0;
    println!("{}", json!({"bench": format!("{label}_spawn_to_first_reply"), "ms": startup_ms}));

    for &size in &[64usize, 100_000, 1_000_000] {
        let html = Arc::new(make_html(size));
        let iters = if size >= 1_000_000 { 2000 } else if size >= 100_000 { 10000 } else { 50000 };
        for _ in 0..500 {
            call(html.clone()).await;
        }
        let mut samples = Vec::with_capacity(iters);
        for _ in 0..iters {
            let t0 = Instant::now();
            let r = call(html.clone()).await;
            std::hint::black_box(r);
            samples.push(t0.elapsed().as_nanos());
        }
        let mean = samples.iter().sum::<u128>() as f64 / iters as f64;
        let p50 = pct(&mut samples, 50);
        let p99 = pct(&mut samples, 99);
        println!(
            "{}",
            json!({"bench": format!("{label}_seq_{size}"), "n": iters, "mean_ns": mean, "p50_ns": p50, "p99_ns": p99})
        );
    }

    for &size in &[64usize, 100_000] {
        let html = Arc::new(make_html(size));
        let iters: usize = if size >= 100_000 { 20000 } else { 200000 };
        let conc = 16;
        let t0 = Instant::now();
        let mut tasks = Vec::new();
        for _ in 0..conc {
            let call = call.clone();
            let html = html.clone();
            tasks.push(tokio::spawn(async move {
                for _ in 0..iters / conc {
                    std::hint::black_box(call(html.clone()).await);
                }
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        let rps = iters as f64 / t0.elapsed().as_secs_f64();
        println!("{}", json!({"bench": format!("{label}_c16_{size}"), "calls_per_s": rps}));
    }
    drop(stdin);
    let _ = child.kill().await;
}
