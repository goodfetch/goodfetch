//! Hello-world PyO3 extension module.
use pyo3::prelude::*;

#[pyfunction]
fn hello() -> String {
    format!("hello from pyo3 extension ({})", env!("BUILD_TARGET"))
}

#[pymodule(gil_used = false)]
fn automan_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    Ok(())
}
