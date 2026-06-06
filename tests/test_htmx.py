from collections.abc import AsyncGenerator
from functools import cache
from pathlib import Path

import pytest
from jsrun import Runtime, SnapshotBuilder

from htmxclient.browser import _populate_builder, build_browser

_ROOT = Path(__file__).parent.parent
_HTMX_TEST = _ROOT / "vendor/htmx/test"
_CHAI_JS = _ROOT / "node_modules/chai/chai.js"
_RUNNER_JS = Path(__file__).parent / "runner.js"
_FETCH_MOCK_JS = _HTMX_TEST / "lib/fetch-mock.js"
_HELPERS_JS = _HTMX_TEST / "lib/helpers.js"

# ---------------------------------------------------------------------------
# htmx vendor unit tests — one pytest case per JS file in tests/unit/
# ---------------------------------------------------------------------------

_SKIP = {
    "package.js",  # asserts htmx has no dependencies — not relevant to this runtime
}
# Individual JS tests to skip, keyed by file stem → set of (suite, test-name).
# Use for tests that are inherently untestable in a headless environment.
_SKIP_TESTS: dict[str, set[tuple[str, str]]] = {
    "hx-swap": {
        ("hx-swap modifiers", "swap with scroll:bottom modifier scrolls to bottom"),
        # scroll position is always 0 in a headless DOM
    },
}
_INFRA_JS = "\n".join(
    [
        "document.body.innerHTML = '<div id=\"test-playground\"></div>';",
        _HELPERS_JS.read_text(),
    ]
)


@cache
def _build_test_snapshot() -> bytes:
    """Snapshot with production scripts + chai, fetch-mock, and runner pre-loaded."""
    builder = SnapshotBuilder()
    _populate_builder(builder)
    builder.execute_script(
        "chai",
        _CHAI_JS.read_text()
        + "\nglobalThis.assert = globalThis.chai.assert;"
        + "\nglobalThis.should = globalThis.chai.should();",
    )
    builder.execute_script("fetch-mock", _FETCH_MOCK_JS.read_text())
    builder.execute_script("runner", _RUNNER_JS.read_text())
    return builder.build()


@pytest.fixture
async def htmx_unit_runtime() -> AsyncGenerator[Runtime, None]:
    """Isolated browser runtime per test — prevents state leakage between JS files."""
    r = await build_browser("http://localhost/", snapshot=_build_test_snapshot())
    r.eval(_INFRA_JS)
    yield r
    r.close()


async def _run_js_tests(r: Runtime, js_file: Path) -> None:
    r.eval(js_file.read_text())
    results = await r.eval_async("__runAllTests()")
    skip = _SKIP_TESTS.get(js_file.stem, set())
    failures = [
        res for res in results if not res["passed"] and (res["suite"], res["name"]) not in skip
    ]
    if failures:
        lines = [f"  [{res['suite']}] {res['name']}: {res['error']}" for res in failures]
        pytest.fail(f"{len(failures)} JS test(s) failed in {js_file.name}:\n" + "\n".join(lines))


_unit_files = [f for f in sorted((_HTMX_TEST / "tests/unit").glob("*.js")) if f.name not in _SKIP]
_attributes_files = sorted((_HTMX_TEST / "tests/attributes").glob("*.js"))
_end2end_files = sorted((_HTMX_TEST / "tests/end2end").glob("*.js"))


@pytest.mark.parametrize("js_file", _unit_files, ids=lambda f: f.stem)
async def test_htmx_unit(js_file: Path, htmx_unit_runtime: Runtime) -> None:
    await _run_js_tests(htmx_unit_runtime, js_file)


@pytest.mark.parametrize("js_file", _attributes_files, ids=lambda f: f.stem)
async def test_htmx_attributes(js_file: Path, htmx_unit_runtime: Runtime) -> None:
    await _run_js_tests(htmx_unit_runtime, js_file)


@pytest.mark.parametrize("js_file", _end2end_files, ids=lambda f: f.stem)
async def test_htmx_e2e(js_file: Path, htmx_unit_runtime: Runtime) -> None:
    await _run_js_tests(htmx_unit_runtime, js_file)
