use std::future::Future;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use crate::runtime::{EvalError, EvalOutcome, Wait};

pub mod browser;
pub mod ops;
pub mod runtime;
pub mod snapshot;

pyo3::create_exception!(
    _miniclient,
    JavaScriptError,
    pyo3::exceptions::PyException,
    "A JS exception, carrying the `name`, `message` and `stack` of the JS Error itself."
);

/// Turns an `EvalError` into the matching Python exception: a plain `RuntimeError`, or a
/// `JavaScriptError` carrying the JS exception's `name`/`message`/`stack`.
fn to_pyerr(py: Python<'_>, err: EvalError) -> PyErr {
    match err {
        EvalError::Other(message) => PyRuntimeError::new_err(message),
        EvalError::Js(error) => {
            let message = error.message.unwrap_or(error.exception_message);
            let err = JavaScriptError::new_err(message.clone());
            let value = err.value(py);
            // Best-effort: these attrs are informational, not worth failing eval() over.
            let _ = value.setattr("name", error.name.unwrap_or_else(|| "Error".into()));
            let _ = value.setattr("message", message);
            let _ = value.setattr("stack", error.stack);
            err
        }
    }
}

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
        Err(err) => Err(to_pyerr(py, err)),
    }
}

fn closed<E>(_: E) -> PyErr {
    PyRuntimeError::new_err("the runtime is closed")
}

/// Blocks the calling thread (off-GIL) until `fut` resolves, driven by the same tokio runtime
/// `pyo3-async-runtimes` already uses for every `async fn` pymethod -- backs the synchronous,
/// no-settle-wait ergonomic methods (`find`/`find_all`/`text`/`attr`/...), which never need the
/// Python event loop pumping concurrently the way a settle-wait does.
fn block_on<T: Send>(
    py: Python<'_>,
    fut: impl Future<Output = Result<T, EvalError>> + Send,
) -> PyResult<T> {
    py.detach(|| pyo3_async_runtimes::tokio::get_runtime().block_on(fut))
        .map_err(|e| to_pyerr(py, e))
}

/// Thin pyo3 wrapper around [`browser::Page`], delegating every ergonomic method straight to
/// it. JS values cross as JSON in both directions: `undefined` and `null` both arrive as
/// `None`, and anything JSON cannot carry raises. `subclass` lets the Python facade's
/// `AsyncPage` extend this directly instead of wrapping an instance of it.
#[pyclass(module = "miniclient._miniclient", subclass)]
struct Page(browser::Page);

#[pymethods]
impl Page {
    #[new]
    #[pyo3(signature = (url="http://localhost/", resource_mounts_json="{}"))]
    fn new(url: &str, resource_mounts_json: &str) -> Self {
        Self(browser::Page::new(url, resource_mounts_json))
    }

    /// Evaluate `source` as a top-level script, `this` bound to `globalThis` by the isolate's
    /// own top-level semantics -- declarations (`var`/`function`) attach to `globalThis` and
    /// persist across calls, same as separate `<script>` tags would. Awaits the script's result
    /// -- a promise is resolved -- while leaving Python's own loop free to serve whatever that
    /// script is waiting on. Does not wait for unrelated pending timers/requests: page
    /// interactions are the ones that settle.
    async fn eval(&self, source: String) -> PyResult<Py<PyAny>> {
        let outcome = self
            .0
            .runtime()
            .send_eval(source, Wait::Promise)
            .await
            .map_err(closed)?;
        Python::attach(|py| to_python(py, outcome))
    }

    /// Return the first element matching `selector`, or `None` -- no-settle-wait, since a plain
    /// query never has anything to wait on.
    #[pyo3(signature = (selector, text=None))]
    fn find(
        &self,
        py: Python<'_>,
        selector: &str,
        text: Option<&str>,
    ) -> PyResult<Option<Element>> {
        block_on(py, self.0.find(selector, text)).map(|found| found.map(Element))
    }

    /// Like [`find`](Self::find), but returns every match.
    #[pyo3(signature = (selector, text=None))]
    fn find_all(
        &self,
        py: Python<'_>,
        selector: &str,
        text: Option<&str>,
    ) -> PyResult<Vec<Element>> {
        block_on(py, self.0.find_all(selector, text))
            .map(|found| found.into_iter().map(Element).collect())
    }

    /// Navigate to `url` in a fresh isolate, fetching and loading the document.
    async fn goto(&self, url: String) -> PyResult<()> {
        let outcome = self.0.goto(&url).await;
        Python::attach(|py| outcome.map_err(|e| to_pyerr(py, e)))
    }

    /// Load `html` into the document and initialize htmx, same-document.
    async fn load(&self, html: String) -> PyResult<()> {
        let outcome = self.0.load(&html).await;
        Python::attach(|py| outcome.map_err(|e| to_pyerr(py, e)))
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

/// Thin pyo3 wrapper around [`browser::Element`], delegating every ergonomic method straight to
/// it. Used directly as the async-facing element type -- the Python facade's sync `Element`
/// wraps an instance of it instead of subclassing it.
#[pyclass(module = "miniclient._miniclient")]
struct Element(browser::Element);

#[pymethods]
impl Element {
    /// `this.textContent`.
    #[getter]
    fn text(&self, py: Python<'_>) -> PyResult<String> {
        block_on(py, self.0.text())
    }

    /// `this.innerHTML`.
    #[getter]
    fn inner_html(&self, py: Python<'_>) -> PyResult<String> {
        block_on(py, self.0.inner_html())
    }

    /// `this.outerHTML`.
    #[getter]
    fn html(&self, py: Python<'_>) -> PyResult<String> {
        block_on(py, self.0.html())
    }

    /// `this.tagName`.
    #[getter]
    fn tag_name(&self, py: Python<'_>) -> PyResult<String> {
        block_on(py, self.0.tag_name())
    }

    /// `this.getAttribute(name)`, or `None` if absent.
    fn attr(&self, py: Python<'_>, name: &str) -> PyResult<Option<String>> {
        block_on(py, self.0.attr(name))
    }

    /// The parent element, or `None` if it has none.
    #[getter]
    fn parent(&self, py: Python<'_>) -> PyResult<Option<Element>> {
        block_on(py, self.0.parent()).map(|found| found.map(Element))
    }

    /// Return the first descendant matching `selector`, or `None`.
    #[pyo3(signature = (selector, text=None))]
    fn find(
        &self,
        py: Python<'_>,
        selector: &str,
        text: Option<&str>,
    ) -> PyResult<Option<Element>> {
        block_on(py, self.0.find(selector, text)).map(|found| found.map(Element))
    }

    /// Like [`find`](Self::find), but returns every match.
    #[pyo3(signature = (selector, text=None))]
    fn find_all(
        &self,
        py: Python<'_>,
        selector: &str,
        text: Option<&str>,
    ) -> PyResult<Vec<Element>> {
        block_on(py, self.0.find_all(selector, text))
            .map(|found| found.into_iter().map(Element).collect())
    }

    /// Dispatch a click `MouseEvent` and wait for the page to settle.
    async fn click(&self) -> PyResult<()> {
        let outcome = self.0.click().await;
        Python::attach(|py| outcome.map_err(|e| to_pyerr(py, e)))
    }

    /// Set the element's value, dispatch `change`, and wait for the page to settle.
    async fn fill(&self, value: String) -> PyResult<()> {
        let outcome = self.0.fill(&value).await;
        Python::attach(|py| outcome.map_err(|e| to_pyerr(py, e)))
    }

    /// Set the element's value, dispatch `input`, and wait for the page to settle.
    #[pyo3(name = "type")]
    async fn r#type(&self, value: String) -> PyResult<()> {
        let outcome = self.0.r#type(&value).await;
        Python::attach(|py| outcome.map_err(|e| to_pyerr(py, e)))
    }

    /// Dispatch a DOM event and wait for the page to settle. `event_init` defaults to
    /// `{bubbles: true, cancelable: true}` when `None`.
    #[pyo3(signature = (event, event_init=None))]
    async fn trigger(&self, event: String, event_init: Option<Py<PyAny>>) -> PyResult<()> {
        let event_init = match event_init {
            Some(obj) => {
                let value = Python::attach(|py| {
                    pythonize::depythonize::<deno_core::serde_json::Value>(obj.bind(py))
                })?;
                Some(value)
            }
            None => None,
        };
        let outcome = self.0.trigger(&event, event_init.as_ref()).await;
        Python::attach(|py| outcome.map_err(|e| to_pyerr(py, e)))
    }

    /// Submit this `<form>` element and wait for it to settle.
    async fn request_submit(&self) -> PyResult<()> {
        let outcome = self.0.request_submit().await;
        Python::attach(|py| outcome.map_err(|e| to_pyerr(py, e)))
    }

    /// Generic escape-hatch eval, `this` bound to this element -- same semantics as
    /// [`Page::eval`], scoped to the element.
    async fn eval(&self, source: String) -> PyResult<Py<PyAny>> {
        let outcome = self.0.eval(&source).await;
        Python::attach(|py| to_python(py, outcome))
    }
}

#[pymodule]
fn _miniclient(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("JavaScriptError", m.py().get_type::<JavaScriptError>())?;
    m.add_class::<Page>()?;
    m.add_class::<Element>()
}
