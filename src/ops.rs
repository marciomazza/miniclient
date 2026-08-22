use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use deno_core::OpState;
use deno_core::ToJsBuffer;
use deno_core::op2;
use deno_error::JsErrorBox;
use pyo3::Bound;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use pyo3_async_runtimes::TaskLocals;
use serde::{Deserialize, Serialize};

/// Matches the dict shape `_fetch_op`/`_fetch_sync_op` already accept in `runtime.py` --
/// `headers` is a plain object there, unlike the response side (see `FetchResponse`), because
/// nothing on the Python side needs duplicate request header names.
#[derive(Deserialize)]
pub struct FetchRequest {
    pub id: String,
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<Vec<u8>>,
}

/// Matches the dict shape `_fetch_op`/`_fetch_sync_op` already return. `headers` is a list of
/// pairs, not an object, because a response can carry the same header name twice (e.g.
/// `Set-Cookie`).
#[derive(Serialize, Deserialize)]
pub struct FetchResponse {
    pub status: u16,
    #[serde(rename = "statusText")]
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub url: String,
}

/// Matches `_fs_stat_op`'s return shape.
#[derive(Serialize, Deserialize)]
pub struct FsStatResult {
    #[serde(rename = "isDirectory")]
    pub is_directory: bool,
}

/// The 5 Python callables these ops bridge to, plus the `TaskLocals` (event loop + context)
/// `op_fetch` needs to await a Python coroutine from the isolate thread. Installed into
/// `OpState` once, before any of these ops can be called from JS.
pub struct HostOps {
    pub fetch: Py<PyAny>,
    pub fetch_locals: TaskLocals,
    pub fetch_abort: Py<PyAny>,
    pub fetch_sync: Py<PyAny>,
    pub fs_stat: Py<PyAny>,
    pub fs_read: Py<PyAny>,
}

fn py_err_to_js(err: PyErr) -> JsErrorBox {
    JsErrorBox::generic(err.to_string())
}

fn fetch_request_to_pydict<'py>(
    py: Python<'py>,
    req: &FetchRequest,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("id", &req.id)?;
    dict.set_item("url", &req.url)?;
    dict.set_item("method", &req.method)?;
    let headers = PyDict::new(py);
    for (k, v) in &req.headers {
        headers.set_item(k, v)?;
    }
    dict.set_item("headers", headers)?;
    match &req.body {
        Some(body) => dict.set_item("body", PyBytes::new(py, body))?,
        None => dict.set_item("body", py.None())?,
    }
    Ok(dict)
}

fn pyobj_to_fetch_response(obj: &Bound<'_, PyAny>) -> PyResult<FetchResponse> {
    Ok(FetchResponse {
        status: obj.get_item("status")?.extract()?,
        status_text: obj
            .get_item("statusText")
            .and_then(|v| v.extract())
            .unwrap_or_default(),
        headers: obj
            .get_item("headers")
            .and_then(|v| v.extract())
            .unwrap_or_default(),
        body: obj.get_item("body")?.extract()?,
        url: obj.get_item("url")?.extract()?,
    })
}

/// Bridges to a Python coroutine via `pyo3_async_runtimes::into_future_with_locals` -- the
/// mechanism spec §1 calls for, not a hand-rolled channel.
#[op2]
#[serde]
pub async fn op_fetch(
    state: Rc<RefCell<OpState>>,
    #[serde] req: FetchRequest,
) -> Result<FetchResponse, JsErrorBox> {
    let (fetch, locals) = Python::attach(|py| {
        let state = state.borrow();
        let ops = state.borrow::<HostOps>();
        (ops.fetch.clone_ref(py), ops.fetch_locals.clone())
    });
    let future = Python::attach(|py| -> PyResult<_> {
        let dict = fetch_request_to_pydict(py, &req)?;
        let coro = fetch.bind(py).call1((dict,))?;
        pyo3_async_runtimes::into_future_with_locals(&locals, coro)
    })
    .map_err(py_err_to_js)?;
    let result = future.await.map_err(py_err_to_js)?;
    Python::attach(|py| pyobj_to_fetch_response(result.bind(py))).map_err(py_err_to_js)
}

#[op2(fast)]
pub fn op_fetch_abort(state: &mut OpState, #[string] request_id: String) -> Result<(), JsErrorBox> {
    Python::attach(|py| {
        let fetch_abort = state.borrow::<HostOps>().fetch_abort.clone_ref(py);
        fetch_abort.call1(py, (request_id,))
    })
    .map_err(py_err_to_js)?;
    Ok(())
}

/// Plain, not fast: this blocks on `future.result()` on the Python side (spec §4).
#[op2]
#[serde]
pub fn op_fetch_sync(
    state: &mut OpState,
    #[serde] req: FetchRequest,
) -> Result<FetchResponse, JsErrorBox> {
    Python::attach(|py| {
        let fetch_sync = state.borrow::<HostOps>().fetch_sync.clone_ref(py);
        let dict = fetch_request_to_pydict(py, &req)?;
        let result = fetch_sync.bind(py).call1((dict,))?;
        pyobj_to_fetch_response(&result)
    })
    .map_err(py_err_to_js)
}

// Not `fast`: V8's fast-call ABI only accepts primitive returns, not this and `op_fs_read`'s
// `serde_v8` struct/buffer -- still runs natively on the isolate thread either way.
#[op2]
#[serde]
pub fn op_fs_stat(state: &mut OpState, #[string] path: String) -> Result<FsStatResult, JsErrorBox> {
    Python::attach(|py| {
        let result = state
            .borrow::<HostOps>()
            .fs_stat
            .clone_ref(py)
            .call1(py, (path,))?;
        let is_directory = result.bind(py).get_item("isDirectory")?.extract()?;
        Ok(FsStatResult { is_directory })
    })
    .map_err(py_err_to_js)
}

#[op2]
#[serde]
pub fn op_fs_read(state: &mut OpState, #[string] path: String) -> Result<ToJsBuffer, JsErrorBox> {
    let fs_read = Python::attach(|py| state.borrow::<HostOps>().fs_read.clone_ref(py));
    let bytes: Vec<u8> =
        Python::attach(|py| fs_read.call1(py, (path,))?.extract(py)).map_err(py_err_to_js)?;
    Ok(bytes.into())
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use deno_core::{JsRuntime, RuntimeOptions};
    use pyo3::types::PyModule;

    use super::*;
    use crate::runtime::extension;
    use crate::snapshot::support::v8_test_lock;

    /// Runs `script`, reads one expression back out as a string -- same trick as
    /// `snapshot.rs`'s `eval_in_snapshot`, but against a live (non-snapshot) runtime.
    fn eval_string(js: &mut JsRuntime, expr: &str) -> String {
        let value = js.execute_script("<test>", expr.to_string()).unwrap();
        deno_core::scope!(scope, js);
        deno_core::v8::Local::new(scope, value).to_rust_string_lossy(scope)
    }

    fn bare_runtime() -> JsRuntime {
        crate::runtime::init_platform();
        JsRuntime::new(RuntimeOptions {
            extensions: vec![extension()],
            ..Default::default()
        })
    }

    /// A Python module with one function per test fixture, so each test can grab just the
    /// callable it needs without repeating `PyModule::from_code` boilerplate.
    fn fixtures(py: Python<'_>) -> Bound<'_, PyModule> {
        PyModule::from_code(
            py,
            &CString::new(
                r#"
calls = []

def fetch_abort_impl(request_id):
    calls.append(request_id)

def fetch_sync_impl(req):
    return {
        "status": 200,
        "statusText": "OK",
        "headers": [["content-type", "text/plain"]],
        "body": b"sync-body",
        "url": req["url"],
    }

def fs_stat_impl(path):
    return {"isDirectory": path.endswith("/")}

def fs_read_impl(path):
    return b"file-contents"

async def fetch_impl(req):
    import asyncio
    await asyncio.sleep(0)
    return {
        "status": 201,
        "statusText": "Created",
        "headers": [["content-type", "application/json"]],
        "body": b"async-body",
        "url": req["url"],
    }
"#,
            )
            .unwrap(),
            &CString::new("ops_test_fixtures.py").unwrap(),
            &CString::new("ops_test_fixtures").unwrap(),
        )
        .unwrap()
    }

    fn host_ops(_py: Python<'_>, fixtures: &Bound<'_, PyModule>, locals: TaskLocals) -> HostOps {
        let get = |name: &str| fixtures.getattr(name).unwrap().into();
        HostOps {
            fetch: get("fetch_impl"),
            fetch_locals: locals,
            fetch_abort: get("fetch_abort_impl"),
            fetch_sync: get("fetch_sync_impl"),
            fs_stat: get("fs_stat_impl"),
            fs_read: get("fs_read_impl"),
        }
    }

    /// A `TaskLocals` pointing at a real asyncio loop, run to completion on its own thread --
    /// `op_fetch` needs somewhere to actually schedule the Python coroutine it awaits.
    fn running_event_loop(py: Python<'_>) -> PyResult<TaskLocals> {
        let asyncio = py.import("asyncio")?;
        let event_loop: Py<PyAny> = asyncio.call_method0("new_event_loop")?.into();
        let for_thread = event_loop.clone_ref(py);
        std::thread::spawn(move || {
            Python::attach(|py| {
                let asyncio = py.import("asyncio").unwrap();
                asyncio
                    .call_method1("set_event_loop", (for_thread.bind(py),))
                    .unwrap();
                for_thread.bind(py).call_method0("run_forever").unwrap();
            });
        });
        Ok(TaskLocals::new(event_loop.into_bound(py)))
    }

    #[test]
    fn fs_stat_and_fs_read_round_trip_to_python() {
        let _guard = v8_test_lock();
        let mut js = bare_runtime();
        Python::attach(|py| {
            let fixtures = fixtures(py);
            let locals = running_event_loop(py).unwrap();
            js.op_state()
                .borrow_mut()
                .put(host_ops(py, &fixtures, locals));
        });
        assert_eq!(
            eval_string(
                &mut js,
                "String(Deno.core.ops.op_fs_stat('/tmp/').isDirectory)"
            ),
            "true"
        );
        assert_eq!(
            eval_string(
                &mut js,
                "Array.from(new Uint8Array(Deno.core.ops.op_fs_read('/tmp/x'))).map(b => String.fromCharCode(b)).join('')"
            ),
            "file-contents"
        );
    }

    #[test]
    fn fetch_sync_and_fetch_abort_round_trip_to_python() {
        let _guard = v8_test_lock();
        let mut js = bare_runtime();
        let fixtures_module: Py<PyModule> = Python::attach(|py| {
            let fixtures = fixtures(py);
            let locals = running_event_loop(py).unwrap();
            js.op_state()
                .borrow_mut()
                .put(host_ops(py, &fixtures, locals));
            fixtures.unbind()
        });
        let req_js = r#"{id: 'r1', url: 'http://x/', method: 'GET', headers: {}, body: null}"#;
        assert_eq!(
            eval_string(
                &mut js,
                &format!(
                    "(() => {{ const r = Deno.core.ops.op_fetch_sync({req_js}); return `${{r.status}} ${{Array.from(new Uint8Array(r.body)).map(b => String.fromCharCode(b)).join('')}}`; }})()"
                ),
            ),
            "200 sync-body"
        );
        // op_fetch_abort has no return value worth reading back -- the Python-side `calls`
        // list (recorded by `fetch_abort_impl`) is the only observable effect.
        js.execute_script("<test>", "Deno.core.ops.op_fetch_abort('some-id');")
            .unwrap();
        Python::attach(|py| {
            let calls: Vec<String> = fixtures_module
                .getattr(py, "calls")
                .unwrap()
                .extract(py)
                .unwrap();
            assert_eq!(calls, vec!["some-id".to_string()]);
        });
    }

    #[test]
    fn fetch_bridges_to_a_python_coroutine() {
        let _guard = v8_test_lock();
        let mut js = bare_runtime();
        Python::attach(|py| {
            let fixtures = fixtures(py);
            let locals = running_event_loop(py).unwrap();
            js.op_state()
                .borrow_mut()
                .put(host_ops(py, &fixtures, locals));
        });
        // Invoking an async op (even just to enqueue its future) needs an ambient Tokio
        // context, same as the real isolate thread's `tokio.block_on` in `runtime.rs` --
        // so the call that triggers `op_fetch` and the event-loop drive both go inside one.
        let tokio = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tokio.block_on(async {
            js.execute_script(
                "<test>",
                r#"
                globalThis.__result = null;
                Deno.core.ops.op_fetch({id: 'r1', url: 'http://x/', method: 'GET', headers: {}, body: null})
                    .then((r) => { globalThis.__result = r; });
                "#,
            )
            .unwrap();
            js.run_event_loop(deno_core::PollEventLoopOptions::default())
                .await
                .unwrap();
        });
        assert_eq!(
            eval_string(
                &mut js,
                "`${globalThis.__result.status} ${Array.from(new Uint8Array(globalThis.__result.body)).map(b => String.fromCharCode(b)).join('')}`"
            ),
            "201 async-body"
        );
    }
}
