use pyo3::prelude::*;

/// The runtime still lives in jsrun until the swap. This exists so the crate makes a real
/// call into V8: without one the linker drops the static lib and the wheel ships no V8 at all.
#[pyfunction]
fn v8_version() -> &'static str {
    deno_core::v8::V8::get_version()
}

#[pymodule]
fn _miniclient(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(v8_version, m)?)
}
