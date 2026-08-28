use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use deno_core::JsBuffer;
use deno_core::OpState;
use deno_core::ToJsBuffer;
use deno_core::op2;
use deno_core::serde_json;
use deno_error::JsErrorBox;
use pyo3::Bound;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use pyo3_async_runtimes::TaskLocals;
use serde::{Deserialize, Serialize};

/// Matches the dict shape `_fetch_op`/`_fetch_sync_op` already accept in `runtime.py` --
/// `headers` is a plain object there, unlike the response side (see `FetchResponse`), because
/// nothing on the Python side needs duplicate request header names.
///
/// `body` is `JsBuffer`, not `Vec<u8>`: `bootstrap.js` always sends it as a `Uint8Array`
/// (never a plain JS array of numbers), and `serde_v8` only accepts that shape through its
/// "magic" buffer types -- a plain `Vec<u8>` field expects an `Array` and throws on a
/// `Uint8Array`.
#[derive(Deserialize)]
pub struct FetchRequest {
    // Only `op_fetch` (async) uses this, to track an in-flight request for abort. The sync
    // path (`op_fetch_sync`, reached from XHR via `node-child-process.js`'s `execFileSync`)
    // has no abort mechanism and never sends one -- defaulted, not required, so it doesn't
    // fail deserialization there.
    #[serde(default)]
    pub id: String,
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<JsBuffer>,
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

/// Shape `node-fs.js`'s `statSync` expects back from `op_fs_stat`.
#[derive(Serialize, Deserialize)]
pub struct FsStatResult {
    #[serde(rename = "isDirectory")]
    pub is_directory: bool,
}

/// The 3 Python callables these ops bridge to, plus the `TaskLocals` (event loop + context)
/// `op_fetch` needs to await a Python coroutine from the isolate thread. Installed into
/// `OpState` once, before any of these ops can be called from JS. (The fs ops hit the real
/// filesystem natively and need nothing here.)
pub struct HostOps {
    pub fetch: Py<PyAny>,
    pub fetch_locals: TaskLocals,
    pub fetch_abort: Py<PyAny>,
    pub fetch_sync: Py<PyAny>,
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
pub fn op_fs_stat(#[string] path: String) -> Result<FsStatResult, JsErrorBox> {
    // Missing path -> not a directory, no error (`is_dir` swallows the stat error).
    let is_directory = std::path::Path::new(&path).is_dir();
    Ok(FsStatResult { is_directory })
}

#[op2]
#[serde]
pub fn op_fs_read(#[string] path: String) -> Result<ToJsBuffer, JsErrorBox> {
    std::fs::read(&path)
        .map(Into::into)
        .map_err(|e| JsErrorBox::generic(format!("{path}: {e}")))
}

/// Callables bound by `Runtime::register_function`, indexed by the id baked into each
/// binding's generated JS (spec §4) -- `op_call_python`'s only job is to look one up and call
/// it, so binding any number of names still costs exactly this one op.
#[derive(Default)]
pub struct PythonFunctions(Vec<Py<PyAny>>);

impl PythonFunctions {
    /// Registers `callable` and returns the id `op_call_python` will look it up by.
    pub fn push(&mut self, callable: Py<PyAny>) -> usize {
        self.0.push(callable);
        self.0.len() - 1
    }

    fn get(&self, call_id: u32, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.0
            .get(call_id as usize)
            .ok_or_else(|| {
                PyRuntimeError::new_err(format!(
                    "op_call_python: no callable registered at id {call_id}"
                ))
            })
            .map(|callable| callable.clone_ref(py))
    }
}

/// Dispatches to a callable registered via `register_function`. Args and the return value cross
/// through Python's own `json` module rather than a second hand-rolled JSON<->PyAny conversion --
/// the same technique `to_python`/`MARSHAL_JS` already use for `eval`'s JSON-only contract.
#[op2]
#[serde]
pub fn op_call_python(
    state: &mut OpState,
    call_id: u32,
    #[serde] args: Vec<serde_json::Value>,
) -> Result<serde_json::Value, JsErrorBox> {
    Python::attach(|py| -> PyResult<serde_json::Value> {
        let callable = state.borrow::<PythonFunctions>().get(call_id, py)?;
        let json = py.import("json")?;
        let args_json = serde_json::to_string(&args).expect("args is always JSON-safe");
        let py_args: Bound<'_, PyList> = json.call_method1("loads", (args_json,))?.extract()?;
        let result = callable.bind(py).call1(py_args.to_tuple())?;
        // `allow_nan=False`: dumps's default lets NaN/Infinity through as bare (non-JSON)
        // tokens, which would make the `expect` below panic instead of raising cleanly.
        let dumps_kwargs = PyDict::new(py);
        dumps_kwargs.set_item("allow_nan", false)?;
        let result_json: String = json
            .call_method("dumps", (result,), Some(&dumps_kwargs))?
            .extract()?;
        Ok(serde_json::from_str(&result_json).expect("json module output is valid JSON"))
    })
    .map_err(py_err_to_js)
}

#[op2]
pub async fn op_sleep(ms: f64) {
    tokio::time::sleep(Duration::from_millis(ms as u64)).await;
}

/// CSPRNG bytes for `crypto.getRandomValues` -- this runtime has no other entropy source.
#[op2]
#[serde]
pub fn op_crypto_random_bytes(#[number] len: usize) -> Result<ToJsBuffer, JsErrorBox> {
    let mut buf = vec![0u8; len];
    getrandom::fill(&mut buf).map_err(|e| JsErrorBox::generic(e.to_string()))?;
    Ok(buf.into())
}

/// `crypto.randomUUID` -- v4 (random) UUID, CSPRNG-backed via the `uuid` crate.
#[op2]
#[string]
pub fn op_crypto_random_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use deno_core::{JsRuntime, RuntimeOptions};
    use pyo3::types::PyModule;

    use super::*;
    use crate::runtime::miniclient_extension;
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
            extensions: vec![miniclient_extension()],
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
    fn fs_stat_and_fs_read_hit_the_real_filesystem() {
        let _guard = v8_test_lock();
        let mut js = bare_runtime();
        let dir = std::env::temp_dir();
        let file = dir.join(format!("miniclient-fs-op-{}.txt", std::process::id()));
        std::fs::write(&file, b"file-contents").unwrap();

        assert_eq!(
            eval_string(
                &mut js,
                &format!(
                    "String(Deno.core.ops.op_fs_stat({:?}).isDirectory)",
                    dir.to_str().unwrap()
                )
            ),
            "true"
        );
        assert_eq!(
            eval_string(
                &mut js,
                &format!(
                    "Array.from(new Uint8Array(Deno.core.ops.op_fs_read({:?}))).map(b => String.fromCharCode(b)).join('')",
                    file.to_str().unwrap()
                )
            ),
            "file-contents"
        );
        std::fs::remove_file(&file).ok();
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

    #[test]
    fn op_sleep_actually_delays() {
        let _guard = v8_test_lock();
        let mut js = bare_runtime();
        let start = std::time::Instant::now();
        let tokio = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tokio.block_on(async {
            js.execute_script("<test>", "Deno.core.ops.op_sleep(30)")
                .unwrap();
            js.run_event_loop(deno_core::PollEventLoopOptions::default())
                .await
                .unwrap();
        });
        assert!(start.elapsed() >= std::time::Duration::from_millis(30));
    }

    #[test]
    fn crypto_ops_produce_random_bytes_and_a_uuid() {
        let _guard = v8_test_lock();
        let mut js = bare_runtime();

        assert_eq!(
            eval_string(
                &mut js,
                "new Uint8Array(Deno.core.ops.op_crypto_random_bytes(16)).length",
            ),
            "16"
        );
        // Two draws must differ -- a constant RNG would collide here.
        assert_ne!(
            eval_string(
                &mut js,
                "new Uint8Array(Deno.core.ops.op_crypto_random_bytes(16)).join()"
            ),
            eval_string(
                &mut js,
                "new Uint8Array(Deno.core.ops.op_crypto_random_bytes(16)).join()"
            ),
        );

        let uuid = eval_string(&mut js, "Deno.core.ops.op_crypto_random_uuid()");
        assert_eq!(uuid.len(), 36);
        assert_eq!(uuid.chars().filter(|&c| c == '-').count(), 4);
        assert_ne!(
            uuid,
            eval_string(&mut js, "Deno.core.ops.op_crypto_random_uuid()")
        );
    }
}
