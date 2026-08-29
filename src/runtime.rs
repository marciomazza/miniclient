use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;

use deno_core::error::{CoreErrorKind, JsError};
use deno_core::{Extension, JsRuntime, PollEventLoopOptions, RuntimeOptions, v8};
use pyo3::{Py, PyAny};
use tokio::sync::{mpsc, oneshot};

use crate::ops;

/// The one script mini always evals at construction (never as an ES module -- there is no
/// resolver or loader op). Embedded at compile time rather than read from disk: a wheel's
/// `env!("CARGO_MANIFEST_DIR")` is the build machine's path, absent at runtime. Cargo tracks
/// this file as a build input, so a checkout still rebuilds on edits.
const BOOTSTRAP_JS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/python/miniclient/js/bootstrap.js"
));

/// Safety net only: the platform init below is the actual fix for deno_core#952. Held
/// across isolate construction and destruction, never while a live runtime is in use.
static ISOLATE_LIFECYCLE: Mutex<()> = Mutex::new(());

pub(crate) fn lifecycle_lock() -> MutexGuard<'static, ()> {
    ISOLATE_LIFECYCLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Initializes V8 once per process, before any isolate exists. Snapshot builders must call
/// this too -- their isolates are bound by the same rule.
// No test guards a regression here -- reverting to new_default_platform reproduces
// deno_core#952 only as a heisenbug needing hundreds of cycles under real mixed JS load; 3050
// raw construct/close cycles in a probe never caught it. Upgrade path: an upstream deno_core
// fix, or a CI-nightly stress job if this ever regresses.
pub fn init_platform() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        // Opt-in V8 tuning/profiling, e.g. MINICLIENT_V8_FLAGS="--prof". Must land before
        // v8::Initialize(), which the platform init below triggers.
        if let Ok(flags) = std::env::var("MINICLIENT_V8_FLAGS") {
            v8::V8::set_flags_from_string(&flags);
        }
        // Memory Protection Keys (pkeys) are a CPU feature V8's default platform uses to
        // write-protect its own heap; it demands every V8-touching thread descend from the
        // one that called v8::Initialize(). Each Runtime owns a thread spawned from whichever
        // caller got there first, which violates that: V8 then segfaults inside
        // WasmCodePointerTable::AllocateUninitializedEntry(). The unprotected platform drops
        // pkeys, giving up one defense-in-depth layer against V8-internal memory-corruption
        // exploits -- accepted because mini runs local page JS, not untrusted multi-tenant JS.
        let platform = deno_core::v8::new_unprotected_default_platform(0, false).make_shared();
        JsRuntime::init_platform(Some(platform));
    });
}

/// Order matters: `deno_web` declares `deps = [deno_webidl]`, checked at init time.
pub(crate) fn extensions() -> Vec<Extension> {
    vec![
        deno_webidl::deno_webidl::init(),
        deno_web::deno_web::init(
            deno_web::BlobStore::default_arc(),
            None,
            false,
            Default::default(),
        ),
        miniclient_extension(),
    ]
}

/// Hand-built rather than via `extension!`: the macro buys nothing for a single fixed
/// extension and hides what is actually registered.
pub(crate) fn miniclient_extension() -> Extension {
    Extension {
        name: "miniclient",
        ops: std::borrow::Cow::Owned(vec![
            ops::op_fetch(),
            ops::op_fetch_abort(),
            ops::op_fetch_sync(),
            ops::op_fs_stat(),
            ops::op_fs_read(),
            ops::op_call_python(),
            ops::op_sleep(),
            ops::op_crypto_random_bytes(),
            ops::op_crypto_random_uuid(),
        ]),
        ..Default::default()
    }
}

/// What a script produced, once it is out of V8's hands: JSON text, or nothing at all for
/// `undefined`/`null`.
pub type EvalOutcome = Result<Option<String>, EvalError>;

/// A JS `Error` keeps its own three field names all the way to Python; anything else that
/// can go wrong in an eval is just a message.
pub enum EvalError {
    Js(Box<JsError>),
    Other(String),
}

/// Everything Python asks of the isolate thread crosses as one of these. The ops of §4 land
/// here as further variants, each carrying the channel its answer goes back on.
enum Command {
    Close,
    Eval {
        source: String,
        /// Resolve the result and pump the event loop before answering.
        is_async: bool,
        reply: oneshot::Sender<EvalOutcome>,
    },
    RegisterFunction {
        name: String,
        callable: Py<PyAny>,
        reply: oneshot::Sender<EvalOutcome>,
    },
    InstallHostOps {
        host_ops: ops::HostOps,
        reply: oneshot::Sender<()>,
    },
}

/// `JSON.stringify` answers `undefined` for a function, a Symbol or an `undefined` member and
/// `null` for `NaN`/`Infinity`, each of which would reach Python as a plausible value that is
/// not what JS had. The replacer throws instead, tagged so a refusal is not read as a throw
/// from the page's own getters or `toJSON`.
const MARSHAL_JS: &str = r#"(value) =>
  JSON.stringify(value, function (key, val) {
    const fail = (what) => {
      const at = key === "" ? "the top level" : `key '${key}'`;
      const error = new TypeError(
        `cannot marshal ${what} to Python at ${at}: eval returns JSON values`,
      );
      error.__miniRefusal = true;
      throw error;
    };
    if (typeof val === "function") fail("a function");
    if (typeof val === "symbol") fail("a Symbol");
    if (typeof val === "bigint") fail("a BigInt");
    if (val === undefined) fail("undefined");
    if (typeof val === "number" && !Number.isFinite(val)) fail(String(val));
    // A Map, a typed array, a boxed primitive or any class instance serializes to something
    // that is not it -- `{}`, an index-keyed object, a bare number. `toJSON` ran already, so
    // a Date is a string by now and a class that defines one is a plain object.
    if (val !== null && typeof val === "object") {
      const proto = Object.getPrototypeOf(val);
      if (proto !== Object.prototype && proto !== Array.prototype && proto !== null) {
        fail(`${val.constructor ? val.constructor.name : "an object"} (not a plain object or array)`);
      }
      // The replacer is never handed a symbol-keyed value, only the object holding it.
      // Non-enumerable keys are left alone: JSON drops those whatever their type.
      const symbols = Object.getOwnPropertySymbols(val).filter((s) =>
        Object.prototype.propertyIsEnumerable.call(val, s),
      );
      if (symbols.length) fail(`a Symbol key (${String(symbols[0].description)})`);
    }
    return val;
  })"#;

/// The tag MARSHAL_JS puts on its own refusals.
const REFUSAL_TAG: &str = "__miniRefusal";

/// Marshaling happens here, on the isolate thread, so only JSON text ever crosses back.
fn to_json(
    js: &mut JsRuntime,
    marshal: &v8::Global<v8::Value>,
    value: v8::Global<v8::Value>,
) -> EvalOutcome {
    deno_core::scope!(scope, js);
    let value = v8::Local::new(scope, value);
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let marshal = v8::Local::new(scope, marshal);
    let marshal = v8::Local::<v8::Function>::try_from(marshal).expect("the helper is a function");
    v8::tc_scope!(scope, scope);
    let recv = v8::undefined(scope).into();
    match marshal.call(scope, recv, &[value]) {
        Some(json) => Ok(Some(json.to_rust_string_lossy(scope))),
        None => Err(match scope.exception() {
            Some(exception) => match refusal_message(scope, exception) {
                Some(message) => EvalError::Other(message),
                // A getter, a `toJSON` or a Proxy trap threw: that is the page's own
                // exception, and a circular structure is JSON.stringify's own refusal.
                None => EvalError::Js(JsError::from_v8_exception(scope, exception)),
            },
            None => EvalError::Other("marshaling the result to JSON failed".into()),
        }),
    }
}

/// What MARSHAL_JS refused, or `None` when the exception came from the page's own code.
fn refusal_message(
    scope: &mut v8::PinScope<'_, '_>,
    exception: v8::Local<'_, v8::Value>,
) -> Option<String> {
    let exception = v8::Local::<v8::Object>::try_from(exception).ok()?;
    let tag = v8::String::new(scope, REFUSAL_TAG)?;
    if !exception.get(scope, tag.into())?.is_true() {
        return None;
    }
    let message = v8::String::new(scope, "message")?;
    Some(
        exception
            .get(scope, message.into())?
            .to_rust_string_lossy(scope),
    )
}

async fn eval(
    js: &mut JsRuntime,
    marshal: &v8::Global<v8::Value>,
    source: String,
    is_async: bool,
    rx: &mut mpsc::UnboundedReceiver<Command>,
    stop: &mut bool,
) -> EvalOutcome {
    let value = js.execute_script("<eval>", source).map_err(EvalError::Js)?;
    let value = if is_async {
        let resolve = Box::pin(js.resolve(value));
        run_with_interleaved_commands(js, marshal, resolve, rx, stop).await?
    } else {
        value
    };
    to_json(js, marshal, value)
}

/// Polls `resolve` to completion the same way a plain `.await` on `with_event_loop_promise`
/// would, but also drains `rx` in between polls: a Python callback invoked from an in-flight op
/// (e.g. a mocked fetch transport awaited by `op_fetch`) can call back into `eval`/`eval_async`
/// on the same runtime without the isolate thread deadlocking on itself, since the command loop
/// no longer has to fully finish this eval before it can service another one.
async fn run_with_interleaved_commands<F>(
    js: &mut JsRuntime,
    marshal: &v8::Global<v8::Value>,
    mut resolve: std::pin::Pin<Box<F>>,
    rx: &mut mpsc::UnboundedReceiver<Command>,
    stop: &mut bool,
) -> Result<v8::Global<v8::Value>, EvalError>
where
    F: std::future::Future<Output = Result<v8::Global<v8::Value>, Box<JsError>>>,
{
    loop {
        tokio::select! {
            biased;
            Some(command) = rx.recv() => {
                Box::pin(handle_command(js, marshal, command, rx, stop)).await;
                if *stop {
                    return Err(EvalError::Other(
                        "runtime closed while a request was in flight".into(),
                    ));
                }
            }
            result = js.with_event_loop_promise(&mut resolve, PollEventLoopOptions::default()) => {
                return result.map_err(|e| match e.into_kind() {
                    CoreErrorKind::Js(js_error) => EvalError::Js(js_error),
                    other => EvalError::Other(other.to_string()),
                });
            }
        }
    }
}

/// One command's worth of work, shared between the top-level command loop and the reentrant
/// path in `run_with_interleaved_commands` so both dispatch the same way.
async fn handle_command(
    js: &mut JsRuntime,
    marshal: &v8::Global<v8::Value>,
    command: Command,
    rx: &mut mpsc::UnboundedReceiver<Command>,
    stop: &mut bool,
) {
    match command {
        Command::Close => *stop = true,
        Command::Eval {
            source,
            is_async,
            reply,
        } => {
            // A dropped receiver just means Python stopped caring.
            reply
                .send(eval(js, marshal, source, is_async, rx, stop).await)
                .ok();
        }
        Command::RegisterFunction {
            name,
            callable,
            reply,
        } => {
            let index = {
                let state = js.op_state();
                let mut state = state.borrow_mut();
                state.borrow_mut::<ops::PythonFunctions>().push(callable)
            };
            // Trailing `void 0`: an assignment expression evaluates to the assigned value, and
            // MARSHAL_JS refuses to marshal the function itself back to Python.
            let binding_js = format!(
                "globalThis.{name} = (...args) => Deno.core.ops.op_call_python({index}, args); void 0;"
            );
            reply
                .send(eval(js, marshal, binding_js, false, rx, stop).await)
                .ok();
        }
        Command::InstallHostOps { host_ops, reply } => {
            js.op_state().borrow_mut().put(host_ops);
            reply.send(()).ok();
        }
    }
}

/// A V8 isolate plus the OS thread that exclusively owns it for its whole life.
pub struct Runtime {
    commands: mpsc::UnboundedSender<Command>,
    // Behind a Mutex only so `close()` can take `&self`: pyo3's async methods borrow the
    // pyclass immutably for the whole future, which rules out a `&mut self` sibling.
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Runtime {
    /// `url` becomes the page's initial location, read off `globalThis.__BASE_URL__` before
    /// `newPage()`. `virtual_servers_json` is a JSON array (already-encoded, matching
    /// `__VIRTUAL_SERVERS__`'s shape) read by `bootstrap.js` at the same point -- both globals
    /// must land before it runs, not after, since it reads them once at top level and never again.
    pub fn new(url: &str, virtual_servers_json: &str) -> Self {
        init_platform();
        let (commands, mut rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let url = url.to_string();
        let virtual_servers_json = virtual_servers_json.to_string();
        let thread = std::thread::spawn(move || {
            let tokio = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build the isolate thread's tokio runtime");
            tokio.block_on(async move {
                let mut js = {
                    let _lock = lifecycle_lock();
                    JsRuntime::new(RuntimeOptions {
                        startup_snapshot: Some(crate::snapshot::DEFAULT_SNAPSHOT),
                        extensions: extensions(),
                        ..Default::default()
                    })
                };
                let marshal = js
                    .execute_script("<marshal>", MARSHAL_JS)
                    .expect("the marshaling helper must compile");
                let url_json =
                    deno_core::serde_json::to_string(&url).expect("URL is always JSON-safe");
                js.execute_script(
                    "<runtime-config>",
                    format!("globalThis.__BASE_URL__ = {url_json};"),
                )
                .expect("failed to install __BASE_URL__");
                js.execute_script(
                    "<runtime-config>",
                    format!("globalThis.__VIRTUAL_SERVERS__ = {virtual_servers_json};"),
                )
                .expect("failed to install __VIRTUAL_SERVERS__");
                js.op_state()
                    .borrow_mut()
                    .put(ops::PythonFunctions::default());
                js.execute_script("bootstrap.js", BOOTSTRAP_JS)
                    .unwrap_or_else(|e| panic!("bootstrap.js failed to load: {e}"));
                ready_tx.send(()).ok();

                let mut stop = false;
                while !stop {
                    let Some(command) = rx.recv().await else {
                        break;
                    };
                    handle_command(&mut js, &marshal, command, &mut rx, &mut stop).await;
                }
                let _lock = lifecycle_lock();
                drop(js);
            });
        });
        ready_rx
            .recv()
            .expect("isolate thread died while constructing the runtime");
        Self {
            commands,
            thread: Mutex::new(Some(thread)),
        }
    }

    /// Queues a script and hands back the channel its result will arrive on, so the caller
    /// decides whether to block on it or await it.
    pub fn send_eval(&self, source: String, is_async: bool) -> oneshot::Receiver<EvalOutcome> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Eval {
                source,
                is_async,
                reply,
            })
            .ok();
        rx
    }

    /// Queues a callable registration and hands back the channel its outcome will arrive on --
    /// same shape as `send_eval`, since binding is just one more eval under the hood.
    pub fn send_register_function(
        &self,
        name: String,
        callable: Py<PyAny>,
    ) -> oneshot::Receiver<EvalOutcome> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::RegisterFunction {
                name,
                callable,
                reply,
            })
            .ok();
        rx
    }

    /// Installs the Python callables `op_fetch`/`op_fetch_abort`/`op_fetch_sync` read from
    /// `OpState`. Safe to call any time before JS first reaches one of
    /// those ops -- `bootstrap.js`'s own top-level execution never does, only functions it
    /// defines for later, so this can run after construction rather than needing to land before
    /// `bootstrap.js` loads.
    pub fn send_install_host_ops(&self, host_ops: ops::HostOps) -> oneshot::Receiver<()> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::InstallHostOps { host_ops, reply })
            .ok();
        rx
    }

    /// Cannot return before the isolate is gone: the thread join is the happens-before.
    pub fn close(&self) {
        let Some(thread) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        self.commands.send(Command::Close).ok();
        thread.join().expect("isolate thread panicked");
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::Runtime;
    use crate::snapshot::support;

    #[test]
    fn constructs_and_closes_concurrently() {
        let _guard = support::v8_test_lock();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..3 {
                        Runtime::new("http://localhost/", "[]").close();
                    }
                    // The last one closes via Drop instead.
                    Runtime::new("http://localhost/", "[]");
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}
