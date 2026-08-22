use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;

use deno_core::error::{CoreErrorKind, JsError};
use deno_core::{Extension, JsRuntime, PollEventLoopOptions, RuntimeOptions, v8};
use tokio::sync::{mpsc, oneshot};

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
pub fn init_platform() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
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

/// Hand-built rather than via `extension!`: the macro buys nothing for a single fixed
/// extension and hides what is actually registered.
pub(crate) fn extension() -> Extension {
    Extension {
        name: "miniclient",
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
) -> EvalOutcome {
    let value = js.execute_script("<eval>", source).map_err(EvalError::Js)?;
    let value = if is_async {
        let resolve = Box::pin(js.resolve(value));
        js.with_event_loop_promise(resolve, PollEventLoopOptions::default())
            .await
            .map_err(|e| match e.into_kind() {
                CoreErrorKind::Js(js_error) => EvalError::Js(js_error),
                other => EvalError::Other(other.to_string()),
            })?
    } else {
        value
    };
    to_json(js, marshal, value)
}

/// A V8 isolate plus the OS thread that exclusively owns it for its whole life.
pub struct Runtime {
    commands: mpsc::UnboundedSender<Command>,
    // Behind a Mutex only so `close()` can take `&self`: pyo3's async methods borrow the
    // pyclass immutably for the whole future, which rules out a `&mut self` sibling.
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Runtime {
    pub fn new() -> Self {
        init_platform();
        let (commands, mut rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let tokio = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build the isolate thread's tokio runtime");
            tokio.block_on(async move {
                let mut js = {
                    let _lock = lifecycle_lock();
                    JsRuntime::new(RuntimeOptions {
                        extensions: vec![extension()],
                        ..Default::default()
                    })
                };
                let marshal = js
                    .execute_script("<marshal>", MARSHAL_JS)
                    .expect("the marshaling helper must compile");
                ready_tx.send(()).ok();

                while let Some(command) = rx.recv().await {
                    match command {
                        Command::Close => break,
                        Command::Eval {
                            source,
                            is_async,
                            reply,
                        } => {
                            // A dropped receiver just means Python stopped caring.
                            reply
                                .send(eval(&mut js, &marshal, source, is_async).await)
                                .ok();
                        }
                    }
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

    /// Cannot return before the isolate is gone: the thread join is the happens-before.
    pub fn close(&self) {
        let Some(thread) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        self.commands.send(Command::Close).ok();
        thread.join().expect("isolate thread panicked");
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
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

    #[test]
    fn constructs_and_closes_concurrently() {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..3 {
                        Runtime::new().close();
                    }
                    // The last one closes via Drop instead.
                    Runtime::new();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}
