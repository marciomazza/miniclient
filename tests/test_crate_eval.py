"""The crate's own `Runtime.eval`/`eval_async`: the JSON value contract and JavaScriptError.

No `install_host_ops()` call -- fetch ops arrive separately from construction.
"""

import asyncio
import re

import pytest

from miniclient._miniclient import JavaScriptError, Runtime
from miniclient.runtime import production_snapshot


@pytest.fixture
def bare_runtime():
    """Not conftest's `runtime`: no install_host_ops() call, so no fetch support."""
    r = Runtime(production_snapshot(), "http://localhost/")
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


@pytest.mark.parametrize(
    "js, expected",
    [
        # toJSON runs before the replacer, so both of these are plain values by the time the
        # non-plain-object check sees them.
        ("new Date(0)", "1970-01-01T00:00:00.000Z"),
        ("({d: new Date(0)})", {"d": "1970-01-01T00:00:00.000Z"}),
        ("new (class Money { toJSON() { return {cents: 1} } })()", {"cents": 1}),
        ("Object.create(null)", {}),
        # Non-enumerable, so JSON would drop it whatever its key type -- not data.
        ("Object.defineProperty({a: 1}, Symbol('h'), {value: 2})", {"a": 1}),
    ],
)
def test_a_value_that_json_carries_faithfully_still_marshals(bare_runtime, js, expected):
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
        ("10n", "a BigInt"),
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
        # Serializing these loses what they are: {} for a Map, an index-keyed object for a
        # typed array, a bare number for a boxed one.
        ("new Map([[1, 2]])", "Map (not a plain object or array)"),
        ("new Uint8Array([1, 2])", "Uint8Array (not a plain object or array)"),
        ("Object(1)", "Number (not a plain object or array)"),
        ("/re/", "RegExp (not a plain object or array)"),
        (
            "({buf: new ArrayBuffer(2)})",
            "ArrayBuffer (not a plain object or array) to Python at key 'buf'",
        ),
        ("new (class Point { constructor() { this.x = 1 } })()", "Point (not a plain object"),
        # A sync eval of async code, which used to come back as an empty dict.
        ("Promise.resolve(1)", "Promise (not a plain object or array)"),
        ("({[Symbol('k')]: 1})", "a Symbol key (k)"),
        ("({a: 1, [Symbol('k')]: 2})", "a Symbol key (k)"),
    ],
)
def test_a_value_json_cannot_carry_raises_and_says_why(bare_runtime, js, expected):
    with pytest.raises(RuntimeError, match=re.escape(expected)):
        bare_runtime.eval(js)


@pytest.mark.parametrize(
    "js",
    [
        # V8 refuses this one itself, so it arrives as the page's own TypeError.
        "(() => { const o = {}; o.self = o; return o; })()",
        "({get a() { throw new TypeError('from a getter') }})",
        "({toJSON() { throw new TypeError('from toJSON') }})",
    ],
)
def test_a_throw_while_marshaling_stays_a_javascript_error(bare_runtime, js):
    with pytest.raises(JavaScriptError) as excinfo:
        bare_runtime.eval(js)
    assert excinfo.value.name == "TypeError"


def test_a_marshaling_throw_keeps_its_name_and_stack(bare_runtime):
    js = "({get a() { throw new RangeError('from a getter') }})"
    with pytest.raises(JavaScriptError) as excinfo:
        bare_runtime.eval(js)
    assert excinfo.value.name == "RangeError"
    assert excinfo.value.message == "from a getter"
    assert "<eval>" in (excinfo.value.stack or "")


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
