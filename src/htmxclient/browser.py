from __future__ import annotations

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
        filename = _NODE_POLYFILL_FILES[spec.removeprefix("node:")]
        return (_POLYFILLS / filename).read_text()
    if spec.startswith("npm:"):
        filename = _NPM_POLYFILL_FILES[spec.removeprefix("npm:")]
        return (_POLYFILLS / filename).read_text()
    if spec.startswith("file://"):
        return Path(spec[7:]).read_text()
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
    r.eval((_JS / "pre_globals.js").read_text())
    r.set_module_resolver(_resolver)
    r.set_module_loader(_loader)
    op_id = r.register_op("fetch", _fetch_op, mode="async")
    r.eval(f"globalThis.__FETCH_OP_ID__ = {op_id};")
    r.eval(f"globalThis.__BASE_URL__ = {json.dumps(url)};")
    r.add_static_module("bootstrap", (_JS / "bootstrap.js").read_text())
    await r.eval_module_async("bootstrap")
    r.eval(_HTMX_SRC.read_text())
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
