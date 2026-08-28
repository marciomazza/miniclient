"""Pins the WHATWG-streams behaviour hx-multipart.js depends on: real backpressure
(`desiredSize`), correct EOF (a pending `read()` waits for a later `enqueue()`/`close()`
instead of reporting done when the queue merely drains), and that the global
`ReadableStream` is the one happy-dom's `Response` accepts. Green on the
`web-streams-polyfill` build, must stay green after the swap to deno_web's `06_streams.js`.
"""

from collections.abc import AsyncIterator
from pathlib import Path

import pytest_asyncio
from conftest import HTMX_SCRIPT_TAG, HTMX_VIRTUAL_SERVER

from miniclient.page import AsyncPage

HX_MULTIPART_TAG = '<script src="http://localhost/vendor/ext/hx-multipart.js"></script>'


@pytest_asyncio.fixture
async def page() -> AsyncIterator[AsyncPage]:
    async with AsyncPage(
        mounts={HTMX_VIRTUAL_SERVER["url"]: Path(HTMX_VIRTUAL_SERVER["directory"])}
    ) as p:
        yield p


async def test_desired_size_backpressure_gates_pull(page: AsyncPage) -> None:
    result = await page.runtime.eval_async("""\
        (async () => {
            let pulls = 0;
            let ctrl;
            const stream = new ReadableStream(
                { start(c) { ctrl = c; }, pull() { pulls++; } },
                { highWaterMark: 2, size: () => 1 },
            );
            const start = ctrl.desiredSize;
            ctrl.enqueue("a");
            const afterOne = ctrl.desiredSize;
            ctrl.enqueue("b");
            ctrl.enqueue("c");
            const overfilled = ctrl.desiredSize;
            await new Promise((r) => setTimeout(r, 5));
            return { start, afterOne, overfilled, pulls };
        })()
    """)
    assert result["start"] == 2
    assert result["afterOne"] == 1
    assert result["overfilled"] == -1  # negative once the queue is over the mark
    assert result["pulls"] == 0  # pull never runs while desiredSize <= 0


async def test_read_waits_for_delayed_enqueue(page: AsyncPage) -> None:
    """The exact regression: a chunk enqueued via setTimeout after the first read()
    must not be lost, and the queue draining must not fake an early {done:true}."""
    result = await page.runtime.eval_async("""\
        (async () => {
            const stream = new ReadableStream({
                start(c) { setTimeout(() => { c.enqueue("a"); c.close(); }, 10); },
            });
            const reader = stream.getReader();
            const first = await reader.read();
            const second = await reader.read();
            return {
                firstValue: first.value, firstDone: first.done,
                secondValue: second.value ?? null, secondDone: second.done,
            };
        })()
    """)
    assert result == {
        "firstValue": "a",
        "firstDone": False,
        "secondValue": None,
        "secondDone": True,
    }


async def test_async_pull_source_delivers_all_chunks_in_order(page: AsyncPage) -> None:
    result = await page.runtime.eval_async("""\
        (async () => {
            let i = 0;
            const stream = new ReadableStream({
                async pull(c) {
                    await new Promise((r) => setTimeout(r, 1));
                    if (i < 5) c.enqueue(i++);
                    else c.close();
                },
            });
            const out = [];
            for await (const chunk of stream) out.push(chunk);
            return out;
        })()
    """)
    assert result == [0, 1, 2, 3, 4]


async def test_response_accepts_the_global_stream(page: AsyncPage) -> None:
    """happy-dom's FetchBodyUtility does `body instanceof ReadableStream` against the
    class the `stream/web` bundle alias re-exports; a Response built from the global
    ReadableStream must round-trip its bytes (guards the shim-identity trap)."""
    result = await page.runtime.eval_async("""\
        (async () => {
            const stream = new ReadableStream({
                start(c) { c.enqueue(new Uint8Array([1, 2, 3])); c.close(); },
            });
            const bytes = await new Response(stream).bytes();
            return Array.from(bytes);
        })()
    """)
    assert result == [1, 2, 3]


async def test_hx_multipart_parses_chunked_straddled_boundaries(page: AsyncPage) -> None:
    """hx-multipart.js's Response.parts() over a body streamed in tiny chunks, so
    boundary markers straddle chunk edges and chunks arrive after the parser's first
    read(). Every part must come through whole and in order."""
    await page.load(f"{HTMX_SCRIPT_TAG}\n{HX_MULTIPART_TAG}\n<div id='x'></div>")
    result = await page.runtime.eval_async("""\
        (async () => {
            const CRLF = String.fromCharCode(13, 10);  // a literal \\r\\n in this source
            const boundary = "BoundaryX";                // string arrives as \\n via the runtime
            const parts = ["first part body", "second part is a bit longer than the first"];
            let payload = "";
            for (const p of parts)
                payload += "--" + boundary + CRLF + "Content-Type: text/plain"
                    + CRLF + CRLF + p + CRLF;
            payload += "--" + boundary + "--" + CRLF;
            const bytes = new TextEncoder().encode(payload);
            const stream = new ReadableStream({
                start(c) {
                    let i = 0;
                    const push = () => {
                        if (i >= bytes.length) { c.close(); return; }
                        c.enqueue(bytes.slice(i, i + 5));  // 5-byte chunks straddle boundaries
                        i += 5;
                        setTimeout(push, 1);
                    };
                    setTimeout(push, 1);
                },
            });
            const res = new Response(stream, {
                headers: { "content-type": `multipart/mixed; boundary=${boundary}` },
            });
            const out = [];
            for await (const part of res.parts()) out.push(await part.text());
            return out;
        })()
    """)
    assert result == ["first part body", "second part is a bit longer than the first"]
