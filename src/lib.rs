use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use crate::runtime::{EvalError, EvalOutcome};

pub mod ops;
pub mod runtime;
pub mod snapshot;

pyo3::create_exception!(
    _miniclient,
    JavaScriptError,
    pyo3::exceptions::PyException,
    "A JS exception, carrying the `name`, `message` and `stack` of the JS Error itself."
);

/// Turns what the isolate thread sent back into a Python value via `pythonize`, straight from
/// the parsed `serde_json::Value` rather than a `json.loads` text round-trip.
fn to_python(py: Python<'_>, outcome: EvalOutcome) -> PyResult<Py<PyAny>> {
    match outcome {
        Ok(None) => Ok(py.None()),
        Ok(Some(json)) => {
            let value: deno_core::serde_json::Value = deno_core::serde_json::from_str(&json)
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok(pythonize::pythonize(py, &value)?.unbind())
        }
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

/// Thin pyo3 wrapper around [`runtime::Page`], the host-state container that owns one
/// isolate at a time: every method delegates straight to the isolate it currently owns. JS
/// values cross as JSON in both directions: `undefined` and `null` both arrive as `None`, and
/// anything JSON cannot carry raises.
#[pyclass(module = "miniclient._miniclient", dict)]
struct Page(runtime::Page);

#[pymethods]
impl Page {
    #[new]
    #[pyo3(signature = (url="http://localhost/", resource_mounts_json="{}"))]
    fn new(url: &str, resource_mounts_json: &str) -> Self {
        Self(runtime::Page::new(url, resource_mounts_json))
    }

    /// Awaits the script's result -- a promise is resolved and the event loop pumped -- while
    /// leaving Python's own loop free to serve whatever that script is waiting on. Always takes
    /// this settle-wait path regardless of whether `source` returns a promise: `runtime.rs`
    /// still checks `v8::Local<Value>::is_promise()` internally, but the caller no longer picks.
    async fn eval(&self, source: String) -> PyResult<Py<PyAny>> {
        let outcome = self
            .0
            .runtime()
            .send_eval(source, true)
            .await
            .map_err(closed)?;
        Python::attach(|py| to_python(py, outcome))
    }

    /// Detaches from the interpreter while waiting, same as `eval`: `close()` blocks on the
    /// isolate thread joining, and that thread needs the GIL back to drop any `Py<PyAny>`
    /// callable still held in `OpState` (the fetch backend) or to finish an in-flight
    /// `op_fetch` await -- holding the GIL here would deadlock against that.
    fn close(&self, py: Python<'_>) {
        py.detach(|| self.0.runtime().close());
    }

    /// Installs the 3 fetch callables `op_fetch` and friends dispatch to (spec §4). `fetch`
    /// must be a coroutine function -- `op_fetch` bridges to it via `TaskLocals` captured from
    /// the caller's own running event loop, so this must be called from inside one. Safe to call
    /// any time before JS first reaches one of those ops (see `send_install_fetch_backend`).
    fn install_host_ops(
        &self,
        py: Python<'_>,
        fetch: Py<PyAny>,
        fetch_abort: Py<PyAny>,
        fetch_sync: Py<PyAny>,
    ) -> PyResult<()> {
        let fetch_locals =
            pyo3_async_runtimes::TaskLocals::with_running_loop(py)?.copy_context(py)?;
        let backend = ops::PythonFetchBackend {
            fetch,
            fetch_locals,
            fetch_abort,
            fetch_sync,
        };
        let rx = self
            .0
            .runtime()
            .send_install_fetch_backend(std::sync::Arc::new(backend));
        py.detach(|| rx.blocking_recv()).map_err(closed)
    }
}

impl Page {
    /// No-settle-wait fast path for later read-only ergonomic methods (`find`, `text`, `attr`,
    /// ...): skips the promise-resolve/event-loop-drain `eval` always pays for, since a plain
    /// read never has anything to wait on. Not exposed to pyo3 -- `eval` is the only public
    /// escape hatch.
    #[allow(dead_code)]
    fn fast_eval(&self, py: Python<'_>, source: String) -> PyResult<Py<PyAny>> {
        let rx = self.0.runtime().send_eval(source, false);
        let outcome = py.detach(|| rx.blocking_recv());
        to_python(py, outcome.map_err(closed)?)
    }
}

#[pymodule]
fn _miniclient(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("JavaScriptError", m.py().get_type::<JavaScriptError>())?;
    m.add_class::<Page>()
}
