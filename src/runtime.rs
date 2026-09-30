use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use deno_core::error::{CoreErrorKind, JsError};
use deno_core::{Extension, JsRuntime, PollEventLoopOptions, RuntimeOptions, v8};
use tokio::sync::{mpsc, oneshot};

use crate::ops;

/// The one script mini always evals at construction (never as an ES module -- there is no
/// resolver or loader op). Embedded at compile time rather than read from disk: a wheel's
/// `env!("CARGO_MANIFEST_DIR")` is the build machine's path, absent at runtime. Cargo tracks
/// this file as a build input, so a checkout still rebuilds on edits.
const BOOTSTRAP_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/js/bootstrap.js"));

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
            ops::op_call_rust(),
            ops::op_crypto_random_bytes(),
            ops::op_crypto_random_uuid(),
        ]),
        ..Default::default()
    }
}

/// What a script produced, once it is out of V8's hands: JSON text, or nothing at all for
/// `undefined`/`null`.
pub type EvalOutcome = Result<Option<String>, EvalError>;

/// A callable `op_call_rust` can dispatch to, shared by `Runtime`'s one-shot registration and
/// `Page`'s `Arc`-based storage (which needs `Sync` too, to survive being cloned into every
/// realm a navigation creates).
pub type RustCallable =
    dyn Fn(Vec<deno_core::serde_json::Value>) -> deno_core::serde_json::Value + Send + Sync;

/// A JS `Error` keeps its own three field names all the way to Python; anything else that
/// can go wrong in an eval is just a message.
#[derive(Debug)]
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
    RegisterRustFunction {
        name: String,
        callable:
            Box<dyn Fn(Vec<deno_core::serde_json::Value>) -> deno_core::serde_json::Value + Send>,
        reply: oneshot::Sender<EvalOutcome>,
    },
    InstallFetchBackend {
        backend: Box<dyn ops::FetchBackend>,
        reply: oneshot::Sender<()>,
    },
    /// Replaces `js`/`marshal` in place with a freshly booted realm -- the OS thread and
    /// command channel this `Runtime` was constructed with are untouched, only the isolate
    /// underneath them changes. `host_state_json` is the `{cookies, history}` blob the caller
    /// captured from the realm being replaced.
    Navigate {
        url: String,
        resource_mounts_json: String,
        host_state_json: String,
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
        let value = run_with_interleaved_commands(js, marshal, resolve, rx, stop).await?;
        // Settle wait: the returned promise resolving only means *that* chain is done, not
        // that the page has gone quiet (a click handler can fire a fetch it never returns,
        // htmx's own dispatch does this routinely). Drain the isolate's real event loop -- not
        // a JS-side task counter -- to idle before handing control back.
        run_event_loop_to_idle(js, marshal, rx, stop).await?;
        value
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

/// Drains the isolate's event loop to idle (`run_event_loop`, not `with_event_loop_promise`:
/// no specific promise to resolve, just every pending op/timer), same interleaving as
/// `run_with_interleaved_commands` for the same reentrancy reason.
async fn run_event_loop_to_idle(
    js: &mut JsRuntime,
    marshal: &v8::Global<v8::Value>,
    rx: &mut mpsc::UnboundedReceiver<Command>,
    stop: &mut bool,
) -> Result<(), EvalError> {
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
            result = js.run_event_loop(PollEventLoopOptions::default()) => {
                return result.map_err(|e| match e.into_kind() {
                    CoreErrorKind::Js(js_error) => EvalError::Js(js_error),
                    other => EvalError::Other(other.to_string()),
                });
            }
        }
    }
}

/// One command's worth of work, shared between the top-level command loop and the reentrant
/// path in `run_with_interleaved_commands` so both dispatch the same way -- except `Navigate`,
/// which the top-level loop intercepts and handles itself (see its own comment) before a
/// command ever reaches here.
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
        Command::RegisterRustFunction {
            name,
            callable,
            reply,
        } => {
            let index = {
                let state = js.op_state();
                let mut state = state.borrow_mut();
                state.borrow_mut::<ops::RustFunctions>().push(callable)
            };
            let binding_js = format!(
                "globalThis.{name} = (...args) =>
  Deno.core.ops.op_call_rust({index}, args);
void 0;"
            );
            reply
                .send(eval(js, marshal, binding_js, false, rx, stop).await)
                .ok();
        }
        Command::InstallFetchBackend { backend, reply } => {
            let backend: Rc<dyn ops::FetchBackend> = Rc::from(backend);
            js.op_state().borrow_mut().put(backend);
            reply.send(()).ok();
        }
        // Constructing a second `JsRuntime` while this one is still alive corrupts V8's
        // handle-scope bookkeeping (only one isolate may be current on a thread at a time), so
        // a navigation needs `js`/`marshal` dropped, not just borrowed -- see the top-level
        // loop's own handling of this variant. Reaching here means `Navigate` arrived while an
        // eval was still in flight (the reentrant path), which nothing sends yet -- the
        // safe-point-only timing ticket 11's navigation trigger is responsible for guaranteeing.
        Command::Navigate { .. } => unreachable!(
            "Command::Navigate must be intercepted by the top-level command loop, never reach handle_command"
        ),
    }
}

/// Boots a fresh isolate up through `bootstrap.js`: a new `JsRuntime`, the marshaling helper,
/// `__BASE_URL__`/`__RESOURCE_MOUNTS__`, an empty `RustFunctions` table, and (if given)
/// `__HOST_STATE__` -- the `{cookies, history}` blob a realm navigation carries forward from the realm
/// it replaces, read once by `bootstrap.js` at top level to rehydrate the cookie jar and seed
/// session history before anything else runs. `host_state_json` is already-encoded JSON, or
/// `None` for a first boot with nothing to carry forward.
fn boot_jsruntime(
    url: &str,
    resource_mounts_json: &str,
    host_state_json: Option<&str>,
) -> (JsRuntime, v8::Global<v8::Value>) {
    let mut js = JsRuntime::new(RuntimeOptions {
        startup_snapshot: Some(crate::snapshot::DEFAULT_SNAPSHOT),
        extensions: extensions(),
        ..Default::default()
    });
    let marshal = js
        .execute_script("<marshal>", MARSHAL_JS)
        .expect("the marshaling helper must compile");
    let url_json = deno_core::serde_json::to_string(url).expect("URL is always JSON-safe");
    js.execute_script(
        "<runtime-config>",
        format!("globalThis.__BASE_URL__ = {url_json};"),
    )
    .expect("failed to install __BASE_URL__");
    js.execute_script(
        "<runtime-config>",
        format!("globalThis.__RESOURCE_MOUNTS__ = {resource_mounts_json};"),
    )
    .expect("failed to install __RESOURCE_MOUNTS__");
    js.execute_script(
        "<runtime-config>",
        format!(
            "globalThis.__HOST_STATE__ = {};",
            host_state_json.unwrap_or("null")
        ),
    )
    .expect("failed to install __HOST_STATE__");
    js.op_state()
        .borrow_mut()
        .put(ops::RustFunctions::default());
    js.execute_script("bootstrap.js", BOOTSTRAP_JS)
        .unwrap_or_else(|e| panic!("bootstrap.js failed to load: {e}"));
    (js, marshal)
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
    /// `newPage()`. `resource_mounts_json` is a JSON object (already-encoded, matching
    /// `__RESOURCE_MOUNTS__`'s shape: `{url_prefix: directory}`) read by `bootstrap.js` at the
    /// same point -- both globals must land before it runs, not after, since it reads them once
    /// at top level and never again.
    pub fn new(url: &str, resource_mounts_json: &str) -> Self {
        init_platform();
        let (commands, mut rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let url = url.to_string();
        let resource_mounts_json = resource_mounts_json.to_string();
        let thread = std::thread::spawn(move || {
            let tokio = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build the isolate thread's tokio runtime");
            tokio.block_on(async move {
                let (mut js, mut marshal) = boot_jsruntime(&url, &resource_mounts_json, None);
                ready_tx.send(()).ok();

                let mut stop = false;
                while !stop {
                    let Some(command) = rx.recv().await else {
                        break;
                    };
                    // Handled here, not inside `handle_command`: `js`/`marshal` are owned by
                    // this loop, never borrowed by anything nested at this point (no eval is in
                    // flight), so the old isolate can be dropped -- disposed, no longer V8's
                    // "current" isolate on this thread -- strictly before the new one is
                    // constructed. Building a second `JsRuntime` while the first is still alive
                    // corrupts V8's handle-scope bookkeeping and aborts the process.
                    let Command::Navigate {
                        url,
                        resource_mounts_json,
                        host_state_json,
                        reply,
                    } = command
                    else {
                        handle_command(&mut js, &marshal, command, &mut rx, &mut stop).await;
                        continue;
                    };
                    drop(js);
                    (js, marshal) =
                        boot_jsruntime(&url, &resource_mounts_json, Some(&host_state_json));
                    reply.send(()).ok();
                }
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

    /// Binds `name` to a Rust closure via `op_call_rust`, queuing the binding on the isolate
    /// thread and handing back the channel its outcome will arrive on.
    pub fn send_register_rust_function(
        &self,
        name: String,
        callable: Box<
            dyn Fn(Vec<deno_core::serde_json::Value>) -> deno_core::serde_json::Value + Send,
        >,
    ) -> oneshot::Receiver<EvalOutcome> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::RegisterRustFunction {
                name,
                callable,
                reply,
            })
            .ok();
        rx
    }

    /// Installs the backend `op_fetch`/`op_fetch_abort`/`op_fetch_sync` read from `OpState`.
    /// Safe to call any time before JS first reaches one of those ops -- `bootstrap.js`'s own
    /// top-level execution never does, only functions it defines for later, so this can run
    /// after construction rather than needing to land before `bootstrap.js` loads.
    pub fn send_install_fetch_backend(
        &self,
        backend: Box<dyn ops::FetchBackend>,
    ) -> oneshot::Receiver<()> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::InstallFetchBackend { backend, reply })
            .ok();
        rx
    }

    /// Navigates to a brand-new realm booted from `url`/`resource_mounts_json`, with
    /// `host_state_json` (already JSON-encoded) injected as `globalThis.__HOST_STATE__` before
    /// its `bootstrap.js` runs -- same OS thread and command channel throughout, only the
    /// isolate underneath them changes. Callers must re-install any `FetchBackend`/
    /// `RustFunctions` afterward: a fresh isolate means fresh `OpState`, nothing survives on
    /// its own.
    pub fn send_navigate(
        &self,
        url: &str,
        resource_mounts_json: &str,
        host_state_json: &str,
    ) -> oneshot::Receiver<()> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Navigate {
                url: url.to_string(),
                resource_mounts_json: resource_mounts_json.to_string(),
                host_state_json: host_state_json.to_string(),
                reply,
            })
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

/// Host-owned state that must survive a realm navigation, plus the [`Runtime`] it currently
/// drives. Owns one `Runtime` at a time -- [`Page::navigate`] sends it a `Navigate` command
/// rather than replacing it, so the same OS thread backs `self.runtime` for this `Page`'s whole life;
/// only the `JsRuntime` underneath that thread changes (see `.scratch/browser-design/map.md`'s
/// rejection of a full destroy-and-respawn-the-thread design: a running isolate can't be torn
/// down from inside its own currently-executing call, e.g. a click handler's promise chain
/// requesting navigation). Plain and pyo3-agnostic, mirroring the `Runtime`/pyclass `Runtime`
/// split -- a future pyclass wraps this the same way.
pub struct Page {
    runtime: Runtime,
    cookies: Option<String>,
    history: Option<String>,
    fetch_backend: Option<Arc<dyn ops::FetchBackend + Send + Sync>>,
    rust_functions: Vec<(String, Arc<RustCallable>)>,
}

impl Page {
    /// `url`/`resource_mounts_json` boot the first realm exactly as [`Runtime::new`] would --
    /// no host state to carry forward yet, nothing has been captured.
    pub fn new(url: &str, resource_mounts_json: &str) -> Self {
        Self {
            runtime: Runtime::new(url, resource_mounts_json),
            cookies: None,
            history: None,
            fetch_backend: None,
            rust_functions: Vec::new(),
        }
    }

    /// The `Runtime` currently backing this `Page` -- callers drive it with the same
    /// `send_eval`/`send_eval_async` API `Runtime` always exposes.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Installs `backend` on the current realm and remembers it so [`Page::navigate`] can
    /// re-install it on every realm that replaces this one.
    pub fn install_fetch_backend(&mut self, backend: Arc<dyn ops::FetchBackend + Send + Sync>) {
        install_fetch_backend_on(&self.runtime, &backend);
        self.fetch_backend = Some(backend);
    }

    /// Binds `name` on the current realm and remembers the binding so [`Page::navigate`] can
    /// re-execute its `globalThis.name = ...` script on every realm that replaces this one --
    /// today's `Runtime::send_register_rust_function` only runs that script once, at
    /// registration time, which breaks the moment a navigation happens afterward.
    pub fn register_rust_function(&mut self, name: String, callable: Arc<RustCallable>) {
        replay_rust_function(&self.runtime, &name, &callable);
        self.rust_functions.push((name, callable));
    }

    /// Captures every piece of host-owned state from the current realm, then navigates
    /// `self.runtime` onto a brand-new realm (via `Runtime::send_navigate`, itself
    /// `boot_jsruntime` run on the same OS thread) and re-injects that state into it: cookies
    /// (`cookieJar.serializeSync()` / `CookieJar.deserializeSync()`), session history (same
    /// `__HOST_STATE__`-before-boot seam), the installed `FetchBackend`, and every registered
    /// `RustFunctions` binding -- a fresh isolate means fresh `OpState`, so the last two don't
    /// survive on their own.
    pub fn navigate(&mut self, url: &str, resource_mounts_json: &str) {
        let cookies = eval_or_panic(&self.runtime, "cookieJar.serializeSync()");
        let history = eval_or_panic(&self.runtime, "sessionEntries()");

        // Both fragments are already-valid JSON text (marshaled by `eval`'s own
        // `JSON.stringify` round trip), so splicing them into the `{...}` shell is safe without
        // a parse-then-reserialize round trip through `serde_json::Value`.
        let host_state_json = format!(
            "{{\"cookies\":{},\"history\":{}}}",
            cookies.as_deref().unwrap_or("null"),
            history.as_deref().unwrap_or("null"),
        );
        self.runtime
            .send_navigate(url, resource_mounts_json, &host_state_json)
            .blocking_recv()
            .expect("navigate channel dropped");

        if let Some(backend) = &self.fetch_backend {
            install_fetch_backend_on(&self.runtime, backend);
        }
        for (name, callable) in &self.rust_functions {
            replay_rust_function(&self.runtime, name, callable);
        }

        self.cookies = cookies;
        self.history = history;
    }
}

/// Runs `source` and unwraps its outcome -- host-state capture/replay scripts are mini's own,
/// never page-authored, so a throw here is a bug in that script, not something a caller needs to
/// recover from.
fn eval_or_panic(runtime: &Runtime, source: &str) -> Option<String> {
    runtime
        .send_eval(source.to_string(), false)
        .blocking_recv()
        .expect("eval channel dropped")
        .unwrap_or_else(|e| panic!("{source} failed: {e:?}"))
}

/// Installs `backend` on `runtime`'s current realm -- shared between first installation and
/// every later navigation-replay so both stay in sync.
fn install_fetch_backend_on(runtime: &Runtime, backend: &Arc<dyn ops::FetchBackend + Send + Sync>) {
    runtime
        .send_install_fetch_backend(Box::new(backend.clone()))
        .blocking_recv()
        .expect("fetch backend install channel dropped");
}

/// Re-executes a `RustFunctions` binding's `globalThis.name = ...` script on `runtime` --
/// shared between first registration and every later navigation-replay so both stay in sync.
fn replay_rust_function(runtime: &Runtime, name: &str, callable: &Arc<RustCallable>) {
    let callable = callable.clone();
    let bound = move |args| (*callable)(args);
    runtime
        .send_register_rust_function(name.to_string(), Box::new(bound))
        .blocking_recv()
        .expect("register channel dropped")
        .unwrap_or_else(|e| panic!("registering {name} failed: {e:?}"));
}

#[cfg(test)]
mod tests {
    use super::Runtime;

    #[test]
    fn constructs_and_closes_concurrently() {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..3 {
                        Runtime::new("http://localhost/", "{}").close();
                    }
                    // The last one closes via Drop instead.
                    Runtime::new("http://localhost/", "{}");
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}

#[cfg(test)]
mod settle_regression {
    use super::Runtime;

    // A cancelled setTimeout's underlying op_sleep promise used to stay refed until its
    // original delay elapsed (pre_globals.js only dropped the JS callback on clearTimeout(),
    // never unrefed the op), so a page that cleared a long timer -- e.g. htmx's own 60s
    // per-request timeout, cleared the instant the response arrives -- would silently pin
    // every subsequent eval_async's native settle-wait until that timer's original delay ran
    // out. Regression test for that fix: clearTimeout() must unref immediately.
    #[test]
    fn clearing_a_long_timeout_does_not_pin_native_settle_wait() {
        let rt = Runtime::new("http://localhost/", "{}");
        rt.send_eval(
            "const id = setTimeout(() => {}, 60000); clearTimeout(id); void 0;".into(),
            false,
        )
        .blocking_recv()
        .expect("channel must not drop")
        .expect("script must not error");

        let started = std::time::Instant::now();
        rt.send_eval("Promise.resolve(1)".into(), true)
            .blocking_recv()
            .expect("channel must not drop")
            .expect("eval_async must not error");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "eval_async waited on a cleared timer's original delay instead of returning immediately"
        );
    }
}
