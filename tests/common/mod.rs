//! Shared eval helpers for the integration tests. `send_eval` hands back the JSON text of the
//! result (`None` for `undefined`/`null`); these decode it into the Rust value a test asserts on.

#![allow(dead_code)]

pub use _miniclient::runtime::Runtime;
use _miniclient::runtime::{EvalError, EvalOutcome};

/// No `install_host_ops()`: fetch and fs are unavailable, but the default snapshot's happy-dom
/// `document`, `Buffer`, timers and deno_web globals are all live from construction.
pub fn runtime() -> Runtime {
    Runtime::new("http://localhost/", "[]")
}

/// Lets tests write `rt.eval(src)` / `rt.eval_async(src)`. Each snippet is wrapped in a block
/// so its `let`/`const`/`class` bindings don't leak into the shared global scope — one
/// `Runtime` can run many snippets that all declare the same names. The block's completion
/// value is still returned, so keep the last line an expression.
pub trait EvalExt {
    fn eval(&self, src: &str) -> EvalOutcome;
    fn eval_async(&self, src: &str) -> EvalOutcome;
    /// A sync eval run only for its side effect; panics if it threw or was refused.
    fn run(&self, src: &str);
}

impl EvalExt for Runtime {
    fn eval(&self, src: &str) -> EvalOutcome {
        self.send_eval(format!("{{ {src} }}"), false)
            .blocking_recv()
            .expect("isolate thread answered")
    }

    fn eval_async(&self, src: &str) -> EvalOutcome {
        self.send_eval(format!("{{ {src} }}"), true)
            .blocking_recv()
            .expect("isolate thread answered")
    }

    fn run(&self, src: &str) {
        json(self.eval(src));
    }
}

/// The raw JSON text a successful eval produced (`None` for `undefined`/`null`).
pub fn json(outcome: EvalOutcome) -> Option<String> {
    match outcome {
        Ok(value) => value,
        Err(EvalError::Other(message)) => panic!("expected a value, got a refusal: {message}"),
        Err(EvalError::Js(error)) => panic!("expected a value, got a JS {:?}", error.name),
    }
}

pub fn decode<T: serde::de::DeserializeOwned>(outcome: EvalOutcome, what: &str) -> T {
    let raw = json(outcome).expect("eval returned undefined/null");
    serde_json::from_str(&raw).unwrap_or_else(|_| panic!("eval did not return {what}: {raw}"))
}

/// The Rust string a JS-string eval result carries, JSON quoting removed.
pub fn text(outcome: EvalOutcome) -> String {
    decode(outcome, "a JS string")
}

/// The bool a JS-boolean eval result carries.
pub fn boolean(outcome: EvalOutcome) -> bool {
    decode(outcome, "a JS boolean")
}
