//! Shared eval helpers for the integration tests. `send_eval` hands back the JSON text of the
//! result; `eval`/`eval_promise` decode that straight into the Rust value a test asserts on, and
//! `eval_json` keeps the raw JSON text for tests that compare on JSON shape.

#![allow(dead_code)]

pub mod fetch_mock;
pub mod htmx_fetch_mock;

pub use _miniclient::runtime::Runtime;
use _miniclient::runtime::{EvalError, EvalOutcome, Wait};

/// No `install_host_ops()`: fetch and fs are unavailable, but the default snapshot's jsdom
/// `document`, `Buffer`, timers and deno_web globals are all live from construction.
pub fn runtime() -> Runtime {
    Runtime::new("{}")
}

/// Lets tests write `rt.eval::<T>(src)` / `rt.eval_promise::<T>(src)`. Each snippet is wrapped in a
/// block so its `let`/`const`/`class` bindings don't leak into the shared global scope — one
/// `Runtime` can run many snippets that all declare the same names. The block's completion value
/// is still returned, so keep the last line an expression.
pub trait EvalExt {
    fn eval<T: serde::de::DeserializeOwned>(&self, src: &str) -> T;
    fn eval_promise<T: serde::de::DeserializeOwned>(&self, src: &str) -> T;
    /// Raw JSON text of a successful eval (`None` for `undefined`/`null`).
    fn eval_json(&self, src: &str) -> Option<String>;
    fn eval_json_promise(&self, src: &str) -> Option<String>;
    /// An eval run only for its side effect; panics if it threw or was refused.
    fn run(&self, src: &str);
    /// Resolves a returned promise, but does not wait for unrelated page work.
    fn run_promise(&self, src: &str);
    /// The raw outcome, for tests that assert on the error variant.
    fn try_eval(&self, src: &str) -> EvalOutcome;
    fn try_eval_promise(&self, src: &str) -> EvalOutcome;
}

impl EvalExt for Runtime {
    fn eval<T: serde::de::DeserializeOwned>(&self, src: &str) -> T {
        decode(eval_blocking(self, src, Wait::None))
    }

    fn eval_promise<T: serde::de::DeserializeOwned>(&self, src: &str) -> T {
        decode(eval_blocking(self, src, Wait::Promise))
    }

    fn eval_json(&self, src: &str) -> Option<String> {
        json(eval_blocking(self, src, Wait::None))
    }

    fn eval_json_promise(&self, src: &str) -> Option<String> {
        json(eval_blocking(self, src, Wait::Promise))
    }

    fn run(&self, src: &str) {
        eval_blocking(self, src, Wait::None).expect("eval succeeded");
    }

    fn run_promise(&self, src: &str) {
        eval_blocking(self, src, Wait::Promise).expect("eval succeeded");
    }

    fn try_eval(&self, src: &str) -> EvalOutcome {
        eval_blocking(self, src, Wait::None)
    }

    fn try_eval_promise(&self, src: &str) -> EvalOutcome {
        eval_blocking(self, src, Wait::Promise)
    }
}

/// `(name, message, stack)` of the JS `Error` an eval surfaced.
pub fn js_error(outcome: EvalOutcome) -> (String, String, String) {
    match outcome {
        Err(EvalError::Js(error)) => (
            error.name.clone().unwrap_or_default(),
            error.message.clone().unwrap_or_default(),
            error.stack.clone().unwrap_or_default(),
        ),
        Err(EvalError::Other(message)) => panic!("expected a JS error, got a refusal: {message}"),
        Ok(value) => panic!("expected a JS error, got {value:?}"),
    }
}

/// The message of a marshal refusal -- what the `#[pyclass]` wrapper turns into `RuntimeError`.
pub fn refusal(outcome: EvalOutcome) -> String {
    match outcome {
        Err(EvalError::Other(message)) => message,
        Err(EvalError::Js(error)) => panic!("expected a refusal, got a JS {:?}", error.name),
        Ok(value) => panic!("expected a refusal, got {value:?}"),
    }
}

fn eval_blocking(rt: &Runtime, src: &str, wait: Wait) -> EvalOutcome {
    rt.send_eval(format!("{{ {src} }}"), wait)
        .blocking_recv()
        .expect("isolate thread answered")
}

/// The raw JSON text a successful eval produced (`None` for `undefined`/`null`).
fn json(outcome: EvalOutcome) -> Option<String> {
    match outcome {
        Ok(value) => value,
        Err(EvalError::Other(message)) => panic!("expected a value, got a refusal: {message}"),
        Err(EvalError::Js(error)) => panic!(
            "expected a value, got a JS {}: {}",
            error.name.clone().unwrap_or_default(),
            error.message.clone().unwrap_or_default()
        ),
    }
}

fn decode<T: serde::de::DeserializeOwned>(outcome: EvalOutcome) -> T {
    let raw = json(outcome).expect("eval returned undefined/null");
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("eval result did not decode ({e}): {raw}"))
}

/// Decodes an [`EvalOutcome`] a caller already has in hand (e.g. from `browser::Element::eval`),
/// as opposed to [`EvalExt`]'s methods which run the eval themselves.
pub fn decode_outcome<T: serde::de::DeserializeOwned>(outcome: EvalOutcome) -> T {
    decode(outcome)
}

async fn aeval_outcome(rt: &Runtime, src: &str, wait: Wait) -> EvalOutcome {
    rt.send_eval(format!("{{ {src} }}"), wait)
        .await
        .expect("isolate thread answered")
}

/// Async counterparts of [`EvalExt`]'s methods, for use inside a `#[tokio::test]` where
/// `blocking_recv` would panic (the isolate thread never gets to run while this task blocks the
/// executor it's also driven by).
pub async fn aeval_async<T: serde::de::DeserializeOwned>(rt: &Runtime, src: &str) -> T {
    decode(aeval_outcome(rt, src, Wait::Promise).await)
}

pub async fn arun_async(rt: &Runtime, src: &str) {
    aeval_outcome(rt, src, Wait::Promise)
        .await
        .expect("eval succeeded");
}
