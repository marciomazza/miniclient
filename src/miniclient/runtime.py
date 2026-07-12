from __future__ import annotations

import json
from collections.abc import Awaitable, Callable
from functools import cache
from pathlib import Path
from typing import TypedDict
from urllib.parse import urljoin

import httpx2 as httpx
from jsrun import Runtime, RuntimeConfig, SnapshotBuilder

_ROOT = Path(__file__).parent.parent.parent
_BUNDLED = Path(__file__).parent / "_vendor"
_NM = _BUNDLED if _BUNDLED.exists() else _ROOT / "node_modules"
_JS = Path(__file__).parent / "js"
_POLYFILLS = _JS / "polyfills"
_HD_LIB = (_NM / "happy-dom/lib").resolve()
_ENTITIES_ESM = (_NM / "entities/dist/esm/index.js").resolve()

_NODE_POLYFILL_FILES: dict[str, str] = {
    "buffer": "node-buffer.js",
    "child_process": "node-child-process.js",
    "console": "node-console.js",
    "crypto": "node-crypto.js",
    "fs": "node-fs.js",
    "http": "node-http.js",
    "https": "node-http.js",
    "net": "node-net.js",
    "path": "node-path.js",
    "perf_hooks": "node-perf-hooks.js",
    "stream": "node-stream.js",
    "stream/web": "node-stream-web.js",
    "url": "node-url.js",
    "util": "node-util.js",
    "vm": "node-vm.js",
    "zlib": "node-zlib.js",
}

_NPM_POLYFILL_FILES: dict[str, str] = {
    "whatwg-mimetype": "npm-whatwg-mimetype.js",
    "ws": "npm-ws.js",
    "buffer-image-size": "npm-buffer-image-size.js",
}


def get_snapshot_builder() -> SnapshotBuilder:
    """Build a SnapshotBuilder with all production scripts (shared by prod and test snapshots)."""
    builder = SnapshotBuilder()
    builder.execute_script("text-encoding", (_NM / "fast-text-encoding/text.min.js").read_text())
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
    return builder


@cache
def _build_snapshot() -> bytes:
    return get_snapshot_builder().build()  # pragma: no cover


@cache
def _read_cached(path: Path) -> str:
    return path.read_text()


def _resolver(spec: str, ref: str) -> str | None:
    bare = spec.removeprefix("node:")
    if bare in _NODE_POLYFILL_FILES:
        return f"node:{bare}"
    if spec.startswith("./") or spec.startswith("../"):
        return urljoin(ref, spec)
    if spec == "happy-dom":
        return (_HD_LIB / "index.js").as_uri()
    if spec.startswith("happy-dom/lib/"):
        return (_HD_LIB / spec.removeprefix("happy-dom/lib/")).as_uri()
    if spec == "entities":
        return _ENTITIES_ESM.as_uri()
    if spec in _NPM_POLYFILL_FILES:
        return f"npm:{spec}"
    if spec.startswith("file://"):
        return spec
    return None  # pragma: no cover


async def _loader(spec: str) -> str:
    if spec.startswith("node:"):
        return _read_cached(_POLYFILLS / _NODE_POLYFILL_FILES[spec.removeprefix("node:")])
    if spec.startswith("npm:"):
        return _read_cached(_POLYFILLS / _NPM_POLYFILL_FILES[spec.removeprefix("npm:")])
    if spec.startswith("file://"):
        return _read_cached(Path(spec[7:]))
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


def _make_fetch_op(
    before_fetch: Callable[[dict], Awaitable[None]] | None = None,
    httpx_transport=None,
):
    async def _fetch_op_impl(req: dict) -> dict:
        if before_fetch is not None:
            await before_fetch(req)
        body = req.get("body")
        content = bytes(body) if isinstance(body, (bytes, bytearray)) else None
        async with httpx.AsyncClient(transport=httpx_transport) as client:
            r = await client.request(
                req["method"],
                req["url"],
                headers=req.get("headers", {}),
                content=content,
            )
        return {
            "status": r.status_code,
            "statusText": "",
            "headers": _clean_response_headers(r),
            "body": r.content,
            "url": str(r.url),
        }

    return _fetch_op_impl


def _make_fetch_sync_op(httpx_transport=None):
    def _fetch_sync_op_impl(req: dict) -> dict:
        body = req.get("body")
        content = bytes(body) if isinstance(body, (bytes, bytearray)) else None
        with httpx.Client(transport=httpx_transport) as client:
            r = client.request(
                req["method"],
                req["url"],
                headers=req.get("headers", {}),
                content=content,
            )
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


async def build_runtime(
    url: str = "http://localhost/",
    snapshot: bytes | None = None,
    before_fetch: Callable[[dict], Awaitable[None]] | None = None,
    httpx_transport=None,
    virtual_servers: list[VirtualServer] | None = None,
) -> Runtime:
    r = Runtime(RuntimeConfig(snapshot=snapshot or _build_snapshot()))

    r.set_module_resolver(_resolver)
    r.set_module_loader(_loader)

    r.bind_function("__host_fetch", _make_fetch_op(before_fetch, httpx_transport))
    r.bind_function("__host_fetch_sync", _make_fetch_sync_op(httpx_transport))
    r.bind_function("__host_fs_stat", _fs_stat_op)
    r.bind_function("__host_fs_read", _fs_read_op)
    r.eval(f"globalThis.__BASE_URL__ = {json.dumps(url)}")
    r.eval(f"globalThis.__VIRTUAL_SERVERS__ = {json.dumps(virtual_servers or [])}")

    _bootstrap_uri = (_JS / "bootstrap.js").as_uri()
    await r.eval_module_async(_bootstrap_uri)
    return r
