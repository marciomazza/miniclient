from __future__ import annotations

import asyncio
from http.cookiejar import DefaultCookiePolicy

import httpx2 as httpx


def new_client(transport: httpx.AsyncBaseTransport | None) -> httpx.AsyncClient:
    # One wire hop per call: redirects, cookies and credentials are the page's (JSDOMDispatcher's)
    # job. httpx's own jar would otherwise overwrite the Cookie header the page decided on.
    client = httpx.AsyncClient(transport=transport)
    client.cookies.jar.set_policy(DefaultCookiePolicy(allowed_domains=[]))
    return client


def _clean_response_headers(r: httpx.Response) -> list[tuple[str, str]]:
    # httpx already transparently decompresses gzip/br/deflate but keeps the
    # original Content-Encoding/Content-Length headers, which would make a
    # consumer try to decode already-decoded bytes. Strip/fix them here so
    # callers never see a mismatch. Headers are pairs (not a dict) to preserve
    # repeated header names (e.g. multiple Set-Cookie), as tuples: the Rust side
    # extracts each pair as `(String, String)`, which rejects a list.
    headers = [
        (k, v)
        for k, v in r.headers.multi_items()
        if k.lower() not in ("content-encoding", "content-length")
    ]
    headers.append(("content-length", str(len(r.content))))
    return headers


def _response_dict(r: httpx.Response) -> dict:
    return {
        "status": r.status_code,
        "statusText": r.reason_phrase,
        "headers": _clean_response_headers(r),
        "body": r.content,
        "url": str(r.url),
    }


def _make_fetch_op(httpx_client: httpx.AsyncClient):
    # Tracks in-flight requests by the id JS generates per fetch, so an
    # AbortSignal/ClientRequest.destroy() can cancel the actual network I/O
    # instead of just discarding the eventual result.
    pending: dict[str, asyncio.Task] = {}
    # Aborts that reached the loop before their request did.
    aborted: set[str] = set()
    # __host_fetch_abort is a sync-bound op, invoked from this runtime's own
    # dedicated thread rather than this event loop's thread -- Task.cancel() isn't
    # thread-safe, so it must be scheduled back onto the loop that owns it.
    loop = asyncio.get_running_loop()

    async def _do_fetch(req: dict) -> dict:
        body = req.get("body")
        content = bytes(body) if isinstance(body, (bytes, bytearray)) else None
        headers = req.get("headers", {})
        r = await httpx_client.request(
            req["method"],
            req["url"],
            headers=headers,
            content=content,
        )
        return _response_dict(r)

    async def _fetch_op_impl(req: dict) -> dict:
        request_id = req["id"]
        if request_id in aborted:
            aborted.discard(request_id)
            raise RuntimeError("fetch aborted")
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

    def _abort(request_id: str) -> None:
        if task := pending.get(request_id):
            task.cancel()
        else:
            aborted.add(request_id)

    def _fetch_abort_op(request_id: str) -> None:
        # The lookup runs on the loop too: the request may still be queued there.
        loop.call_soon_threadsafe(_abort, request_id)

    return _fetch_op_impl, _fetch_abort_op
