//! Shared helpers for the Rust ports of the pure-eval Python test modules. `send_eval` hands
//! back the JSON text of the result (`None` for `undefined`/`null`); these decode it into the
//! Rust value a test actually wants to assert on.

#![allow(dead_code)]

use _miniclient::runtime::{EvalError, EvalOutcome, Runtime};

/// No `install_host_ops()`: fetch and fs are unavailable, but the default snapshot's happy-dom
/// `document`, `Buffer`, timers and deno_web globals are all live from construction.
pub fn runtime() -> Runtime {
    Runtime::new("http://localhost/", "[]")
}

pub fn eval(rt: &Runtime, src: &str) -> EvalOutcome {
    rt.send_eval(src.to_string(), false)
        .blocking_recv()
        .expect("isolate thread answered")
}

pub fn eval_async(rt: &Runtime, src: &str) -> EvalOutcome {
    rt.send_eval(src.to_string(), true)
        .blocking_recv()
        .expect("isolate thread answered")
}

/// A sync eval run only for its side effect on the isolate; panics if it threw or was refused.
pub fn run(rt: &Runtime, src: &str) {
    json(eval(rt, src));
}

/// The raw JSON text a successful eval produced (`None` for `undefined`/`null`).
pub fn json(outcome: EvalOutcome) -> Option<String> {
    match outcome {
        Ok(value) => value,
        Err(EvalError::Other(message)) => panic!("expected a value, got a refusal: {message}"),
        Err(EvalError::Js(error)) => panic!("expected a value, got a JS {:?}", error.name),
    }
}

fn decode<T: serde::de::DeserializeOwned>(outcome: EvalOutcome, what: &str) -> T {
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
