from __future__ import annotations

import asyncio
import json
from collections.abc import AsyncGenerator, Awaitable, Callable
from contextlib import asynccontextmanager
from typing import TypedDict

import httpx2 as httpx

from miniclient._miniclient import Runtime


def _clean_response_headers(r: httpx.Response) -> list[tuple[str, str]]:
    # httpx already transparently decompresses gzip/br/deflate but keeps the
    # original Content-Encoding/Content-Length headers, which would make a
    # consumer try to decode already-decoded bytes. Strip/fix them here so
    # callers never see a mismatch. Headers are pairs (not a dict) to preserve
    # repeated header names (e.g. multiple Set-Cookie) -- tuples, not lists:
    # the Rust side extracts each pair as `(String, String)`, which pyo3 only
    # accepts from an actual Python tuple, silently dropping every header
    # into an empty Vec (via `.unwrap_or_default()`) if given a 2-element list.
    # Set-Cookie set on an intermediate redirect hop (e.g. a login/session rotation that
    # answers 302 + Set-Cookie) lands only in httpx's jar, never in the final response --
    # replay each hop's Set-Cookie ahead of the final headers so happy-dom's jar (bootstrap.js)
    # applies them in order, the final response still winning on any name clash.
    hop_cookies = [
        ("set-cookie", v) for hop in r.history for v in hop.headers.get_list("set-cookie")
    ]
    headers = hop_cookies + [
        (k, v)
        for k, v in r.headers.multi_items()
        if k.lower() not in ("content-encoding", "content-length")
    ]
    headers.append(("content-length", str(len(r.content))))
    return headers


def _sync_client_cookies(httpx_client: httpx.AsyncClient, headers: dict) -> None:
    # httpx follows redirects internally and, on each hop, rebuilds the Cookie header
    # from `httpx_client.cookies` -- it discards whatever Cookie header the request was
    # sent with (httpx2/_client.py:_redirect_headers). happy-dom's own cookie jar
    # (bootstrap.js) is the real source of truth and only attaches a Cookie header to
    # the outgoing request itself, not to any redirect hop -- reseed httpx's jar from
    # that header before every request so a redirect chain still carries it.
    httpx_client.cookies.jar.clear()
    if cookie_header := next((v for k, v in headers.items() if k.lower() == "cookie"), None):
        for pair in cookie_header.split(";"):
            name, _, value = pair.strip().partition("=")
            if name:
                httpx_client.cookies.set(name, value)


def _make_fetch_op(
    before_fetch: Callable[[dict], Awaitable[None]] | None,
    httpx_client: httpx.AsyncClient,
):
    # Tracks in-flight requests by the id JS generates per fetch, so an
    # AbortSignal/ClientRequest.destroy() can cancel the actual network I/O
    # (including a pending before_fetch() gate) instead of just discarding
    # the eventual result.
    pending: dict[str, asyncio.Task] = {}
    # __host_fetch_abort is a sync-bound op, invoked from this runtime's own
    # dedicated thread rather than this event loop's thread -- Task.cancel() isn't
    # thread-safe, so it must be scheduled back onto the loop that owns it.
    loop = asyncio.get_running_loop()

    async def _do_fetch(req: dict) -> dict:
        if before_fetch is not None:
            await before_fetch(req)
        body = req.get("body")
        content = bytes(body) if isinstance(body, (bytes, bytearray)) else None
        headers = req.get("headers", {})
        _sync_client_cookies(httpx_client, headers)
        r = await httpx_client.request(
            req["method"],
            req["url"],
            headers=headers,
            content=content,
        )
        return {
            "status": r.status_code,
            "statusText": "",
            "headers": _clean_response_headers(r),
            "body": r.content,
            "url": str(r.url),
        }

    async def _fetch_op_impl(req: dict) -> dict:
        request_id = req["id"]
        task = asyncio.ensure_future(_do_fetch(req))
        pending[request_id] = task
        try:
            return await task
        except asyncio.CancelledError:
            # Convert rather than re-raise: a bare CancelledError escaping this
            # coroutine would mark the *outer* task (the one pyo3-async-runtimes is
            # awaiting to resolve the JS promise) as cancelled too, since we
            # only meant to cancel our own child task above.
            raise RuntimeError("fetch aborted") from None
        finally:
            pending.pop(request_id, None)

    def _fetch_abort_op(request_id: str) -> None:
        if task := pending.get(request_id):
            loop.call_soon_threadsafe(task.cancel)

    return _fetch_op_impl, _fetch_abort_op


def _make_fetch_sync_op(httpx_client: httpx.AsyncClient, loop: asyncio.AbstractEventLoop):
    # This runtime calls sync-bound functions from its own OS thread, never the
    # loop's thread, so blocking here on a coroutine scheduled onto the loop is safe.
    def _fetch_sync_op_impl(req: dict) -> dict:
        body = req.get("body")
        content = bytes(body) if isinstance(body, (bytes, bytearray)) else None
        headers = req.get("headers", {})
        _sync_client_cookies(httpx_client, headers)
        future = asyncio.run_coroutine_threadsafe(
            httpx_client.request(
                req["method"],
                req["url"],
                headers=headers,
                content=content,
            ),
            loop,
        )
        r = future.result()
        return {
            "status": r.status_code,
            "statusText": "",
            "headers": _clean_response_headers(r),
            "body": r.content,
            "url": str(r.url),
        }

    return _fetch_sync_op_impl


class VirtualServer(TypedDict):
    url: str
    directory: str


@asynccontextmanager
async def open_runtime(
    url: str = "http://localhost/",
    before_fetch: Callable[[dict], Awaitable[None]] | None = None,
    httpx_transport=None,
    virtual_servers: list[VirtualServer] | None = None,
) -> AsyncGenerator[Runtime]:
    """Build a Runtime, pooling one httpx.AsyncClient for every fetch made during
    the context, and tear both the client and the runtime down on exit."""
    async with httpx.AsyncClient(transport=httpx_transport, follow_redirects=True) as client:
        r = Runtime(url, json.dumps(virtual_servers or []))

        _fetch_op, _fetch_abort_op = _make_fetch_op(before_fetch, client)
        r.install_host_ops(
            _fetch_op,
            _fetch_abort_op,
            _make_fetch_sync_op(client, asyncio.get_running_loop()),
        )

        try:
            yield r
        finally:
            r.close()
