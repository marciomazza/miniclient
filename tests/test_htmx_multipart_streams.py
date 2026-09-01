"""Pins the stream integration points hx-multipart.js depends on: the global
`ReadableStream` is the one happy-dom's `Response` accepts, and `Response.parts()`
reassembles a multipart body streamed in tiny straddling chunks. The generic
WHATWG-streams semantics (backpressure, delayed enqueue, async pull) are deno_web's
own concern and tested upstream.
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
