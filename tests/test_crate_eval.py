"""The crate's own `Runtime.eval`/`eval_async`: the JSON value contract and JavaScriptError.

Bare isolates, no snapshot and no bootstrap.js -- those arrive with the ops.
"""

import asyncio
import re

import pytest

from miniclient._miniclient import JavaScriptError, Runtime


@pytest.fixture
def bare_runtime():
    """Not conftest's `runtime`: no snapshot, no happy-dom, no bootstrap.js."""
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
def test_eval_marshals_json_values(bare_runtime, js, expected):
    assert bare_runtime.eval(js) == expected


def test_eval_sees_earlier_state(bare_runtime):
    bare_runtime.eval("globalThis.x = 41")
    assert bare_runtime.eval("x + 1") == 42


async def test_eval_async_resolves_a_promise(bare_runtime):
    # A microtask, not a timer: bare isolates have no setTimeout until the snapshot lands.
    js = "Promise.resolve().then(() => ({ok: 1}))"
    assert await bare_runtime.eval_async(js) == {"ok": 1}


async def test_eval_async_returns_a_plain_value_too(bare_runtime):
    assert await bare_runtime.eval_async("'plain'") == "plain"


def test_a_throw_becomes_a_javascript_error(bare_runtime):
    with pytest.raises(JavaScriptError) as excinfo:
        bare_runtime.eval("function boom() { throw new TypeError('bad thing'); } boom()")
    error = excinfo.value
    assert error.name == "TypeError"
    assert error.message == "bad thing"
    assert "boom" in (error.stack or "")
    assert str(error) == "bad thing"


async def test_a_rejected_promise_becomes_a_javascript_error(bare_runtime):
    with pytest.raises(JavaScriptError) as excinfo:
        await bare_runtime.eval_async("Promise.reject(new RangeError('too far'))")
    assert excinfo.value.name == "RangeError"
    assert excinfo.value.message == "too far"


def test_a_syntax_error_becomes_a_javascript_error(bare_runtime):
    with pytest.raises(JavaScriptError) as excinfo:
        bare_runtime.eval("this is not javascript")
    assert excinfo.value.name == "SyntaxError"


@pytest.mark.parametrize(
    "js, expected",
    [
        ("() => 1", "a function"),
        ("Symbol('s')", "a Symbol"),
        ("10n", "BigInt"),
        ("(() => { const o = {}; o.self = o; return o; })()", "circular"),
        ("NaN", "NaN"),
        ("Infinity", "Infinity"),
        ("-Infinity", "-Infinity"),
        # Nested, where JSON.stringify would drop the member or write null in its place.
        ("({f: () => 1})", "a function to Python at key 'f'"),
        ("({s: Symbol('s')})", "a Symbol to Python at key 's'"),
        ("({u: undefined})", "undefined to Python at key 'u'"),
        ("({n: NaN})", "NaN to Python at key 'n'"),
        ("[1, () => 1]", "a function to Python at key '1'"),
        ("[undefined]", "undefined to Python at key '0'"),
        ("({deep: {deeper: [Symbol('s')]}})", "a Symbol"),
    ],
)
def test_a_value_json_cannot_carry_raises_and_says_why(bare_runtime, js, expected):
    with pytest.raises(RuntimeError, match=re.escape(expected)):
        bare_runtime.eval(js)


async def test_eval_async_leaves_the_python_loop_free(bare_runtime):
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
    assert await bare_runtime.eval_async(
        "const t = Date.now(); while (Date.now() - t < 50) {} ('done')"
    )
    ticker.cancel()
    assert ticks > 1
