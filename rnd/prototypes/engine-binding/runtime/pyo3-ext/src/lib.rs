//! PyO3 binding (abi3-py311): the engine runs on a Tokio runtime owned by the
//! extension, inside the Python process. Handlers are scheduled on the caller's
//! asyncio loop with run_coroutine_threadsafe; the run result comes back through
//! loop.call_soon_threadsafe.
use std::ffi::c_int;
use std::sync::{Arc, LazyLock, Mutex};

use engine_core::{self as ec, BoxFut, CancellationToken, Event, PageMsg, RunOpts};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyCFunction, PyDict, PyList, PyMemoryView, PyTuple};
use tokio::sync::oneshot;

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("engine").build().unwrap());

#[pyclass]
struct CancelToken {
    t: CancellationToken,
}

#[pymethods]
impl CancelToken {
    #[new]
    fn new() -> Self {
        CancelToken { t: CancellationToken::new() }
    }
    fn cancel(&self) {
        self.t.cancel();
    }
}

/// Read-only buffer over the engine's page bytes: memoryview(PageBody) is zero-copy.
#[pyclass(frozen)]
struct PageBody {
    data: Vec<u8>,
}

#[pymethods]
impl PageBody {
    unsafe fn __getbuffer__(slf: Bound<'_, Self>, view: *mut pyo3::ffi::Py_buffer, flags: c_int) -> PyResult<()> {
        let d = &slf.get().data;
        if pyo3::ffi::PyBuffer_FillInfo(view, slf.as_ptr(), d.as_ptr() as *mut _, d.len() as isize, 1, flags) == -1 {
            return Err(PyErr::fetch(slf.py()));
        }
        Ok(())
    }
    unsafe fn __releasebuffer__(&self, _view: *mut pyo3::ffi::Py_buffer) {}
}

struct PyHost {
    handler: Py<PyAny>,
    on_event: Option<Py<PyAny>>,
    event_loop: Py<PyAny>,
    delivery: String,
    body_kind: String,
}

fn ev_list<'py>(py: Python<'py>, evs: Vec<Event>) -> PyResult<Bound<'py, PyList>> {
    let l = PyList::empty(py);
    for e in evs {
        let d = PyDict::new(py);
        d.set_item("kind", e.kind)?;
        d.set_item("run_id", e.run_id)?;
        d.set_item("seq", e.seq)?;
        d.set_item("idx", e.idx)?;
        d.set_item("t_us", e.t_us)?;
        d.set_item("data", e.data)?;
        l.append(d)?;
    }
    Ok(l)
}

/// Cancels the asyncio task if the engine drops the call (timeout, run cancelled).
struct CancelGuard(Option<Py<PyAny>>);
impl Drop for CancelGuard {
    fn drop(&mut self) {
        if let Some(cf) = self.0.take() {
            Python::attach(|py| {
                let _ = cf.call_method0(py, "cancel");
            });
        }
    }
}

type Reply = Result<serde_json::Value, String>;

impl PyHost {
    fn deliver(&self, evs: Vec<Event>, blocking: bool) {
        let Some(cb) = &self.on_event else { return };
        Python::attach(|py| {
            let r = ev_list(py, evs).and_then(|l| {
                if blocking || self.delivery == "direct" {
                    cb.call1(py, (l,)).map(|_| ())
                } else {
                    self.event_loop.call_method1(py, "call_soon_threadsafe", (cb, l)).map(|_| ())
                }
            });
            if let Err(e) = r {
                e.print(py);
            }
        });
    }
}

impl ec::Host for PyHost {
    fn handle(&self, p: PageMsg) -> BoxFut<Reply> {
        let (tx, rx) = oneshot::channel::<Reply>();
        let started: PyResult<Py<PyAny>> = Python::attach(|py| {
            let d = PyDict::new(py);
            d.set_item("run_id", &p.run_id)?;
            d.set_item("idx", p.idx)?;
            d.set_item("url", &p.url)?;
            d.set_item("t0_us", p.t0_us)?;
            let b = PyDict::new(py);
            b.set_item("path", &p.blob.path)?;
            b.set_item("size", p.blob.size)?;
            b.set_item("sha256", &p.blob.sha256)?;
            d.set_item("blob", b)?;
            if let Some(body) = p.body {
                if self.body_kind == "memoryview" {
                    let obj = Bound::new(py, PageBody { data: body })?;
                    d.set_item("body", PyMemoryView::from(obj.as_any())?)?;
                } else {
                    d.set_item("body", PyBytes::new(py, &body))?;
                }
            }
            let coro = self.handler.call1(py, (d,))?;
            let asyncio = py.import("asyncio")?;
            let cf = asyncio.call_method1("run_coroutine_threadsafe", (coro, self.event_loop.bind(py)))?;
            let tx = Mutex::new(Some(tx));
            let cb = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>| -> PyResult<()> {
                let py = args.py();
                let f = args.get_item(0)?;
                let res: Reply = if f.call_method0("cancelled")?.is_truthy()? {
                    Err("cancelled".into())
                } else {
                    let exc = f.call_method0("exception")?;
                    if !exc.is_none() {
                        let lines = py.import("traceback")?.call_method1("format_exception", (exc,))?;
                        Err(lines.extract::<Vec<String>>()?.concat())
                    } else {
                        let s: String = py.import("json")?.call_method1("dumps", (f.call_method0("result")?,))?.extract()?;
                        Ok(serde_json::from_str(&s).unwrap_or(serde_json::Value::Null))
                    }
                };
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(res);
                }
                Ok(())
            })?;
            cf.call_method1("add_done_callback", (cb,))?;
            Ok(cf.unbind())
        });
        Box::pin(async move {
            let cf = started.map_err(|e| e.to_string())?;
            let mut g = CancelGuard(Some(cf));
            let r = rx.await.unwrap_or_else(|_| Err("dropped".into()));
            g.0 = None;
            r
        })
    }
    fn emit(&self, evs: Vec<Event>) -> BoxFut<()> {
        // "direct" (default): call the Python callback from the engine thread under the GIL,
        // the engine waits. "loop": hand it to the asyncio loop (unbounded ready queue).
        self.deliver(evs, false);
        Box::pin(async {})
    }
    fn emit_await(&self, evs: Vec<Event>) -> BoxFut<()> {
        self.deliver(evs, true);
        Box::pin(async {})
    }
}

/// Starts a run on the extension's Tokio runtime; `done(summary_json)` is invoked on `event_loop`.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (opts_json, handler, on_event, event_loop, done, token, delivery="direct", body_kind="bytes"))]
fn start_run(
    py: Python<'_>,
    opts_json: &str,
    handler: Py<PyAny>,
    on_event: Option<Py<PyAny>>,
    event_loop: Py<PyAny>,
    done: Py<PyAny>,
    token: PyRef<'_, CancelToken>,
    delivery: &str,
    body_kind: &str,
) -> PyResult<()> {
    let opts: RunOpts = serde_json::from_str(opts_json).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let lp = event_loop.clone_ref(py);
    let host = Arc::new(PyHost { handler, on_event, event_loop, delivery: delivery.into(), body_kind: body_kind.into() });
    let tok = token.t.clone();
    RT.spawn(async move {
        let s = ec::run(opts, host, tok).await;
        let js = serde_json::to_string(&s).unwrap();
        Python::attach(|py| {
            if let Err(e) = lp.call_method1(py, "call_soon_threadsafe", (done, js)) {
                e.print(py);
            }
        });
    });
    Ok(())
}

#[pymodule]
fn automan_engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<CancelToken>()?;
    m.add_function(wrap_pyfunction!(start_run, m)?)?;
    Ok(())
}
