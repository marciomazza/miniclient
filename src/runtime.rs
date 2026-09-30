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
            ops::op_request_navigation(),
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

/// What handling an eval produced: either its answer, or -- under `allow_navigate`, when JS
/// requested navigation -- what `Runtime::new`'s loop needs to perform it. Returned
/// rather than performed here, since only that loop owns `js`/`marshal` outright and can drop
/// the old isolate before building the new one; doing so here, mid-borrow, would leave two
/// `JsRuntime`s alive at once and corrupt V8's handle-scope bookkeeping.
enum EvalStep {
    Done(Option<String>),
    Navigate {
        url: String,
        host_state_json: String,
    },
}

async fn eval(
    js: &mut JsRuntime,
    marshal: &v8::Global<v8::Value>,
    source: String,
    is_async: bool,
    rx: &mut mpsc::UnboundedReceiver<Command>,
    stop: &mut bool,
    allow_navigate: bool,
) -> Result<EvalStep, EvalError> {
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

    // Checked here, once the top-level promise/script is fully settled -- the first safe
    // point to act on `op_request_navigation`, regardless of where it was called from.
    //
    // `allow_navigate` is false only for a reentrant eval, dispatched while another eval further
    // up the call stack is still suspended mid-await and holding v8 handles into this isolate
    // -- navigating there would drop the isolate out from under that suspended frame. Left
    // untaken, the flag just waits in `OpState` for the top-level eval to find it instead.
    if allow_navigate
        && let Some(nav) = js
            .op_state()
            .borrow_mut()
            .try_take::<ops::PendingNavigation>()
    {
        // `value` belongs to a document about to be thrown away -- dropped, not marshaled.
        drop(value);
        let host_state_json = capture_navigation_host_state(js, marshal, &nav);
        return Ok(EvalStep::Navigate {
            url: nav.url,
            host_state_json,
        });
    }

    to_json(js, marshal, value).map(EvalStep::Done)
}

/// Captures the host state the realm being replaced carries forward -- cookies, and session
/// history folded through `nav.mode` the same way `bootstrap.js`'s own `navigateDocument` used
/// to before a navigation replaced that in-realm mechanism. Only reads from the still-alive old
/// isolate; the navigation itself (dropping it) happens elsewhere, see `EvalStep::Navigate`.
fn capture_navigation_host_state(
    js: &mut JsRuntime,
    marshal: &v8::Global<v8::Value>,
    nav: &ops::PendingNavigation,
) -> String {
    let cookies = capture_json(js, marshal, "cookieJar.serializeSync()");
    let history = capture_json(js, marshal, "sessionEntries()");
    let history = history.as_deref().map(|h| apply_navigation_mode(h, nav));
    host_state_json(cookies.as_deref(), history.as_deref())
}

/// The `{cookies, history}` shell `__HOST_STATE__` expects, shared by every capture site
/// (`Page::navigate`, `capture_navigation_host_state`). `cookies`/`history` are already
/// marshaled JSON text, so splicing them in skips a parse-then-reserialize round trip.
fn host_state_json(cookies: Option<&str>, history: Option<&str>) -> String {
    format!(
        r#"{{"cookies":{},"history":{}}}"#,
        cookies.unwrap_or("null"),
        history.unwrap_or("null"),
    )
}

/// Runs `source` (a mini-authored script, never page content) and returns its marshaled JSON --
/// panics on failure, since a throw here is a bug in that script, not something to recover from.
fn capture_json(
    js: &mut JsRuntime,
    marshal: &v8::Global<v8::Value>,
    source: &str,
) -> Option<String> {
    let value = js
        .execute_script("<navigate-capture>", source.to_string())
        .unwrap_or_else(|e| panic!("{source} failed: {e}"));
    to_json(js, marshal, value).unwrap_or_else(|e| panic!("{source} failed to marshal: {e:?}"))
}

/// Applies `nav.mode` to `sessionEntries()`'s `{entries, current}` JSON the way
/// `bootstrap.js`'s `navigateDocument` used to: `push` appends (dropping anything ahead of
/// `current`), `replace` overwrites the current entry, `traverseTo(idx)` just moves `current`
/// -- entries and any `pushState` state on them are left alone.
fn apply_navigation_mode(history_json: &str, nav: &ops::PendingNavigation) -> String {
    let mut history: deno_core::serde_json::Value = deno_core::serde_json::from_str(history_json)
        .expect("sessionEntries() must return valid JSON");
    let current = history["current"]
        .as_u64()
        .expect("sessionEntries().current must be a number") as usize;
    match &nav.mode {
        ops::NavigationMode::Push => {
            let entries = history["entries"]
                .as_array_mut()
                .expect("sessionEntries().entries must be an array");
            entries.truncate(current + 1);
            entries.push(deno_core::serde_json::json!({"url": nav.url, "state": null}));
            history["current"] = deno_core::serde_json::json!(current + 1);
        }
        ops::NavigationMode::Replace => {
            history["entries"][current] =
                deno_core::serde_json::json!({"url": nav.url, "state": null});
        }
        ops::NavigationMode::TraverseTo(idx) => {
            history["current"] = deno_core::serde_json::json!(idx);
        }
    }
    history.to_string()
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
                // `allow_navigate: false` -- reentrant, guarantees `None` back (see `handle_command`).
                let pending = Box::pin(handle_command(js, marshal, command, rx, stop, false)).await;
                debug_assert!(pending.is_none(), "a reentrant eval must never request a navigation");
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
                let pending = Box::pin(handle_command(js, marshal, command, rx, stop, false)).await;
                debug_assert!(pending.is_none(), "a reentrant eval must never request a navigation");
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

/// Everything needed to answer a `Command::Eval`'s caller once the top-level loop has performed
/// the navigation `handle_command` couldn't safely do itself -- see `EvalStep`'s own doc comment.
struct PendingNavigate {
    url: String,
    host_state_json: String,
    reply: oneshot::Sender<EvalOutcome>,
}

/// One command's worth of work, shared by the top-level command loop and the reentrant path
/// in `run_with_interleaved_commands` -- except `Navigate`, which the top-level loop intercepts
/// itself before reaching here. Returns `Some` only when a `Command::Eval` (`allow_navigate: true`)
/// requested a navigation, for the top-level loop to perform it and answer `reply` itself; every
/// other case answers its own `reply` and returns `None`.
async fn handle_command(
    js: &mut JsRuntime,
    marshal: &v8::Global<v8::Value>,
    command: Command,
    rx: &mut mpsc::UnboundedReceiver<Command>,
    stop: &mut bool,
    allow_navigate: bool,
) -> Option<PendingNavigate> {
    match command {
        Command::Close => *stop = true,
        Command::Eval {
            source,
            is_async,
            reply,
        } => {
            match eval(js, marshal, source, is_async, rx, stop, allow_navigate).await {
                // A dropped receiver just means Python stopped caring.
                Ok(EvalStep::Done(value)) => {
                    reply.send(Ok(value)).ok();
                }
                Ok(EvalStep::Navigate {
                    url,
                    host_state_json,
                }) => {
                    return Some(PendingNavigate {
                        url,
                        host_state_json,
                        reply,
                    });
                }
                Err(e) => {
                    reply.send(Err(e)).ok();
                }
            }
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
            // `allow_navigate: false` -- this is mini's own plumbing script, not a caller-visible
            // eval; a pending navigation just waits for the next real one to find it.
            let outcome = match eval(js, marshal, binding_js, false, rx, stop, false).await {
                Ok(EvalStep::Done(value)) => Ok(value),
                Ok(EvalStep::Navigate { .. }) => unreachable!("allow_navigate: false never yields Navigate"),
                Err(e) => Err(e),
            };
            reply.send(outcome).ok();
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
        // eval was still in flight (the reentrant path), which nothing sends yet.
        Command::Navigate { .. } => unreachable!(
            "Command::Navigate must be intercepted by the top-level command loop, never reach handle_command"
        ),
    }
    None
}

/// Boots a fresh isolate and drains its event loop to idle before handing it back -- the one
/// operation shared by initial construction and every navigation alike: a realm isn't actually ready
/// until its `defer`/`type=module` bootstrap scripts (which run asynchronously) have settled, not
/// just after `boot_jsruntime`'s own synchronous scripts return.
async fn boot_and_settle(
    url: &str,
    resource_mounts_json: &str,
    host_state_json: Option<&str>,
    rx: &mut mpsc::UnboundedReceiver<Command>,
    stop: &mut bool,
) -> (JsRuntime, v8::Global<v8::Value>, Result<(), EvalError>) {
    let (mut js, marshal) = boot_jsruntime(url, resource_mounts_json, host_state_json);
    let outcome = run_event_loop_to_idle(&mut js, &marshal, rx, stop).await;
    (js, marshal, outcome)
}

/// Boots a fresh isolate up through `bootstrap.js`: a new `JsRuntime`, the marshaling helper,
/// `__BASE_URL__`/`__RESOURCE_MOUNTS__`, an empty `RustFunctions` table, and (if given)
/// `__HOST_STATE__` -- the `{cookies, history}` blob a navigation carries forward, read once by
/// `bootstrap.js` to rehydrate the cookie jar and seed session history. `host_state_json` is
/// already-encoded JSON, or `None` for a first boot.
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
                let mut stop = false;
                let (mut js, mut marshal, outcome) =
                    boot_and_settle(&url, &resource_mounts_json, None, &mut rx, &mut stop).await;
                outcome.unwrap_or_else(|e| panic!("bootstrap.js failed to settle: {e:?}"));
                ready_tx.send(()).ok();

                // Mutable, not just the constructor's argument: an externally-driven `Navigate`
                // (below) can change it, and a later navigation-triggered navigation reuses
                // whatever it was most recently set to.
                let mut resource_mounts_json = resource_mounts_json;
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
                        url: new_url,
                        resource_mounts_json: new_mounts,
                        host_state_json,
                        reply,
                    } = command
                    else {
                        // A JS-triggered navigation (`EvalStep::Navigate`, from `op_request_navigation`)
                        // comes back the same way: `handle_command` couldn't safely perform it from
                        // inside a borrowed `&mut JsRuntime`, so it's done here instead, right where
                        // `Command::Navigate` above is -- the one place that owns `js`/`marshal` outright.
                        //
                        // Same contract as `Command::Navigate`/`Runtime::send_navigate`: `FetchBackend`/
                        // `RustFunctions` are *not* reinstalled into the new realm here, since
                        // `Page` has no visibility into a navigation this loop triggers on its own.
                        if let Some(PendingNavigate {
                            url: nav_url,
                            host_state_json,
                            reply,
                        }) = handle_command(&mut js, &marshal, command, &mut rx, &mut stop, true)
                            .await
                        {
                            drop(js);
                            let outcome;
                            (js, marshal, outcome) = boot_and_settle(
                                &nav_url,
                                &resource_mounts_json,
                                Some(&host_state_json),
                                &mut rx,
                                &mut stop,
                            )
                            .await;
                            reply.send(outcome.map(|()| None)).ok();
                        }
                        continue;
                    };
                    drop(js);
                    let outcome;
                    (js, marshal, outcome) = boot_and_settle(
                        &new_url,
                        &new_mounts,
                        Some(&host_state_json),
                        &mut rx,
                        &mut stop,
                    )
                    .await;
                    resource_mounts_json = new_mounts;
                    outcome.unwrap_or_else(|e| panic!("navigation failed to settle: {e:?}"));
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
/// rather than replacing it, so the same OS thread backs `self.runtime` for this `Page`'s whole
/// life; only the `JsRuntime` underneath that thread changes. A running isolate can't be torn
/// down from inside its own currently-executing call (e.g. a click handler's promise chain
/// requesting navigation), which rules out destroying and respawning the thread itself.
/// Plain and pyo3-agnostic, mirroring the `Runtime`/pyclass `Runtime` split -- a future
/// pyclass wraps this the same way.
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
    /// `Runtime::send_register_rust_function` alone only runs that script once, at
    /// registration time, and has no realm to re-run it on after a navigation.
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
        let host_state_json = host_state_json(cookies.as_deref(), history.as_deref());
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
