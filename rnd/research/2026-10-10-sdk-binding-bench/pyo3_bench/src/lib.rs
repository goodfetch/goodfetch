use std::time::Instant;

use pyo3::prelude::*;
use pyo3::types::PyDict;

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

fn stats(py: Python<'_>, label: &str, mut v: Vec<u128>) -> PyResult<Py<PyAny>> {
    v.sort_unstable();
    let n = v.len();
    let d = PyDict::new(py);
    d.set_item("bench", label)?;
    d.set_item("n", n)?;
    d.set_item("mean_ns", v.iter().sum::<u128>() as f64 / n as f64)?;
    d.set_item("p50_ns", v[n / 2] as u64)?;
    d.set_item("p99_ns", v[(n * 99) / 100] as u64)?;
    Ok(d.into_any().unbind())
}

#[pyfunction]
fn noop(x: u64) -> u64 {
    x.wrapping_add(1)
}

#[pyfunction]
fn take_string(s: &str) -> usize {
    s.len()
}

#[pyfunction]
fn take_bytes(b: &[u8]) -> usize {
    b.len()
}

/// Python awaits a Rust future resolved on the Tokio runtime.
#[pyfunction]
fn noop_async(py: Python<'_>, x: u64) -> PyResult<Bound<'_, PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(x.wrapping_add(1)) })
}

/// Rust (Tokio task) calls a Python `async def handler(html) -> str` per simulated page,
/// awaiting each coroutine on the caller's asyncio loop, sequentially.
#[pyfunction]
fn bench_handler(py: Python<'_>, handler: Py<PyAny>, payload_len: usize, iterations: usize) -> PyResult<Bound<'_, PyAny>> {
    let locals = pyo3_async_runtimes::tokio::get_current_locals(py)?;
    pyo3_async_runtimes::tokio::future_into_py_with_locals(py, locals.clone(), async move {
        let html = make_html(payload_len);
        let mut samples = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let t0 = Instant::now();
            let fut = Python::attach(|py| {
                let coro = handler.call1(py, (html.as_str(),))?;
                pyo3_async_runtimes::into_future_with_locals(&locals, coro.into_bound(py))
            })?;
            let out = fut.await?;
            std::hint::black_box(out);
            samples.push(t0.elapsed().as_nanos());
        }
        Python::attach(|py| stats(py, &format!("rust_calls_py_handler_seq_{payload_len}"), samples))
    })
}

#[pymodule]
fn pyo3_bench(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(noop, m)?)?;
    m.add_function(wrap_pyfunction!(take_string, m)?)?;
    m.add_function(wrap_pyfunction!(take_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(noop_async, m)?)?;
    m.add_function(wrap_pyfunction!(bench_handler, m)?)?;
    Ok(())
}
