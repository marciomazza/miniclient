from collections.abc import AsyncGenerator
from pathlib import Path

import pytest
from conftest import HTMX_BASE_HTML, HTMX_VIRTUAL_SERVER
from htmx_fetch_mock import HttpxFetchMock
from jsrun import Runtime

from miniclient.runtime import build_runtime

_ROOT = Path(__file__).parent.parent
_HTMX_TEST = _ROOT / "vendor/htmx/test"
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
        f"__document_write(`{HTMX_BASE_HTML}`);",
        _HELPERS_JS.read_text(),
    ]
)


@pytest.fixture(scope="module")
async def htmx_runtime(snapshot: bytes) -> AsyncGenerator[Runtime, None]:
    fetch_mock = HttpxFetchMock()
    r = await build_runtime(
        "http://localhost/",
        snapshot=snapshot,
        before_fetch=fetch_mock.before_fetch,
        httpx_transport=fetch_mock.transport,
        virtual_servers=[HTMX_VIRTUAL_SERVER],
    )
    fetch_mock.install(r)
    r.eval(_INFRA_JS)
    yield r
    r.close()


@pytest.fixture(autouse=True)
async def _reset_htmx(htmx_runtime: Runtime) -> AsyncGenerator[None, None]:
    yield
    htmx_runtime.eval("""\
        __clearAllTimers();
        __resetRunner();
        cleanupTest();
    """)


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
async def test_htmx_unit(js_file: Path, htmx_runtime: Runtime) -> None:
    await _run_js_tests(htmx_runtime, js_file)


@pytest.mark.parametrize("js_file", _attributes_files, ids=lambda f: f.stem)
async def test_htmx_attributes(js_file: Path, htmx_runtime: Runtime) -> None:
    await _run_js_tests(htmx_runtime, js_file)


@pytest.mark.parametrize("js_file", _end2end_files, ids=lambda f: f.stem)
async def test_htmx_e2e(js_file: Path, htmx_runtime: Runtime) -> None:
    await _run_js_tests(htmx_runtime, js_file)
