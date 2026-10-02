"""Tests for real (non-mocked-transport) resource fetching under jsdom: `<script src>` and
`<link>`, routed through jsdom's `resources`/`requestInterceptor` option to the runtime's own
`httpx`-backed `fetch()` (see ticket 19 of `.scratch/jsdom-port/`).

All requests target the runtime's own origin (http://localhost/) rather than a cross-origin
URL: `<script>` fetches run in CORS mode by spec
and would otherwise be blocked by jsdom's Same-Origin Policy check — a real, correct check
living above our patch boundary, not something this suite is testing.

`<img src>` real-network loading is out of scope: it's gated behind jsdom's `resources`
option (default: no subresource loading), which this project doesn't enable for images --
a separate, pre-existing opt-in, not part of this patch.
"""

from collections.abc import AsyncIterator

import pytest
import pytest_asyncio

from miniclient.page import AsyncPage


@pytest_asyncio.fixture
async def async_page() -> AsyncIterator[AsyncPage]:
    async with AsyncPage() as page:
        yield page


# ---------------------------------------------------------------------------
# <script async|defer|type=module src> executes and runs real network fetch
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "attr_setup_js",
    ["script.async = true;", "script.defer = true;", "script.type = 'module';"],
    ids=["async", "defer", "module"],
)
async def test_script_src_real_network_executes(async_page, httpx_mock, attr_setup_js):
    httpx_mock.add_response(url="http://localhost/x.js", text="window.__ran = 1;")
    result = await async_page.eval(f"""
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


@pytest.mark.xfail(reason="no real ESM loader: bare imports are only a SyntaxError, not resolved")
async def test_module_script_relative_import_real_network(async_page, httpx_mock):
    httpx_mock.add_response(
        url="http://localhost/entry.js",
        text="import { value } from './helper.js'; window.__ran = value;",
    )
    httpx_mock.add_response(url="http://localhost/helper.js", text="export const value = 1;")
    result = await async_page.eval("""
        new Promise((resolve, reject) => {
          const script = document.createElement('script');
          script.type = 'module';
          script.src = 'http://localhost/entry.js';
          script.onload = () => resolve(window.__ran);
          script.onerror = () => reject(new Error('script failed to load'));
          document.head.appendChild(script);
        });
    """)
    assert result == 1


async def test_script_src_non_ok_status_fires_error_not_exception(async_page, httpx_mock):
    httpx_mock.add_response(url="http://localhost/missing.js", status_code=404)
    result = await async_page.eval("""
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


async def test_script_src_follows_redirect(async_page, httpx_mock):
    httpx_mock.add_response(
        url="http://localhost/old.js",
        status_code=302,
        headers=[("location", "http://localhost/new.js")],
    )
    httpx_mock.add_response(url="http://localhost/new.js", text="window.__ran = 'redirected';")
    result = await async_page.eval("""
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


async def test_classic_script_src_real_network_executes(async_page, httpx_mock):
    httpx_mock.add_response(url="http://localhost/classic.js", text="window.__ran = 1;")
    result = await async_page.eval("""
        new Promise((resolve, reject) => {
          const script = document.createElement('script');
          script.src = 'http://localhost/classic.js';
          script.onload = () => resolve(window.__ran);
          script.onerror = () => reject(new Error('script failed to load'));
          document.head.appendChild(script);
        });
    """)
    assert result == 1


async def test_classic_script_src_non_ok_status_fires_error_not_exception(async_page, httpx_mock):
    httpx_mock.add_response(url="http://localhost/missing.js", status_code=404)
    result = await async_page.eval("""
        new Promise(resolve => {
          const script = document.createElement('script');
          script.src = 'http://localhost/missing.js';
          script.onload = () => resolve('loaded');
          script.onerror = () => resolve('errored');
          document.head.appendChild(script);
        });
    """)
    assert result == "errored"


async def test_classic_script_src_follows_redirect(async_page, httpx_mock):
    httpx_mock.add_response(
        url="http://localhost/old.js",
        status_code=302,
        headers=[("location", "http://localhost/new.js")],
    )
    httpx_mock.add_response(url="http://localhost/new.js", text="window.__ran = 'redirected';")
    result = await async_page.eval("""
        new Promise((resolve, reject) => {
          const script = document.createElement('script');
          script.src = 'http://localhost/old.js';
          script.onload = () => resolve(window.__ran);
          script.onerror = () => reject(new Error('script failed to load'));
          document.head.appendChild(script);
        });
    """)
    assert result == "redirected"


# ---------------------------------------------------------------------------
# <link rel=stylesheet> smoke test (same Fetch path, different consumer)
# ---------------------------------------------------------------------------


async def test_link_stylesheet_real_network_loads(async_page, httpx_mock):
    httpx_mock.add_response(url="http://localhost/x.css", text="body { color: red; }")
    result = await async_page.eval("""
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


async def test_unmocked_stylesheet_fires_error_not_exception(async_page):
    result = await async_page.eval("""
        new Promise(resolve => {
          const link = document.createElement('link');
          link.rel = 'stylesheet';
          link.href = 'http://localhost/unmocked.css';
          link.onload = () => resolve('loaded');
          link.onerror = () => resolve('errored');
          document.head.appendChild(link);
        });
    """)
    assert result == "errored"
