from pathlib import Path

import pytest

from htmxclient.browser import build_browser

_ROOT = Path(__file__).parent.parent
_VENDOR_TEST = _ROOT / "vendor/htmx/test"
_CHAI_JS = _ROOT / "node_modules/chai/chai.js"
_RUNNER_JS = Path(__file__).parent / "runner.js"
_FETCH_MOCK_JS = _VENDOR_TEST / "lib/fetch-mock.js"
_HELPERS_JS = _VENDOR_TEST / "lib/helpers.js"
_UNIT_TEST_DIR = _VENDOR_TEST / "tests/unit"

# ---------------------------------------------------------------------------
# htmx vendor unit tests — one pytest case per JS file in tests/unit/
# ---------------------------------------------------------------------------


_INFRASTRUCTURE_JS = "\n".join(
    [
        _RUNNER_JS.read_text(),
        _FETCH_MOCK_JS.read_text(),
        "document.body.innerHTML = '<div id=\"test-playground\"></div>';",
        _HELPERS_JS.read_text(),
    ]
)


_unit_files = sorted(_UNIT_TEST_DIR.glob("*.js"))


@pytest.mark.parametrize("js_file", _unit_files, ids=lambda f: f.stem)
async def test_htmx_unit(js_file: Path) -> None:
    r = await build_browser("http://localhost/")
    try:
        r.eval(_JS_SETUP)
        r.eval(_INFRASTRUCTURE_JS)
        r.eval(js_file.read_text())
        results = await r.eval_async("__runAllTests()")
        failures = [res for res in results if not res["passed"]]
        if failures:
            lines = [f"  [{res['suite']}] {res['name']}: {res['error']}" for res in failures]
            pytest.fail(
                f"{len(failures)} JS test(s) failed in {js_file.name}:\n" + "\n".join(lines)
            )
    finally:
        r.close()
