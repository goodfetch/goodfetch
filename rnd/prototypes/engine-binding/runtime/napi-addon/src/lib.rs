//! napi-rs binding: the engine runs inside the Node process on napi-rs's Tokio runtime.
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use engine_core::{self as ec, BoxFut, CancellationToken, Event, PageMsg, RunOpts};
use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;

static TOKENS: LazyLock<Mutex<HashMap<String, CancellationToken>>> = LazyLock::new(Default::default);

#[napi(object)]
pub struct BlobJs {
    pub path: String,
    pub size: f64,
    pub sha256: String,
}

#[napi(object)]
pub struct PageJs {
    #[napi(js_name = "run_id")]
    pub run_id: String,
    pub idx: f64,
    pub url: String,
    /// Zero-copy external Buffer over the engine's Vec when `inline`.
    pub body: Option<Buffer>,
    pub blob: BlobJs,
    #[napi(js_name = "t0_us")]
    pub t0_us: f64,
}

#[napi(object)]
pub struct EventJs {
    pub kind: String,
    #[napi(js_name = "run_id")]
    pub run_id: String,
    pub seq: f64,
    pub idx: f64,
    #[napi(js_name = "t_us")]
    pub t_us: f64,
    pub data: String,
}

type HandlerFn = ThreadsafeFunction<PageJs, Promise<serde_json::Value>, PageJs, Status, false>;
type EventFn = ThreadsafeFunction<Vec<EventJs>, (), Vec<EventJs>, Status, false>;

struct NapiHost {
    handler: Arc<HandlerFn>,
    on_event: Arc<EventFn>,
}

fn to_js(evs: Vec<Event>) -> Vec<EventJs> {
    evs.into_iter()
        .map(|e| EventJs { kind: e.kind, run_id: e.run_id, seq: e.seq as f64, idx: e.idx as f64, t_us: e.t_us, data: e.data })
        .collect()
}

impl ec::Host for NapiHost {
    fn handle(&self, p: PageMsg) -> BoxFut<std::result::Result<serde_json::Value, String>> {
        let h = self.handler.clone();
        Box::pin(async move {
            let js = PageJs {
                run_id: p.run_id,
                idx: p.idx as f64,
                url: p.url,
                body: p.body.map(Buffer::from),
                blob: BlobJs { path: p.blob.path, size: p.blob.size as f64, sha256: p.blob.sha256 },
                t0_us: p.t0_us,
            };
            // call_async would napi_fatal_exception (kill the process) on a sync throw; _catch maps it to Err.
            let promise = h.call_async_catch(js).await.map_err(|e| format!("{e}"))?;
            promise.await.map_err(|e| format!("{e}"))
        })
    }
    fn emit(&self, evs: Vec<Event>) -> BoxFut<()> {
        // Default: NonBlocking call into the TSFN's unbounded queue; the engine never waits.
        self.on_event.call(to_js(evs), ThreadsafeFunctionCallMode::NonBlocking);
        Box::pin(async {})
    }
    fn emit_await(&self, evs: Vec<Event>) -> BoxFut<()> {
        let f = self.on_event.clone();
        Box::pin(async move {
            let _ = f.call_async(to_js(evs)).await;
        })
    }
}

/// Runs the engine; resolves with the run summary as JSON.
#[napi]
pub async fn run(opts_json: String, handler: HandlerFn, on_event: EventFn) -> Result<String> {
    let opts: RunOpts = serde_json::from_str(&opts_json).map_err(|e| Error::from_reason(e.to_string()))?;
    let tok = CancellationToken::new();
    TOKENS.lock().unwrap().insert(opts.run_id.clone(), tok.clone());
    let id = opts.run_id.clone();
    let host = Arc::new(NapiHost { handler: Arc::new(handler), on_event: Arc::new(on_event) });
    let s = ec::run(opts, host, tok).await;
    TOKENS.lock().unwrap().remove(&id);
    Ok(serde_json::to_string(&s).unwrap())
}

#[napi]
pub fn cancel(run_id: String) -> bool {
    TOKENS.lock().unwrap().get(&run_id).map(|t| t.cancel()).is_some()
}
