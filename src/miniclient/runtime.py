from __future__ import annotations

import asyncio
import fcntl
import json
import subprocess
import threading
from collections.abc import AsyncGenerator, Awaitable, Callable
from contextlib import asynccontextmanager
from functools import cache
from pathlib import Path
from typing import TypedDict

import httpx2 as httpx
from jsrun import Runtime, RuntimeConfig, SnapshotBuilder

_ROOT = Path(__file__).parent.parent.parent
_BUNDLED = Path(__file__).parent / "_vendor"
_NM = _BUNDLED if _BUNDLED.exists() else _ROOT / "node_modules"
_JS = Path(__file__).parent / "js"
_POLYFILLS = _JS / "polyfills"
_HAPPYDOM_BUNDLE = _JS / "_generated" / "happy-dom-bundle.js"


def _happydom_bundle_source_list() -> list[Path]:
    # Nearly every file in polyfills/ ends up inlined into the bundle (happy-dom's static
    # import graph reaches almost all Node builtins) -- glob instead of hand-listing, so a
    # new polyfill file can't silently go unwatched the way node-stream-web.js's staleness
    # check originally did.
    return [
        _JS / "build-happydom-bundle.mjs",
        _JS / "happydom-entry.js",
        *_JS.glob("patch-*.js"),
        *_POLYFILLS.glob("*.js"),
        _ROOT / "package-lock.json",
    ]


def _happydom_bundle_update() -> None:
    # flock is not a cross-platform lock lib -- fine since jsrun/deno_core is Linux/mac-only anyway.
    # Guards against parallel test workers racing esbuild onto the same outfile
    # (truncated/partial reads).
    _HAPPYDOM_BUNDLE.parent.mkdir(parents=True, exist_ok=True)
    with open(_HAPPYDOM_BUNDLE.parent / ".happy-dom-bundle.lock", "w") as lock_file:
        fcntl.flock(lock_file, fcntl.LOCK_EX)
        try:
            stale = not _HAPPYDOM_BUNDLE.exists() or any(
                p.stat().st_mtime > _HAPPYDOM_BUNDLE.stat().st_mtime
                for p in _happydom_bundle_source_list()
            )
            if stale:
                # A local checkout (re)builds it here on first use or whenever one of the
                # files above changes, including package-lock.json -- so bumping happy-dom
                # (or any npm dep) triggers a rebuild too.
                subprocess.run(
                    ["node", "build-happydom-bundle.mjs"], cwd=_JS, check=True, capture_output=True
                )  # pragma: no cover
        finally:
            fcntl.flock(lock_file, fcntl.LOCK_UN)


def _happydom_bundle_source() -> str:
    if not _BUNDLED.exists():
        _happydom_bundle_update()
    # else: packaged wheel (_vendor/) ships a pre-built bundle -- trust it.
    return _HAPPYDOM_BUNDLE.read_text()


def get_snapshot_builder() -> SnapshotBuilder:
    """Build a SnapshotBuilder with all production scripts (shared by prod and test snapshots)."""
    builder = SnapshotBuilder()
    builder.execute_script("text-encoding", (_NM / "text-encoding/lib/encoding.js").read_text())
    xpath_src = (_NM / "xpath/xpath.js").read_text()
    builder.execute_script(
        "xpath",
        f"""const __xpathLib = {{}};
        (function(exports){{{xpath_src}}})(__xpathLib);
        globalThis.__xpathLib = __xpathLib;""",
    )
    builder.execute_script("pre_globals", (_JS / "pre_globals.js").read_text())
    builder.execute_script("formdata", (_JS / "formdata.js").read_text())
    builder.execute_script("element-registry", (_JS / "element_registry.js").read_text())
    builder.execute_script("submit", (_JS / "submit.js").read_text())
    builder.execute_script("happy-dom-bundle", _happydom_bundle_source())
    return builder


@cache
def _build_v8_snapshot() -> bytes:
    return get_snapshot_builder().build()  # pragma: no cover


@cache
def _read_cached(path: Path) -> str:
    return path.read_text()


def _resolver(spec: str, ref: str) -> str | None:
    # The only module jsrun ever loads is bootstrap.js itself (a file:// entry URI with
    # no `import`s of its own -- happy-dom is baked into the snapshot, and all page-level
    # <script>/<script type=module> execution is handled by happy-dom's own JS-side fetch
    # and module compiler, not by jsrun's native ES module system). Confirmed by
    # instrumenting this function across the full test suite: every call site is
    # file:///.../bootstrap.js. Anything else is unexpected -- fail loudly rather than
    # silently mis-resolve it.
    if spec.startswith("file://"):
        return spec
    return None  # pragma: no cover


async def _loader(spec: str) -> str:
    if spec.startswith(prefix := "file://"):
        return _read_cached(Path(spec.removeprefix(prefix)))
    raise ValueError(f"Cannot load module: {spec!r}")  # pragma: no cover


def _fs_stat_op(path: str) -> dict:
    return {"isDirectory": Path(path).is_dir()}


def _fs_read_op(path: str) -> bytes:
    return Path(path).read_bytes()


def _clean_response_headers(r: httpx.Response) -> list[list[str]]:
    # httpx already transparently decompresses gzip/br/deflate but keeps the
    # original Content-Encoding/Content-Length headers, which would make a
    # consumer try to decode already-decoded bytes. Strip/fix them here so
    # callers never see a mismatch. Headers are a list of pairs (not a dict)
    # to preserve repeated header names (e.g. multiple Set-Cookie).
    headers = [
        [k, v]
        for k, v in r.headers.multi_items()
        if k.lower() not in ("content-encoding", "content-length")
    ]
    headers.append(["content-length", str(len(r.content))])
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
    # __host_fetch_abort is a sync-bound op, invoked from jsrun's own runtime
    # thread rather than this event loop's thread -- Task.cancel() isn't
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
            # coroutine would mark the *outer* task (the one jsrun/pyo3 is
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
    # jsrun calls sync-bound functions from its own OS thread, never the loop's
    # thread, so blocking here on a coroutine scheduled onto the loop is safe.
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


# jsrun's close() acks before the isolate is actually dropped on its own
# un-joined OS thread, so a Runtime() constructed right after can race that teardown
# on V8's process-wide WasmCodePointerTable (segfault; upstream denoland/deno_core#952).
# This lock+delay keeps construction and close+teardown mutually exclusive across the
# whole process -- it doesn't stop concurrent Runtimes from running, just from
# constructing/closing at the same instant. Ceiling: heuristic delay, not a real fix.
# Remove once jsrun's RuntimeHandle::close() joins the teardown thread before returning
# (see /home/mazza/extra/hc/tmp/handoff-miniclient-jsrun-segfault-20260803-3.md).
_RUNTIME_LIFECYCLE_LOCK = threading.Lock()
_RUNTIME_TEARDOWN_BUFFER_S = 0.01


@asynccontextmanager
async def open_runtime(
    url: str = "http://localhost/",
    v8_snapshot: bytes | None = None,
    before_fetch: Callable[[dict], Awaitable[None]] | None = None,
    httpx_transport=None,
    virtual_servers: list[VirtualServer] | None = None,
) -> AsyncGenerator[Runtime]:
    """Build a Runtime, pooling one httpx.AsyncClient for every fetch made during
    the context, and tear both the client and the runtime down on exit."""
    async with httpx.AsyncClient(transport=httpx_transport, follow_redirects=True) as client:
        with _RUNTIME_LIFECYCLE_LOCK:
            r = Runtime(RuntimeConfig(snapshot=v8_snapshot or _build_v8_snapshot()))

        r.set_module_resolver(_resolver)
        r.set_module_loader(_loader)

        _fetch_op, _fetch_abort_op = _make_fetch_op(before_fetch, client)
        r.bind_function("__host_fetch", _fetch_op)
        r.bind_function("__host_fetch_abort", _fetch_abort_op)
        r.bind_function(
            "__host_fetch_sync", _make_fetch_sync_op(client, asyncio.get_running_loop())
        )
        r.bind_function("__host_fs_stat", _fs_stat_op)
        r.bind_function("__host_fs_read", _fs_read_op)
        r.eval(f"globalThis.__BASE_URL__ = {json.dumps(url)}")
        r.eval(f"globalThis.__VIRTUAL_SERVERS__ = {json.dumps(virtual_servers or [])}")

        _bootstrap_uri = (_JS / "bootstrap.js").as_uri()
        await r.eval_module_async(_bootstrap_uri)

        try:
            yield r
        finally:
            with _RUNTIME_LIFECYCLE_LOCK:
                r.close()
                await asyncio.sleep(_RUNTIME_TEARDOWN_BUFFER_S)
