use std::time::Instant;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunction;
use napi_derive::napi;

#[napi(object)]
pub struct Stats {
    pub iterations: u32,
    pub mean_ns: f64,
    pub p50_ns: f64,
    pub p99_ns: f64,
    pub max_ns: f64,
}

fn stats(mut v: Vec<u128>) -> Stats {
    v.sort_unstable();
    let n = v.len();
    let sum: u128 = v.iter().sum();
    Stats {
        iterations: n as u32,
        mean_ns: sum as f64 / n as f64,
        p50_ns: v[n / 2] as f64,
        p99_ns: v[(n * 99) / 100] as f64,
        max_ns: v[n - 1] as f64,
    }
}

/// JS -> Rust sync call, no payload.
#[napi]
pub fn noop(x: u32) -> u32 {
    x.wrapping_add(1)
}

/// JS -> Rust sync call, payload copied from a JS string into a Rust String.
#[napi]
pub fn take_string(s: String) -> u32 {
    s.len() as u32
}

/// JS -> Rust sync call, payload passed as a Buffer (no UTF-8 conversion).
#[napi]
pub fn take_buffer(b: Buffer) -> u32 {
    b.len() as u32
}

/// JS -> Rust async call resolved on the napi-rs Tokio runtime.
#[napi]
pub async fn noop_async(x: u32) -> u32 {
    x.wrapping_add(1)
}

/// Rust (Tokio task) -> JS handler -> Promise<string> -> back to Rust, sequentially.
/// Simulates the engine invoking a user page handler with an HTML payload.
#[napi]
pub async fn bench_handler(
    handler: ThreadsafeFunction<String, Promise<String>, String, Status, false>,
    payload_len: u32,
    iterations: u32,
) -> Result<Stats> {
    let payload: String = make_html(payload_len as usize);
    let mut samples = Vec::with_capacity(iterations as usize);
    for _ in 0..iterations {
        let start = Instant::now();
        let promise = handler.call_async(payload.clone()).await?;
        let out = promise.await?;
        std::hint::black_box(out);
        samples.push(start.elapsed().as_nanos());
    }
    Ok(stats(samples))
}

/// Same as bench_handler but with `concurrency` calls in flight (throughput).
#[napi]
pub async fn bench_handler_concurrent(
    handler: ThreadsafeFunction<String, Promise<String>, String, Status, false>,
    payload_len: u32,
    iterations: u32,
    concurrency: u32,
) -> Result<f64> {
    let payload: String = make_html(payload_len as usize);
    let handler = std::sync::Arc::new(handler);
    let start = Instant::now();
    let per = iterations / concurrency;
    let mut tasks = Vec::new();
    for _ in 0..concurrency {
        let h = handler.clone();
        let p = payload.clone();
        tasks.push(tokio::spawn(async move {
            for _ in 0..per {
                let promise = h.call_async(p.clone()).await?;
                let out = promise.await?;
                std::hint::black_box(out);
            }
            Ok::<(), napi::Error>(())
        }));
    }
    for t in tasks {
        t.await.map_err(|e| Error::from_reason(e.to_string()))??;
    }
    let secs = start.elapsed().as_secs_f64();
    Ok((per * concurrency) as f64 / secs)
}

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
