"""The crate's own `Runtime.eval`/`eval_async`: the JSON value contract and JavaScriptError.

Bare isolates, no snapshot and no bootstrap.js -- those arrive with the ops.
"""

import asyncio

import pytest

from miniclient._miniclient import JavaScriptError, Runtime


@pytest.fixture
def runtime():
    r = Runtime()
    yield r
    r.close()


@pytest.mark.parametrize(
    "js, expected",
    [
        ("1 + 1", 2),
        ("'hi'", "hi"),
        ("true", True),
        ("[1, 'two', null]", [1, "two", None]),
        ("({a: {b: [1]}})", {"a": {"b": [1]}}),
        ("undefined", None),
        ("null", None),
        ("void 0", None),
    ],
)
def test_eval_marshals_json_values(runtime, js, expected):
    assert runtime.eval(js) == expected


def test_eval_sees_earlier_state(runtime):
    runtime.eval("globalThis.x = 41")
    assert runtime.eval("x + 1") == 42


async def test_eval_async_resolves_a_promise(runtime):
    # A microtask, not a timer: bare isolates have no setTimeout until the snapshot lands.
    js = "Promise.resolve().then(() => ({ok: 1}))"
    assert await runtime.eval_async(js) == {"ok": 1}


async def test_eval_async_returns_a_plain_value_too(runtime):
    assert await runtime.eval_async("'plain'") == "plain"


def test_a_throw_becomes_a_javascript_error(runtime):
    with pytest.raises(JavaScriptError) as excinfo:
        runtime.eval("function boom() { throw new TypeError('bad thing'); } boom()")
    error = excinfo.value
    assert error.name == "TypeError"
    assert error.message == "bad thing"
    assert error.stack is not None and "boom" in error.stack
    assert str(error) == "bad thing"


async def test_a_rejected_promise_becomes_a_javascript_error(runtime):
    with pytest.raises(JavaScriptError) as excinfo:
        await runtime.eval_async("Promise.reject(new RangeError('too far'))")
    assert excinfo.value.name == "RangeError"
    assert excinfo.value.message == "too far"


def test_a_syntax_error_becomes_a_javascript_error(runtime):
    with pytest.raises(JavaScriptError) as excinfo:
        runtime.eval("this is not javascript")
    assert excinfo.value.name == "SyntaxError"


@pytest.mark.parametrize(
    "js, expected",
    [
        ("() => 1", "function"),
        ("Symbol('s')", "Symbol"),
        ("10n", "BigInt"),
        ("(() => { const o = {}; o.self = o; return o; })()", "circular"),
    ],
)
def test_a_value_json_cannot_carry_raises_and_says_why(runtime, js, expected):
    with pytest.raises(RuntimeError, match=expected) as excinfo:
        runtime.eval(js)
    assert not isinstance(excinfo.value, JavaScriptError)


async def test_eval_async_leaves_the_python_loop_free(runtime):
    # The point of awaiting rather than blocking: the ops mini adds next call back into this
    # very loop, so a blocked loop while JS runs would be a deadlock.
    ticks = 0

    async def tick():
        nonlocal ticks
        while True:
            ticks += 1
            await asyncio.sleep(0)

    ticker = asyncio.ensure_future(tick())
    await asyncio.sleep(0)
    assert await runtime.eval_async(
        "const t = Date.now(); while (Date.now() - t < 200) {} ('done')"
    )
    ticker.cancel()
    assert ticks > 1
