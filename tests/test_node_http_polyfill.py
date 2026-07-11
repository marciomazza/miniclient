"""Tests for the `node:http`/`node:https` polyfill that backs happy-dom's real
(non-virtual-server) resource fetching: `Fetch` (async), used by async/module/defer
`<script src>`, `<link>`, and async `XMLHttpRequest`.

All requests target the runtime's own origin (http://localhost/, see conftest.py's
`runtime` fixture) rather than a cross-origin URL: XHR and `type="module"` scripts
fetch in CORS mode by spec and would otherwise be blocked by happy-dom's Same-Origin
Policy check — a real, correct check living above our patch boundary, not something
this suite is testing.

Classic blocking `<script src>` and sync XHR use `SyncFetch` (`node:child_process`),
which is a separate, not-yet-implemented piece — see zz/fetch-src-polyfill-plan.md Part B.

`<img src>` real-network loading is out of scope: it's gated behind happy-dom's
`enableImageFileLoading` setting, which defaults to off and this project doesn't
enable — a separate, pre-existing opt-in, not part of this patch.
"""

import pytest

# ---------------------------------------------------------------------------
# <script async|defer|type=module src> executes and runs real network fetch
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "attr_setup_js",
    ["script.async = true;", "script.defer = true;", "script.type = 'module';"],
    ids=["async", "defer", "module"],
)
async def test_script_src_real_network_executes(runtime, httpx_mock, attr_setup_js):
    httpx_mock.add_response(url="http://localhost/x.js", text="window.__ran = 1;")
    result = await runtime.eval_async(f"""
        new Promise((resolve, reject) => {{
          const script = document.createElement('script');
          {attr_setup_js};
          script.src = 'http://localhost/x.js';
          script.onload = () => resolve(window.__ran);
          script.onerror = () => reject(new Error('script failed to load'));
          document.head.appendChild(script);
        }});
    """)
    assert result == 1


async def test_script_src_non_ok_status_fires_error_not_exception(runtime, httpx_mock):
    httpx_mock.add_response(url="http://localhost/missing.js", status_code=404)
    result = await runtime.eval_async("""
        new Promise(resolve => {
          const script = document.createElement('script');
          script.async = true;
          script.src = 'http://localhost/missing.js';
          script.onload = () => resolve('loaded');
          script.onerror = () => resolve('errored');
          document.head.appendChild(script);
        });
    """)
    assert result == "errored"


async def test_script_src_follows_redirect(runtime, httpx_mock):
    httpx_mock.add_response(
        url="http://localhost/old.js",
        status_code=302,
        headers=[("location", "http://localhost/new.js")],
    )
    httpx_mock.add_response(url="http://localhost/new.js", text="window.__ran = 'redirected';")
    result = await runtime.eval_async("""
        new Promise((resolve, reject) => {
          const script = document.createElement('script');
          script.async = true;
          script.src = 'http://localhost/old.js';
          script.onload = () => resolve(window.__ran);
          script.onerror = () => reject(new Error('script failed to load'));
          document.head.appendChild(script);
        });
    """)
    assert result == "redirected"


@pytest.mark.xfail(reason="classic blocking <script src> needs SyncFetch/Part B", strict=True)
async def test_classic_script_src_real_network_executes(runtime, httpx_mock):
    httpx_mock.add_response(url="http://localhost/classic.js", text="window.__ran = 1;")
    result = await runtime.eval_async("""
        new Promise((resolve, reject) => {
          const script = document.createElement('script');
          script.src = 'http://localhost/classic.js';
          script.onload = () => resolve(window.__ran);
          script.onerror = () => reject(new Error('script failed to load'));
          document.head.appendChild(script);
        });
    """)
    assert result == 1


# ---------------------------------------------------------------------------
# <link rel=stylesheet> smoke test (same Fetch path, different consumer)
# ---------------------------------------------------------------------------


async def test_link_stylesheet_real_network_loads(runtime, httpx_mock):
    httpx_mock.add_response(url="http://localhost/x.css", text="body { color: red; }")
    result = await runtime.eval_async("""
        new Promise((resolve, reject) => {
          const link = document.createElement('link');
          link.rel = 'stylesheet';
          link.href = 'http://localhost/x.css';
          link.onload = () => resolve('loaded');
          link.onerror = () => reject(new Error('link failed to load'));
          document.head.appendChild(link);
        });
    """)
    assert result == "loaded"


# ---------------------------------------------------------------------------
# Async XMLHttpRequest — direct access to status/headers/body via the same Fetch path
# ---------------------------------------------------------------------------


async def test_async_xhr_status_and_body(runtime, httpx_mock):
    httpx_mock.add_response(url="http://localhost/data", text="hello", status_code=201)
    result = await runtime.eval_async("""
        new Promise((resolve, reject) => {
          const xhr = new XMLHttpRequest();
          xhr.open('GET', 'http://localhost/data', true);
          xhr.onload = () => resolve({status: xhr.status, body: xhr.responseText});
          xhr.onerror = () => reject(new Error('xhr failed'));
          xhr.send();
        });
    """)
    assert result == {"status": 201, "body": "hello"}


async def test_async_xhr_response_headers_including_duplicates(runtime, httpx_mock):
    # A header repeated in the raw response must survive end-to-end: happy-dom's
    # Headers.append() collects repeats and getResponseHeader() joins them with ", "
    # (per the Fetch spec) — if our rawHeaders plumbing silently collapsed/dropped a
    # duplicate (e.g. by going through a Python dict keyed by header name), only one
    # value would show up here instead of both.
    httpx_mock.add_response(
        url="http://localhost/data",
        text="ok",
        headers=[("x-custom", "value1"), ("x-tag", "a=1"), ("x-tag", "a=2")],
    )
    result = await runtime.eval_async("""
        new Promise((resolve, reject) => {
          const xhr = new XMLHttpRequest();
          xhr.open('GET', 'http://localhost/data', true);
          xhr.onload = () =>
            resolve({
              custom: xhr.getResponseHeader('x-custom'),
              tag: xhr.getResponseHeader('x-tag'),
            });
          xhr.onerror = () => reject(new Error('xhr failed'));
          xhr.send();
        });
    """)
    assert result["custom"] == "value1"
    assert result["tag"] == "a=1, a=2"


async def test_async_xhr_sends_request_body(runtime, httpx_mock):
    httpx_mock.add_response(url="http://localhost/data", text="ok")
    await runtime.eval_async("""
        new Promise((resolve, reject) => {
          const xhr = new XMLHttpRequest();
          xhr.open('POST', 'http://localhost/data', true);
          xhr.onload = () => resolve();
          xhr.onerror = () => reject(new Error('xhr failed'));
          xhr.send('hello=world');
        });
    """)
    request = httpx_mock.get_request(url="http://localhost/data")
    assert request is not None
    assert request.content == b"hello=world"


async def test_async_xhr_network_error_fires_onerror(runtime, httpx_mock):
    httpx_mock.add_exception(ConnectionError("boom"), url="http://localhost/down")
    result = await runtime.eval_async("""
        new Promise(resolve => {
          const xhr = new XMLHttpRequest();
          xhr.open('GET', 'http://localhost/down', true);
          xhr.onload = () => resolve('loaded');
          xhr.onerror = () => resolve('errored');
          xhr.send();
        });
    """)
    assert result == "errored"
