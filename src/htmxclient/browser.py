from __future__ import annotations

from pathlib import Path
from urllib.parse import urljoin

from jsrun import Runtime

_ROOT = Path(__file__).parent.parent.parent
_NM = _ROOT / "node_modules"
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


async def build_browser() -> Runtime:
    r = Runtime()
    r.eval((_JS / "pre_globals.js").read_text())
    r.set_module_resolver(_resolver)
    r.set_module_loader(_loader)
    r.add_static_module("bootstrap", (_JS / "bootstrap.js").read_text())
    await r.eval_module_async("bootstrap")
    return r
