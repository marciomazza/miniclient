//! The crate's own `send_eval` contract -- the JSON value boundary and `EvalError` -- with no
//! host ops installed. Assertions are on the raw JSON text `send_eval` returns, not a decoded
//! value.

mod common;

use common::{EvalExt, js_error, refusal, runtime};

#[test]
fn eval_marshals_json_values() {
    let rt = runtime();
    for (js, want) in [
        ("1 + 1", "2"),
        ("'hi'", "\"hi\""),
        ("true", "true"),
        ("[1, 'two', null]", "[1,\"two\",null]"),
        ("({a: {b: [1]}})", "{\"a\":{\"b\":[1]}}"),
    ] {
        assert_eq!(rt.eval_json(js).as_deref(), Some(want), "{js}");
    }
    for js in ["undefined", "null", "void 0"] {
        assert_eq!(rt.eval_json(js), None, "{js}");
    }
}

#[test]
fn a_value_that_json_carries_faithfully_still_marshals() {
    let rt = runtime();
    for (js, want) in [
        ("new Date(0)", "\"1970-01-01T00:00:00.000Z\""),
        ("({d: new Date(0)})", "{\"d\":\"1970-01-01T00:00:00.000Z\"}"),
        (
            "new (class Money { toJSON() { return {cents: 1}; } })()",
            "{\"cents\":1}",
        ),
        ("Object.create(null)", "{}"),
        (
            "Object.defineProperty({a: 1}, Symbol('h'), {value: 2})",
            "{\"a\":1}",
        ),
    ] {
        assert_eq!(rt.eval_json(js).as_deref(), Some(want), "{js}");
    }
}

#[test]
fn eval_sees_earlier_state() {
    let rt = runtime();
    rt.run("globalThis.x = 41");
    assert_eq!(rt.eval_json("x + 1").as_deref(), Some("42"));
}

#[test]
fn eval_async_resolves_a_promise() {
    let rt = runtime();
    assert_eq!(
        rt.eval_json_async("Promise.resolve().then(() => ({ok: 1}))")
            .as_deref(),
        Some("{\"ok\":1}"),
    );
}

#[test]
fn eval_async_returns_a_plain_value_too() {
    let rt = runtime();
    assert_eq!(rt.eval_json_async("'plain'").as_deref(), Some("\"plain\""));
}

#[test]
fn a_throw_becomes_a_javascript_error() {
    let rt = runtime();
    let (name, message, stack) =
        js_error(rt.try_eval("function boom() { throw new TypeError('bad thing'); } boom()"));
    assert_eq!(name, "TypeError");
    assert_eq!(message, "bad thing");
    assert!(stack.contains("boom"), "stack was {stack:?}");
}

#[test]
fn a_rejected_promise_becomes_a_javascript_error() {
    let rt = runtime();
    let (name, message, _) =
        js_error(rt.try_eval_async("Promise.reject(new RangeError('too far'))"));
    assert_eq!(name, "RangeError");
    assert_eq!(message, "too far");
}

#[test]
fn a_syntax_error_becomes_a_javascript_error() {
    let rt = runtime();
    let (name, ..) = js_error(rt.try_eval("this is not javascript"));
    assert_eq!(name, "SyntaxError");
}

#[test]
fn a_value_json_cannot_carry_raises_and_says_why() {
    let rt = runtime();
    for (js, want) in [
        ("() => 1", "a function"),
        ("Symbol('s')", "a Symbol"),
        ("10n", "a BigInt"),
        ("NaN", "NaN"),
        ("Infinity", "Infinity"),
        ("-Infinity", "-Infinity"),
        ("({f: () => 1})", "a function to Python at key 'f'"),
        ("({s: Symbol('s')})", "a Symbol to Python at key 's'"),
        ("({u: undefined})", "undefined to Python at key 'u'"),
        ("({n: NaN})", "NaN to Python at key 'n'"),
        ("[1, () => 1]", "a function to Python at key '1'"),
        ("[undefined]", "undefined to Python at key '0'"),
        ("({deep: {deeper: [Symbol('s')]}})", "a Symbol"),
        ("new Map([[1, 2]])", "Map (not a plain object or array)"),
        (
            "new Uint8Array([1, 2])",
            "Uint8Array (not a plain object or array)",
        ),
        ("Object(1)", "Number (not a plain object or array)"),
        ("/re/", "RegExp (not a plain object or array)"),
        (
            "({buf: new ArrayBuffer(2)})",
            "ArrayBuffer (not a plain object or array) to Python at key 'buf'",
        ),
        (
            "new (class Point { constructor() { this.x = 1 } })()",
            "Point (not a plain object",
        ),
        (
            "Promise.resolve(1)",
            "Promise (not a plain object or array)",
        ),
        ("({[Symbol('k')]: 1})", "a Symbol key (k)"),
        ("({a: 1, [Symbol('k')]: 2})", "a Symbol key (k)"),
    ] {
        let message = refusal(rt.try_eval(js));
        assert!(message.contains(want), "{js}: {message:?} lacks {want:?}");
    }
}

#[test]
fn a_throw_while_marshaling_stays_a_javascript_error() {
    let rt = runtime();
    for js in [
        "(() => { const o = {}; o.self = o; return o; })()",
        "({get a() { throw new TypeError('from a getter') }})",
        "({toJSON() { throw new TypeError('from toJSON') }})",
    ] {
        let (name, ..) = js_error(rt.try_eval(js));
        assert_eq!(name, "TypeError", "{js}");
    }
}

#[test]
fn a_marshaling_throw_keeps_its_name_and_stack() {
    let rt = runtime();
    let (name, message, stack) =
        js_error(rt.try_eval("({get a() { throw new RangeError('from a getter') }})"));
    assert_eq!(name, "RangeError");
    assert_eq!(message, "from a getter");
    assert!(stack.contains("<eval>"), "stack was {stack:?}");
}
