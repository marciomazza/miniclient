use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

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

/// `inspect.iscoroutinefunction` already unwraps `functools.partial`, but it looks at the
/// object itself, not at `__call__` -- so a plain function check misses an instance whose
/// `__call__` is async. Checking both covers every shape `register_function` can be handed.
fn is_async_callable(py: Python<'_>, callable: &Py<PyAny>) -> PyResult<bool> {
    let inspect = py.import("inspect")?;
    let is_coroutine_function = |obj: &Bound<'_, PyAny>| -> PyResult<bool> {
        inspect
            .call_method1("iscoroutinefunction", (obj,))?
            .extract()
    };
    let bound = callable.bind(py);
    if is_coroutine_function(bound)? {
        return Ok(true);
    }
    match bound.getattr("__call__") {
        Ok(call) => is_coroutine_function(&call),
        Err(_) => Ok(false),
    }
}

/// `name` is spliced verbatim into `globalThis.<name> = ...` (spec §4's binding line), so it
/// must be a valid JS identifier -- otherwise a caller could inject arbitrary JS through it.
fn is_valid_js_identifier(name: &str) -> bool {
    static IDENTIFIER: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^[A-Za-z_$][A-Za-z0-9_$]*$").unwrap());
    IDENTIFIER.is_match(name)
}

/// A V8 isolate with its own thread. JS values cross as JSON in both directions (spec §4):
/// `undefined` and `null` both arrive as `None`, and anything JSON cannot carry raises.
#[pyclass(module = "miniclient._miniclient")]
struct Runtime(runtime::Runtime);

#[pymethods]
impl Runtime {
    /// The isolate holding the leaked snapshot dies with the process anyway (see snapshot::leak).
    #[new]
    fn new(snapshot: &[u8], url: &str) -> Self {
        Self(runtime::Runtime::new(Box::leak(Box::from(snapshot)), url))
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

    /// Binds a sync Python callable to a JS global, backed by one dispatch op regardless of how
    /// many names are bound (spec §4). An async callable is refused here, at bind time -- there
    /// is no `register_function` twin that parks a `PromiseResolver` for it, and nothing needs one.
    fn register_function(&self, py: Python<'_>, name: String, callable: Py<PyAny>) -> PyResult<()> {
        if !is_valid_js_identifier(&name) {
            return Err(PyValueError::new_err(format!(
                "register_function({name:?}): name must be a valid JS identifier"
            )));
        }
        if is_async_callable(py, &callable)? {
            return Err(PyTypeError::new_err(format!(
                "register_function({name:?}): callable must be sync, got an async function"
            )));
        }
        let rx = self.0.send_register_function(name, callable);
        let outcome = py.detach(|| rx.blocking_recv());
        to_python(py, outcome.map_err(closed)?)?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::sync::OnceLock;

    use pyo3::types::PyModule;

    use super::*;
    use crate::snapshot::{self, support};

    /// Built once and shared across tests below, same rationale as `runtime.rs`'s own copy of
    /// this helper: a snapshot is expensive and only ever read here, never mutated.
    fn test_snapshot() -> &'static [u8] {
        static SNAPSHOT: OnceLock<Box<[u8]>> = OnceLock::new();
        SNAPSHOT.get_or_init(|| {
            snapshot::create_snapshot(
                support::production_scripts(),
                Some(support::warmup_script()),
            )
            .expect("failed to build the test snapshot")
        })
    }

    fn fixtures(py: Python<'_>) -> Bound<'_, PyModule> {
        PyModule::from_code(
            py,
            &CString::new(
                r#"
def add(a, b):
    return a + b

def echo(*args):
    return list(args)

async def async_fn():
    return 1

def not_json():
    return float("nan")

class AsyncCallable:
    async def __call__(self):
        return 1

async_instance = AsyncCallable()
"#,
            )
            .unwrap(),
            &CString::new("register_function_test_fixtures.py").unwrap(),
            &CString::new("register_function_test_fixtures").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn register_function_binds_a_sync_callable_reachable_from_js() {
        let _guard = support::v8_test_lock();
        let runtime = Runtime(runtime::Runtime::new(test_snapshot(), "http://localhost/"));
        Python::attach(|py| {
            let add = fixtures(py).getattr("add").unwrap().unbind();
            runtime
                .register_function(py, "__test_add".into(), add)
                .unwrap();
        });
        let result = Python::attach(|py| runtime.eval(py, "__test_add(1, 2)".into())).unwrap();
        Python::attach(|py| assert_eq!(result.extract::<i64>(py).unwrap(), 3));
    }

    /// `json.dumps`'s default `allow_nan=True` would otherwise hand `op_call_python` a
    /// `NaN`/`Infinity` token `serde_json::from_str` can't parse -- a bare `.expect` on that
    /// would panic across the V8 FFI boundary instead of raising a clean Python-visible error.
    #[test]
    fn register_function_raises_cleanly_on_a_non_json_return_value() {
        let _guard = support::v8_test_lock();
        let runtime = Runtime(runtime::Runtime::new(test_snapshot(), "http://localhost/"));
        Python::attach(|py| {
            let not_json = fixtures(py).getattr("not_json").unwrap().unbind();
            runtime
                .register_function(py, "__test_not_json".into(), not_json)
                .unwrap();
        });
        let err = Python::attach(|py| runtime.eval(py, "__test_not_json()".into())).unwrap_err();
        assert!(err.to_string().contains("not JSON compliant"));
    }

    #[test]
    fn registering_many_names_still_costs_exactly_one_op() {
        let _guard = support::v8_test_lock();
        // fetch, fetch_abort, fetch_sync, fs_stat, fs_read, call_python (spec §4).
        assert_eq!(crate::runtime::extension().ops.len(), 6);
        let runtime = Runtime(runtime::Runtime::new(test_snapshot(), "http://localhost/"));
        Python::attach(|py| {
            let fixtures = fixtures(py);
            for name in [
                "__mini_fm_register",
                "__mini_fm_reset",
                "__mini_fm_register_seq",
                "__mini_fm_next",
            ] {
                let echo = fixtures.getattr("echo").unwrap().unbind();
                runtime.register_function(py, name.into(), echo).unwrap();
            }
        });
        let result =
            Python::attach(|py| runtime.eval(py, "__mini_fm_next(1, 2, 3)".into())).unwrap();
        Python::attach(|py| {
            assert_eq!(result.extract::<Vec<i64>>(py).unwrap(), vec![1, 2, 3]);
        });
        assert_eq!(crate::runtime::extension().ops.len(), 6);
    }

    #[test]
    fn register_function_refuses_a_name_that_is_not_a_js_identifier() {
        let _guard = support::v8_test_lock();
        let runtime = Runtime(runtime::Runtime::new(test_snapshot(), "http://localhost/"));
        let err = Python::attach(|py| {
            let add = fixtures(py).getattr("add").unwrap().unbind();
            runtime
                .register_function(py, "x; globalThis.__pwned = true".into(), add)
                .unwrap_err()
        });
        assert!(err.to_string().contains("identifier"));
        let pwned =
            Python::attach(|py| runtime.eval(py, "typeof globalThis.__pwned".into())).unwrap();
        Python::attach(|py| assert_eq!(pwned.extract::<String>(py).unwrap(), "undefined"));
    }

    #[test]
    fn register_function_refuses_an_async_callable() {
        let _guard = support::v8_test_lock();
        let runtime = Runtime(runtime::Runtime::new(test_snapshot(), "http://localhost/"));
        let err = Python::attach(|py| {
            let async_fn = fixtures(py).getattr("async_fn").unwrap().unbind();
            runtime
                .register_function(py, "__bad".into(), async_fn)
                .unwrap_err()
        });
        assert!(err.to_string().contains("async"));
        let unset = Python::attach(|py| runtime.eval(py, "typeof __bad".into())).unwrap();
        Python::attach(|py| assert_eq!(unset.extract::<String>(py).unwrap(), "undefined"));
    }

    /// `inspect.iscoroutinefunction` alone misses this shape: an instance is not itself a
    /// coroutine function even though calling it returns one.
    #[test]
    fn register_function_refuses_an_object_with_an_async_call() {
        let _guard = support::v8_test_lock();
        let runtime = Runtime(runtime::Runtime::new(test_snapshot(), "http://localhost/"));
        let err = Python::attach(|py| {
            let async_instance = fixtures(py).getattr("async_instance").unwrap().unbind();
            runtime
                .register_function(py, "__bad_instance".into(), async_instance)
                .unwrap_err()
        });
        assert!(err.to_string().contains("async"));
    }
}
