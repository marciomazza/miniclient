from __future__ import annotations

import asyncio
import json
from pathlib import Path
from urllib.parse import urljoin

import httpx
from jsrun import Runtime

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


_module_source_cache: dict[str, str] = {}


def _read_cached(path: Path) -> str:
    key = str(path)
    if key not in _module_source_cache:
        _module_source_cache[key] = path.read_text()
    return _module_source_cache[key]


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


async def _fetch_op(req: dict) -> dict:
    body = req.get("body")
    content = bytes(body) if isinstance(body, (bytes, bytearray)) else None
    async with httpx.AsyncClient() as client:
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


async def build_browser(url: str = "http://localhost/") -> Runtime:
    r = Runtime()
    r.eval((_NM / "fast-text-encoding/text.min.js").read_text())
    xpath_src = (_NM / "xpath/xpath.js").read_text()
    r.eval(
        f"""const __xpathLib = {{}};
        (function(exports){{{xpath_src}}})(__xpathLib);
        globalThis.__xpathLib = __xpathLib;"""
    )
    r.eval((_JS / "pre_globals.js").read_text())
    r.eval((_JS / "formdata.js").read_text())
    htmx_source = (
        _HTMX_SRC.read_text()
        .replace("var htmx =", "var Htmx =", 1)
        .replace("return new Htmx()", "return Htmx", 1)
    )

    await r.eval_async(htmx_source)

    r.set_module_resolver(_resolver)
    r.set_module_loader(_loader)

    fetch_op_id = r.register_op("fetch", _fetch_op, mode="async")
    r.eval(f"globalThis.__FETCH_OP_ID__ = {fetch_op_id};")

    pending_timers: dict[int, asyncio.Event] = {}

    async def _sleep_op(req: dict[str, int]) -> dict:
        timer_id = req["id"]
        ms = req.get("ms", 0)
        cancel = asyncio.Event()
        pending_timers[timer_id] = cancel
        try:
            sleep_task = asyncio.ensure_future(asyncio.sleep(max(ms, 0) / 1000))
            cancel_task = asyncio.ensure_future(cancel.wait())
            done, pending = await asyncio.wait(
                [sleep_task, cancel_task],
                return_when=asyncio.FIRST_COMPLETED,
            )
            for t in pending:
                t.cancel()
            return {"cancelled": cancel.is_set()}
        finally:
            pending_timers.pop(timer_id, None)

    async def _clear_timer_op(req: dict[str, int]) -> dict:
        timer_id = req.get("id")
        if cancel := timer_id and pending_timers.get(timer_id):
            cancel.set()
        return {}

    sleep_op_id = r.register_op("sleep", _sleep_op, mode="async")
    clear_timer_op_id = r.register_op("clear_timer", _clear_timer_op, mode="async")
    r.eval(f"globalThis.__SLEEP_OP_ID__ = {sleep_op_id};")
    r.eval(f"globalThis.__CLEAR_TIMER_OP_ID__ = {clear_timer_op_id};")
    r.eval(f"globalThis.__BASE_URL__ = {json.dumps(url)};")

    for bare, fname in _NODE_POLYFILL_FILES.items():
        r.add_static_module(f"node:{bare}", _read_cached(_POLYFILLS / fname))
    for name, fname in _NPM_POLYFILL_FILES.items():
        r.add_static_module(f"npm:{name}", _read_cached(_POLYFILLS / fname))
    for js_path in [_JS / "urlsearch-dom-patches.js", _JS / "patch-dom-parser.js"]:
        r.add_static_module(js_path.as_uri(), _read_cached(js_path))

    _bootstrap_uri = (_JS / "bootstrap.js").as_uri()
    r.add_static_module(_bootstrap_uri, _read_cached(_JS / "bootstrap.js"))
    await r.eval_module_async(_bootstrap_uri)
    r.eval("var htmx = new Htmx();")
    # Drain any setTimeout(fn, 0) calls made during htmx init so their
    # _sleep_op coroutines complete before the caller's event loop exits.
    for _ in range(4):
        await asyncio.sleep(0)
    return r


class Browser:
    def __init__(self, runtime: Runtime) -> None:
        self.runtime = runtime

    @classmethod
    async def create(cls, url: str = "http://localhost/") -> Browser:
        r = await build_browser(url)
        return cls(r)

    async def load(self, html: str) -> None:
        """Set document body and initialize htmx on the new content."""
        self.runtime.eval(f"document.body.innerHTML = {json.dumps(html)};")
        self.runtime.eval("htmx.process(document.body);")

    async def trigger(self, selector: str, event: str = "click") -> None:
        """Dispatch a DOM event and wait for htmx to settle."""
        js = f"""
        new Promise((resolve, reject) => {{
            document.addEventListener('htmx:after:settle', () => resolve(), {{once: true}});
            document.addEventListener('htmx:error', (e) => {{
                reject(new Error('htmx:error — ' + (e.detail?.error ?? e.detail?.ctx?.status)));
            }}, {{once: true}});
            const el = document.querySelector({json.dumps(selector)});
            if (!el) {{
                reject(new Error('Element not found: {selector}'));
            }} else {{
                el.dispatchEvent(new Event({json.dumps(event)}, {{bubbles: true}}));
            }}
        }})
        """
        await self.runtime.eval_async(js)

    def query(self, selector: str) -> str:
        """Return innerHTML of the first matching element."""
        return self.runtime.eval(f"document.querySelector({json.dumps(selector)}).innerHTML")

    def close(self) -> None:
        self.runtime.close()

    def __enter__(self) -> Browser:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()
