use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::runtime::{EvalError, EvalOutcome};

pub mod runtime;
pub mod snapshot;

pyo3::create_exception!(
    _miniclient,
    JavaScriptError,
    pyo3::exceptions::PyException,
    "A JS exception, carrying the `name`, `message` and `stack` of the JS Error itself."
);

/// Turns what the isolate thread sent back into a Python value, parsing the JSON with the
/// stdlib rather than a second serde stack on the Rust side.
fn to_python(py: Python<'_>, outcome: EvalOutcome) -> PyResult<Py<PyAny>> {
    match outcome {
        Ok(None) => Ok(py.None()),
        Ok(Some(json)) => Ok(py.import("json")?.call_method1("loads", (json,))?.unbind()),
        Err(EvalError::Other(message)) => Err(PyRuntimeError::new_err(message)),
        Err(EvalError::Js(error)) => {
            let message = error.message.unwrap_or(error.exception_message);
            let err = JavaScriptError::new_err(message.clone());
            let value = err.value(py);
            value.setattr("name", error.name.unwrap_or_else(|| "Error".into()))?;
            value.setattr("message", message)?;
            value.setattr("stack", error.stack)?;
            Err(err)
        }
    }
}

fn closed<E>(_: E) -> PyErr {
    PyRuntimeError::new_err("the runtime is closed")
}

/// A V8 isolate with its own thread. JS values cross as JSON in both directions (spec §4):
/// `undefined` and `null` both arrive as `None`, and anything JSON cannot carry raises.
#[pyclass(module = "miniclient._miniclient")]
struct Runtime(runtime::Runtime);

#[pymethods]
impl Runtime {
    #[new]
    fn new() -> Self {
        Self(runtime::Runtime::new())
    }

    /// Detaches from the interpreter while waiting: the script may call back into Python.
    fn eval(&self, py: Python<'_>, source: String) -> PyResult<Py<PyAny>> {
        let rx = self.0.send_eval(source, false);
        let outcome = py.detach(|| rx.blocking_recv());
        to_python(py, outcome.map_err(closed)?)
    }

    /// Awaits the script's result -- a promise is resolved and the event loop pumped -- while
    /// leaving Python's own loop free to serve whatever that script is waiting on.
    async fn eval_async(&self, source: String) -> PyResult<Py<PyAny>> {
        let outcome = self.0.send_eval(source, true).await.map_err(closed)?;
        Python::attach(|py| to_python(py, outcome))
    }

    fn close(&self) {
        self.0.close();
    }
}

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
    m.add("JavaScriptError", m.py().get_type::<JavaScriptError>())?;
    m.add_class::<Runtime>()?;
    m.add_function(wrap_pyfunction!(v8_version, m)?)?;
    m.add_function(wrap_pyfunction!(create_snapshot, m)?)
}
