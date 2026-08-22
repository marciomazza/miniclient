use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

pub mod runtime;
pub mod snapshot;

/// The runtime still lives in jsrun until the swap. This exists so the crate makes a real
/// call into V8: without one the linker drops the static lib and the wheel ships no V8 at all.
#[pyfunction]
fn v8_version() -> &'static str {
    deno_core::v8::V8::get_version()
}

/// Serializes `scripts` (ordered `(name, source)` pairs) into a V8 snapshot blob, with
/// `warmup` run as deno_core's second pass.
#[pyfunction]
#[pyo3(signature = (scripts, warmup=None))]
fn create_snapshot<'py>(
    py: Python<'py>,
    scripts: Vec<(String, String)>,
    warmup: Option<String>,
) -> PyResult<Bound<'py, PyBytes>> {
    let blob = py
        .detach(|| snapshot::create_snapshot(scripts, warmup))
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PyBytes::new(py, &blob))
}

#[pymodule]
fn _miniclient(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(v8_version, m)?)?;
    m.add_function(wrap_pyfunction!(create_snapshot, m)?)
}
