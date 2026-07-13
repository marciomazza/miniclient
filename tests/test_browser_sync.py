import subprocess
import sys
from collections.abc import Iterator
from pathlib import Path
from textwrap import dedent

import pytest
from conftest import HTMX_BASE_HTML, HTMX_VIRTUAL_SERVER
from pytest_httpx2 import HTTPXMock

from miniclient.browser import Browser, Element, FormElement

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def browser(snapshot: bytes) -> Iterator[Browser]:
    """A fresh Browser, closed automatically unless the test closes it first."""
    b = Browser(snapshot=snapshot)
    try:
        yield b
    finally:
        b.close()


@pytest.fixture
def htmx_browser(snapshot: bytes) -> Iterator[Browser]:
    """A fresh htmx-loaded Browser, closed automatically unless the test closes it first."""
    b = Browser(
        snapshot=snapshot,
        mounts={HTMX_VIRTUAL_SERVER["url"]: Path(HTMX_VIRTUAL_SERVER["directory"])},
    )
    b.load(HTMX_BASE_HTML)
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# Bridge relays calls/results correctly.
#
# JS-generation correctness (query selectors, attribute handling, htmx
# settle logic, ...) is already covered exhaustively by test_browser.py /
# test_htmx_integration.py against AsyncBrowser. These only check that going
# through the sync facade's background-thread bridge doesn't change the
# result — so one broad test per bridge "shape" (plain queries/mutations,
# htmx interactions) rather than one test per method.
# ---------------------------------------------------------------------------


def test_goto_and_queries(browser: Browser, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(
        url="http://localhost/page",
        text="""\
            <html><head><title>T</title></head><body>
            <span id='s' data-x='1'>ok</span>
            <input id='inp' value='old'>
            <ul><li>a</li><li>b</li></ul>
            </body></html>
        """,
    )
    browser.goto("http://localhost/page")
    assert browser.eval("document.title") == "T"

    el = browser.find("#s")
    assert isinstance(el, Element)
    assert el.text() == "ok"
    assert el.attr("data-x") == "1"
    assert el.attr("missing") is None

    inp = browser.find("#inp")
    assert inp is not None
    inp.fill("new")
    assert browser.eval("document.querySelector('#inp').value") == "new"

    assert [i.text() for i in browser.find_all("li")] == ["a", "b"]
    assert browser.find("#does-not-exist") is None


def test_click_and_form_submit_via_htmx(htmx_browser: Browser, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(url="http://localhost/click-target", text="<b>clicked</b>")
    httpx_mock.add_response(url="http://localhost/form-action", text="<p>submitted</p>")
    htmx_browser.load("""\
        <div id="out">
        <button hx-get="/click-target" hx-target="#out" hx-swap="innerHTML">click</button>
        </div>
        <form id="f" hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">
        <input name="x" value="1">
        <button type="submit">send</button>
        </form>
        <div id="result"></div>
    """)
    btn = htmx_browser.find("button")
    assert btn is not None
    btn.click()
    out = htmx_browser.find("#out")
    assert out and out.innerHTML() == "<b>clicked</b>"

    form = htmx_browser.find("form")
    assert isinstance(form, FormElement)
    form.requestSubmit()
    result = htmx_browser.find("#result")
    assert result and result.innerHTML() == "<p>submitted</p>"


# ---------------------------------------------------------------------------
# Sync-specific behavior — no AsyncBrowser equivalent to mirror.
# ---------------------------------------------------------------------------


def test_browser_context_manager_closes(snapshot: bytes) -> None:
    with Browser(snapshot=snapshot) as b:
        b.load("<p>hi</p>")
        assert b.find("p") is not None
    assert b._closed


def test_browser_virtual_servers(snapshot: bytes, tmp_path: Path) -> None:
    (tmp_path / "external-script.js").write_text("window.__ran = 1;")
    with Browser(snapshot=snapshot, mounts={"http://localhost/ext/": tmp_path}) as b:
        b.eval(
            """document.head.innerHTML = '<script src="http://localhost/ext/external-script.js"></script>'"""
        )
        assert b.eval("window.__ran") == 1


# ---------------------------------------------------------------------------
# Browser GC / cross-thread lifecycle (regression)
#
# jsrun's Runtime panics (Rust-level) if it is ever garbage-collected on a
# thread other than the one that created it. Browser.close() defuses this,
# but the defusing only works if it actually runs and actually reaches every
# live reference — these are process-level regression tests (the panic is a
# GC/interpreter-shutdown phenomenon, not observable reliably as a plain
# exception inside the pytest process itself), run in a subprocess so a
# regression can't corrupt the test runner.
# ---------------------------------------------------------------------------


def _run_script(tmp_path: Path, name: str, body: str) -> subprocess.CompletedProcess[str]:
    script = tmp_path / name
    script.write_text(body)
    return subprocess.run([sys.executable, str(script)], capture_output=True, text=True, timeout=30)


@pytest.mark.parametrize(
    "name,body",
    [
        (
            # Element/FormElement kept alive past Browser.close() must not
            # crash when it's eventually garbage collected, even at
            # interpreter shutdown.
            "held_element.py",
            dedent("""\
                from miniclient.browser import Browser
                with Browser('http://localhost/') as b:
                    b.load("<button id='x'>hi</button>")
                    el = b.find('#x')
                    assert el is not None
                print('done')
            """),
        ),
        (
            # A Browser that is never explicitly closed must still clean up
            # silently via __del__ when it's dropped or the process exits.
            "never_closed.py",
            dedent("""\
                from miniclient.browser import Browser
                b = Browser('http://localhost/')
                b.load("<button id='x'>hi</button>")
                el = b.find('#x')
                el.click()
                b = None
                el = None
                print('done')
            """),
        ),
    ],
)
def test_gc_panic_regression(tmp_path: Path, name: str, body: str) -> None:
    result = _run_script(tmp_path, name, body)
    assert result.returncode == 0
    assert result.stdout.strip() == "done"
    assert result.stderr == ""


def test_close_clears_runtime_on_held_elements(snapshot: bytes) -> None:
    # prevents runtime panic when el is garbage collected in another thread
    b = Browser(snapshot=snapshot)
    b.load("<p id='p'>hi</p>")
    el = b.find("#p")
    assert el is not None
    b.close()
    assert el.runtime is None
