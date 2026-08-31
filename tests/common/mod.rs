//! Shared eval helpers for the integration tests. `send_eval` hands back the JSON text of the
//! result; `eval`/`eval_async` decode that straight into the Rust value a test asserts on, and
//! `eval_json` keeps the raw JSON text for tests that compare on JSON shape.

#![allow(dead_code)]

pub use _miniclient::runtime::Runtime;
use _miniclient::runtime::{EvalError, EvalOutcome};

/// No `install_host_ops()`: fetch and fs are unavailable, but the default snapshot's happy-dom
/// `document`, `Buffer`, timers and deno_web globals are all live from construction.
pub fn runtime() -> Runtime {
    Runtime::new("http://localhost/", "[]")
}

/// Lets tests write `rt.eval::<T>(src)` / `rt.eval_async::<T>(src)`. Each snippet is wrapped in a
/// block so its `let`/`const`/`class` bindings don't leak into the shared global scope — one
/// `Runtime` can run many snippets that all declare the same names. The block's completion value
/// is still returned, so keep the last line an expression.
pub trait EvalExt {
    fn eval<T: serde::de::DeserializeOwned>(&self, src: &str) -> T;
    fn eval_async<T: serde::de::DeserializeOwned>(&self, src: &str) -> T;
    /// Raw JSON text of a successful eval (`None` for `undefined`/`null`).
    fn eval_json(&self, src: &str) -> Option<String>;
    fn eval_json_async(&self, src: &str) -> Option<String>;
    /// A sync eval run only for its side effect; panics if it threw or was refused.
    fn run(&self, src: &str);
}

impl EvalExt for Runtime {
    fn eval<T: serde::de::DeserializeOwned>(&self, src: &str) -> T {
        decode(eval_blocking(self, src, false))
    }

    fn eval_async<T: serde::de::DeserializeOwned>(&self, src: &str) -> T {
        decode(eval_blocking(self, src, true))
    }

    fn eval_json(&self, src: &str) -> Option<String> {
        json(eval_blocking(self, src, false))
    }

    fn eval_json_async(&self, src: &str) -> Option<String> {
        json(eval_blocking(self, src, true))
    }

    fn run(&self, src: &str) {
        eval_blocking(self, src, false).expect("eval succeeded");
    }
}

fn eval_blocking(rt: &Runtime, src: &str, is_async: bool) -> EvalOutcome {
    rt.send_eval(format!("{{ {src} }}"), is_async)
        .blocking_recv()
        .expect("isolate thread answered")
}

/// The raw JSON text a successful eval produced (`None` for `undefined`/`null`).
fn json(outcome: EvalOutcome) -> Option<String> {
    match outcome {
        Ok(value) => value,
        Err(EvalError::Other(message)) => panic!("expected a value, got a refusal: {message}"),
        Err(EvalError::Js(error)) => panic!("expected a value, got a JS {:?}", error.name),
    }
}

fn decode<T: serde::de::DeserializeOwned>(outcome: EvalOutcome) -> T {
    let raw = json(outcome).expect("eval returned undefined/null");
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("eval result did not decode ({e}): {raw}"))
}
