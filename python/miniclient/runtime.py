from __future__ import annotations

import asyncio
import fcntl
import hashlib
import json
import subprocess
from collections.abc import AsyncGenerator, Awaitable, Callable
from contextlib import asynccontextmanager
from functools import cache
from pathlib import Path
from typing import TypedDict

import httpx2 as httpx

from miniclient._miniclient import Runtime, create_snapshot, v8_version

_ROOT = Path(__file__).parent.parent.parent
# build.rs writes _vendor/ into a checkout too, so its presence cannot tell the two apart.
# Cargo.toml sits beside the package only in a checkout; package.json would also match any
# Node project a wheel happens to be installed under.
_IN_CHECKOUT = (_ROOT / "Cargo.toml").exists()
_NM = _ROOT / "node_modules" if _IN_CHECKOUT else Path(__file__).parent / "_vendor"
_JS = Path(__file__).parent / "js"
_POLYFILLS = _JS / "polyfills"
_GENERATED = _JS / "_generated"
_HAPPYDOM_BUNDLE = _GENERATED / "happy-dom-bundle.js"
_SNAPSHOT_WARMUP = _JS / "snapshot_warmup.js"


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
    # flock is not a cross-platform lock lib -- fine since this runtime is Linux/mac-only anyway.
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
    if _IN_CHECKOUT:
        _happydom_bundle_update()
    # else: the packaged wheel ships a pre-built bundle -- trust it.
    return _HAPPYDOM_BUNDLE.read_text()


def get_snapshot_scripts() -> list[tuple[str, str]]:
    """The production scripts, in execution order (shared by prod and test snapshots)."""
    xpath_src = (_NM / "xpath/xpath.js").read_text()
    return [
        ("text-encoding", (_NM / "text-encoding/lib/encoding.js").read_text()),
        (
            "xpath",
            f"""const __xpathLib = {{}};
        (function(exports){{{xpath_src}}})(__xpathLib);
        globalThis.__xpathLib = __xpathLib;""",
        ),
        ("pre_globals", (_JS / "pre_globals.js").read_text()),
        ("formdata", (_JS / "formdata.js").read_text()),
        ("element-registry", (_JS / "element_registry.js").read_text()),
        ("submit", (_JS / "submit.js").read_text()),
        ("happy-dom-bundle", _happydom_bundle_source()),
    ]


def _snapshot_cache_path(scripts: list[tuple[str, str]], warmup: str) -> Path:
    # The V8 build identity belongs in the key as much as the sources do: bumping deno_core
    # leaves every script byte-identical, and V8 refuses a blob stamped by another build --
    # an error pointing nowhere near a cache file nobody knew existed. The key rides in the
    # file name, so a miss is a missing file and a stale blob can never be read back.
    key = hashlib.sha256(v8_version().encode())
    for name, source in [*scripts, ("warmup", warmup)]:
        key.update(f"{name}\0{source}\0".encode())
    return _GENERATED / f"snapshot-{key.hexdigest()[:16]}.bin"


@cache
def production_snapshot() -> bytes:
    """The snapshot of `get_snapshot_scripts()`, cached on disk in a local checkout."""
    scripts = get_snapshot_scripts()
    warmup = _SNAPSHOT_WARMUP.read_text()
    if not _IN_CHECKOUT:
        return create_snapshot(scripts, warmup)
    path = _snapshot_cache_path(scripts, warmup)
    _GENERATED.mkdir(parents=True, exist_ok=True)
    # Same flock as the happy-dom bundle above, for the same reason: parallel test workers
    # would otherwise race each other onto the same output file.
    with open(_GENERATED / ".snapshot.lock", "w") as lock_file:
        fcntl.flock(lock_file, fcntl.LOCK_EX)
        try:
            if not path.exists():
                for stale in _GENERATED.glob("snapshot-*.bin"):
                    stale.unlink()
                path.write_bytes(create_snapshot(scripts, warmup))
            return path.read_bytes()
        finally:
            fcntl.flock(lock_file, fcntl.LOCK_UN)


def _fs_stat_op(path: str) -> dict:
    return {"isDirectory": Path(path).is_dir()}


def _fs_read_op(path: str) -> bytes:
    return Path(path).read_bytes()


def _clean_response_headers(r: httpx.Response) -> list[tuple[str, str]]:
    # httpx already transparently decompresses gzip/br/deflate but keeps the
    # original Content-Encoding/Content-Length headers, which would make a
    # consumer try to decode already-decoded bytes. Strip/fix them here so
    # callers never see a mismatch. Headers are pairs (not a dict) to preserve
    # repeated header names (e.g. multiple Set-Cookie) -- tuples, not lists:
    # the Rust side extracts each pair as `(String, String)`, which pyo3 only
    # accepts from an actual Python tuple, silently dropping every header
    # into an empty Vec (via `.unwrap_or_default()`) if given a 2-element list.
    headers = [
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
    v8_snapshot: bytes | None = None,
    before_fetch: Callable[[dict], Awaitable[None]] | None = None,
    httpx_transport=None,
    virtual_servers: list[VirtualServer] | None = None,
) -> AsyncGenerator[Runtime]:
    """Build a Runtime, pooling one httpx.AsyncClient for every fetch made during
    the context, and tear both the client and the runtime down on exit."""
    async with httpx.AsyncClient(transport=httpx_transport, follow_redirects=True) as client:
        r = Runtime(v8_snapshot or production_snapshot(), url, json.dumps(virtual_servers or []))

        _fetch_op, _fetch_abort_op = _make_fetch_op(before_fetch, client)
        r.install_host_ops(
            _fetch_op,
            _fetch_abort_op,
            _make_fetch_sync_op(client, asyncio.get_running_loop()),
            _fs_stat_op,
            _fs_read_op,
        )

        try:
            yield r
        finally:
            r.close()
