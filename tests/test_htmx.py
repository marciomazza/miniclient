import asyncio
from collections.abc import Generator
from pathlib import Path

import pytest
from jsrun import Runtime

from htmxclient.browser import build_browser

_ROOT = Path(__file__).parent.parent
_VENDOR_TEST = _ROOT / "vendor/htmx/test"
_CHAI_JS = _ROOT / "node_modules/chai/chai.js"
_RUNNER_JS = Path(__file__).parent / "runner.js"
_FETCH_MOCK_JS = _VENDOR_TEST / "lib/fetch-mock.js"
_HELPERS_JS = _VENDOR_TEST / "lib/helpers.js"
_UNIT_TEST_DIR = _VENDOR_TEST / "tests/unit"
_ATTR_TEST_DIR = _VENDOR_TEST / "tests/attributes"
_E2E_TEST_DIR = _VENDOR_TEST / "tests/end2end"

# ---------------------------------------------------------------------------
# htmx vendor unit tests — one pytest case per JS file in tests/unit/
# ---------------------------------------------------------------------------

_SKIP = {
    "package.js",  # asserts htmx has no dependencies — not relevant to this runtime
}
_unit_files = [f for f in sorted(_UNIT_TEST_DIR.glob("*.js")) if f.name not in _SKIP]
_attr_files = sorted(_ATTR_TEST_DIR.glob("*.js"))
_e2e_files = sorted(_E2E_TEST_DIR.glob("*.js"))
_RUNNER_JS_TEXT = _RUNNER_JS.read_text()
_CHAI_SETUP_JS = (
    _CHAI_JS.read_text()
    + "\nglobalThis.assert = window.chai.assert;"
    + "\nglobalThis.should = window.chai.should();"
)
_INFRA_JS = "\n".join(
    [
        _FETCH_MOCK_JS.read_text(),
        "document.body.innerHTML = '<div id=\"test-playground\"></div>';",
        _HELPERS_JS.read_text(),
    ]
)


@pytest.fixture(scope="module")
def htmx_unit_runtime() -> Generator[Runtime, None, None]:
    """Single browser runtime shared across all unit tests in this module."""

    async def _build() -> Runtime:
        r = await build_browser("http://localhost/")
        r.eval(_CHAI_SETUP_JS)
        r.eval(_INFRA_JS)
        return r

    r = asyncio.run(_build())
    yield r
    r.close()


async def _run_js_tests(r: Runtime, js_file: Path) -> None:
    r.eval(_RUNNER_JS_TEXT)  # resets _suites for this file
    r.eval(js_file.read_text())
    results = await r.eval_async("__runAllTests()")
    failures = [res for res in results if not res["passed"]]
    if failures:
        lines = [f"  [{res['suite']}] {res['name']}: {res['error']}" for res in failures]
        pytest.fail(f"{len(failures)} JS test(s) failed in {js_file.name}:\n" + "\n".join(lines))


@pytest.mark.parametrize("js_file", _unit_files, ids=lambda f: f.stem)
async def test_htmx_unit(js_file: Path, htmx_unit_runtime: Runtime) -> None:
    await _run_js_tests(htmx_unit_runtime, js_file)


@pytest.mark.parametrize("js_file", _attr_files, ids=lambda f: f.stem)
async def test_htmx_attr(js_file: Path, htmx_unit_runtime: Runtime) -> None:
    await _run_js_tests(htmx_unit_runtime, js_file)


@pytest.mark.parametrize("js_file", _e2e_files, ids=lambda f: f.stem)
async def test_htmx_e2e(js_file: Path, htmx_unit_runtime: Runtime) -> None:
    await _run_js_tests(htmx_unit_runtime, js_file)
