from __future__ import annotations

import json
from collections.abc import Awaitable, Callable
from functools import cache
from pathlib import Path
from urllib.parse import urljoin

import httpx2 as httpx
from jsrun import Runtime, RuntimeConfig, SnapshotBuilder

_ROOT = Path(__file__).parent.parent.parent
_NM = _ROOT / "node_modules"
_JS = Path(__file__).parent / "js"
_POLYFILLS = _JS / "polyfills"
_HD_LIB = (_NM / "happy-dom/lib").resolve()
_ENTITIES_ESM = (_NM / "entities/dist/esm/index.js").resolve()
_HTMX_SRC = _ROOT / "vendor/htmx/src/htmx.js"

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
}


def _populate_builder(builder: SnapshotBuilder) -> None:
    """Add all production scripts to a SnapshotBuilder (shared by prod and test snapshots)."""
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
    htmx_source = (
        _HTMX_SRC.read_text()
        .replace("var htmx =", "var Htmx =", 1)
        .replace("return new Htmx()", "return Htmx", 1)
    )
    builder.execute_script("htmx", htmx_source)
    builder.execute_script("htmxclient-submit", (_JS / "submit.js").read_text())


@cache
def _build_snapshot() -> bytes:
    builder = SnapshotBuilder()
    _populate_builder(builder)
    return builder.build()


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
    if spec == "entities":
        return _ENTITIES_ESM.as_uri()
    if spec in _NPM_POLYFILL_FILES:
        return f"npm:{spec}"
    if spec.startswith("file://"):
        return spec
    return None


async def _loader(spec: str) -> str:
    if spec.startswith("node:"):
        return _read_cached(_POLYFILLS / _NODE_POLYFILL_FILES[spec.removeprefix("node:")])
    if spec.startswith("npm:"):
        return _read_cached(_POLYFILLS / _NPM_POLYFILL_FILES[spec.removeprefix("npm:")])
    if spec.startswith("file://"):
        return _read_cached(Path(spec[7:]))
    raise ValueError(f"Cannot load module: {spec!r}")


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
            "headers": dict(r.headers),
            "body": r.content,
        }

    return _fetch_op_impl


async def build_runtime(
    url: str = "http://localhost/",
    snapshot: bytes | None = None,
    before_fetch: Callable[[dict], Awaitable[None]] | None = None,
    httpx_transport=None,
) -> Runtime:
    r = Runtime(RuntimeConfig(snapshot=snapshot or _build_snapshot()))

    r.set_module_resolver(_resolver)
    r.set_module_loader(_loader)

    fetch_op_id = r.register_op(
        "fetch", _make_fetch_op(before_fetch, httpx_transport), mode="async"
    )
    r.eval(f"globalThis.__FETCH_OP_ID__ = {fetch_op_id};")
    r.eval(f"globalThis.__BASE_URL__ = {json.dumps(url)};")

    _bootstrap_uri = (_JS / "bootstrap.js").as_uri()
    await r.eval_module_async(_bootstrap_uri)
    r.eval("var htmx = new Htmx();")
    return r
