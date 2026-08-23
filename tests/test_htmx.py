import json
from collections.abc import AsyncGenerator
from pathlib import Path

import pytest
from conftest import HTMX_BASE_HTML, HTMX_VIRTUAL_SERVER
from htmx_fetch_mock import HttpxFetchMock

from miniclient.runtime import Runtime, create_snapshot, get_snapshot_scripts, open_runtime

_ROOT = Path(__file__).parent.parent
_HTMX_TEST = _ROOT / "vendor/htmx/test"
_HELPERS_JS = _HTMX_TEST / "lib/helpers.js"
_CHAI_JS = _ROOT / "node_modules/chai/chai.js"
_RUNNER_JS = Path(__file__).parent / "runner.js"
_FETCH_MOCK_BRIDGE_JS = Path(__file__).parent / "htmx_fetch_mock_bridge.js"

# ---------------------------------------------------------------------------
# htmx vendor unit tests — one pytest case per JS file in tests/unit/
# ---------------------------------------------------------------------------

_SKIP = {
    "package.js",  # asserts htmx has no dependencies — not relevant to this runtime
}

# Individual JS tests to skip, keyed by file stem → set of (suite, test-name).
# Rewritten to it.skip(...) before the file runs — not just filtered from results —
# since some failures (e.g. an orphaned promise rejection) are fatal to the whole
# eval_async call and never produce a result to filter.
_SKIP_TESTS: dict[str, set[tuple[str, str]]] = {
    "hx-swap": {
        ("hx-swap modifiers", "swap with scroll:bottom modifier scrolls to bottom"),
        # scroll position is always 0 in a headless DOM
    },
}

# Tests that assert on real elapsed wall-clock time, or that use a waitForEvent()
# guard racing unscaled async work (e.g. a mocked fetch round trip) — exempted from
# timer scaling (run via runner.js's __unscaledTests check) rather than skipped
# outright, since unlike _SKIP_TESTS these are meaningful, just incompatible with a
# scaled clock.
_UNSCALED_TESTS: dict[str, set[tuple[str, str]]] = {
    "timeout": {
        ("timeout() unit tests", "returns promise that resolves after milliseconds"),
        ("timeout() unit tests", "accepts string time format"),
        ("timeout() unit tests", "accepts seconds format"),
    },
    "hx-swap": {
        ("hx-swap modifiers", "main swap with delay respects blocking behavior"),
    },
    "hx-live": {
        ("hx-live extension", "debounce(ms) supersedes prior calls"),
    },
    "hx-ws": {
        ("Deep Review Fixes", "cleans up expired pending requests on message receive"),
    },
    "morph": {
        ("htmx processing during morph", "processes new htmx attributes added during innerMorph"),
        ("htmx processing during morph", "processes new htmx attributes added during outerMorph"),
    },
}


@pytest.fixture(scope="session")
def htmx_v8_snapshot() -> bytes:
    scripts = [
        *get_snapshot_scripts(),
        (
            "chai",
            f"""{_CHAI_JS.read_text()}
            globalThis.assert = globalThis.chai.assert;
            globalThis.should = globalThis.chai.should();""",
        ),
        ("fetch-mock-bridge", _FETCH_MOCK_BRIDGE_JS.read_text()),
        ("runner", _RUNNER_JS.read_text()),
    ]
    return create_snapshot(scripts)


# todo: Perhaps make this a module scoped feature again after the tests pass.
@pytest.fixture
async def htmx_runtime(htmx_v8_snapshot: bytes) -> AsyncGenerator[Runtime, None]:
    fetch_mock = HttpxFetchMock()
    async with open_runtime(
        "http://localhost/",
        v8_snapshot=htmx_v8_snapshot,
        before_fetch=fetch_mock.before_fetch,
        httpx_transport=fetch_mock.transport,
        virtual_servers=[
            HTMX_VIRTUAL_SERVER,
            {"url": "http://localhost/test/", "directory": str(_HTMX_TEST)},
        ],
    ) as r:
        await r.eval_async(f"__document_write(`{HTMX_BASE_HTML}`)")
        # Bound after the navigation above (a real happy-dom navigation replaces
        # globalThis's contents), not before: anything bound earlier is wiped by it.
        fetch_mock.install(r)
        r.eval(_HELPERS_JS.read_text())
        yield r


@pytest.fixture(autouse=True)
async def _reset_htmx(htmx_runtime: Runtime) -> AsyncGenerator[None, None]:
    yield
    htmx_runtime.eval("""\
        __clearAllTimers();
        __resetRunner();
        cleanupTest();
    """)


async def _run_js_tests(r: Runtime, js_file: Path) -> None:
    js = js_file.read_text()
    # ext tests load extensions/libs via relative <script src>, resolved against the
    # real htmx repo layout (repo/src/ext, repo/test/lib) — rewrite to our virtual servers.
    js = js.replace("'../src/ext/", "'http://localhost/vendor/ext/")
    js = js.replace("'../test/lib/", "'http://localhost/test/lib/")
    for _suite, name in _SKIP_TESTS.get(js_file.stem, set()):
        js = js.replace(f"it('{name}'", f"it.skip('{name}'")
    unscaled = [f"{suite}::{name}" for suite, name in _UNSCALED_TESTS.get(js_file.stem, set())]
    r.eval(f"globalThis.__unscaledTests = new Set({json.dumps(unscaled)}); void 0")
    r.eval(js)
    results = await r.eval_async("__runAllTests()")
    failures = [res for res in results if not res["passed"]]
    if failures:
        lines = [f"  [{res['suite']}] {res['name']}: {res['error']}" for res in failures]
        pytest.fail(f"{len(failures)} JS test(s) failed in {js_file.name}:\n" + "\n".join(lines))


_unit_files = [f for f in sorted((_HTMX_TEST / "tests/unit").glob("*.js")) if f.name not in _SKIP]
_attributes_files = sorted((_HTMX_TEST / "tests/attributes").glob("*.js"))
_end2end_files = sorted((_HTMX_TEST / "tests/end2end").glob("*.js"))
_ext_files = [f for f in sorted((_HTMX_TEST / "tests/ext").glob("*.js")) if f.name not in _SKIP]


@pytest.mark.parametrize("js_file", _unit_files, ids=lambda f: f.stem)
async def test_htmx_unit(js_file: Path, htmx_runtime: Runtime) -> None:
    await _run_js_tests(htmx_runtime, js_file)


@pytest.mark.parametrize("js_file", _attributes_files, ids=lambda f: f.stem)
async def test_htmx_attributes(js_file: Path, htmx_runtime: Runtime) -> None:
    await _run_js_tests(htmx_runtime, js_file)


@pytest.mark.parametrize("js_file", _end2end_files, ids=lambda f: f.stem)
async def test_htmx_e2e(js_file: Path, htmx_runtime: Runtime) -> None:
    await _run_js_tests(htmx_runtime, js_file)


@pytest.mark.parametrize("js_file", _ext_files, ids=lambda f: f.stem)
async def test_htmx_ext(js_file: Path, htmx_runtime: Runtime) -> None:
    await _run_js_tests(htmx_runtime, js_file)
