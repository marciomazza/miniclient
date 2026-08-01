import subprocess
import sys
from collections.abc import Iterator
from pathlib import Path
from textwrap import dedent

import pytest
from conftest import HTMX_SCRIPT_TAG, HTMX_VIRTUAL_SERVER
from pytest_httpx2 import HTTPXMock

from miniclient.page import Element, FormElement, Page

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def page(v8_snapshot: bytes) -> Iterator[Page]:
    """A fresh Page, closed automatically unless the test closes it first."""
    b = Page(v8_snapshot=v8_snapshot)
    try:
        yield b
    finally:
        b.close()


@pytest.fixture
def htmx_page(v8_snapshot: bytes) -> Iterator[Page]:
    """A fresh Page that can reach the vendored htmx.js, closed automatically unless
    the test closes it first. Each load()/goto() is a real navigation, so htmx must be
    (re-)loaded per page via HTMX_SCRIPT_TAG, same as a real page."""
    b = Page(
        v8_snapshot=v8_snapshot,
        mounts={HTMX_VIRTUAL_SERVER["url"]: Path(HTMX_VIRTUAL_SERVER["directory"])},
    )
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# Bridge relays calls/results correctly.
#
# JS-generation correctness (query selectors, attribute handling, htmx
# settle logic, ...) is already covered exhaustively by test_page_async.py /
# test_htmx_integration.py against AsyncPage. These only check that going
# through the sync facade's background-thread bridge doesn't change the
# result — so one broad test per bridge "shape" (plain queries/mutations,
# htmx interactions) rather than one test per method.
# ---------------------------------------------------------------------------


def test_goto_and_queries(page: Page, httpx_mock: HTTPXMock) -> None:
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
    page.goto("http://localhost/page")
    assert page.eval("document.title") == "T"

    el = page.find("#s")
    assert isinstance(el, Element)
    assert el.text == "ok"
    assert el.attr("data-x") == "1"
    assert el.attr("missing") is None
    # parent is <body>, containing the other elements
    assert el.parent and el.parent.find("#inp") is not None

    inp = page.find("#inp")
    assert inp is not None
    inp.fill("new")
    assert page.eval("document.querySelector('#inp').value") == "new"

    ul = page.find("ul")
    assert ul is not None
    assert [i.text for i in ul.find_all("li")] == ["a", "b"]
    assert ul.find("li") is not None

    assert [i.text for i in page.find_all("li")] == ["a", "b"]
    assert page.find("#does-not-exist") is None


def test_click_and_form_submit_via_htmx(htmx_page: Page, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(url="http://localhost/click-target", text="<b>clicked</b>")
    httpx_mock.add_response(url="http://localhost/form-action", text="<p>submitted</p>")
    htmx_page.load(
        f"""\
        {HTMX_SCRIPT_TAG}
        <div id="out">
        <button hx-get="/click-target" hx-target="#out" hx-swap="innerHTML">click</button>
        </div>
        <form id="f" hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">
        <input name="x" value="1">
        <button type="submit">send</button>
        </form>
        <div id="result"></div>
    """
    )
    btn = htmx_page.find("button")
    assert btn is not None
    btn.click()
    out = htmx_page.find("#out")
    assert out and out.innerHTML == "<b>clicked</b>"

    form = htmx_page.find("form")
    assert isinstance(form, FormElement)
    form.requestSubmit()
    result = htmx_page.find("#result")
    assert result and result.innerHTML == "<p>submitted</p>"


# ---------------------------------------------------------------------------
# Sync-specific behavior — no AsyncPage equivalent to mirror.
# ---------------------------------------------------------------------------


def test_page_context_manager_closes(v8_snapshot: bytes) -> None:
    with Page(v8_snapshot=v8_snapshot) as b:
        b.load("<p>hi</p>")
        assert b.find("p") is not None
    assert b._closed


def test_page_virtual_servers(v8_snapshot: bytes, tmp_path: Path) -> None:
    (tmp_path / "external-script.js").write_text("window.__ran = 1;")
    with Page(v8_snapshot=v8_snapshot, mounts={"http://localhost/ext/": tmp_path}) as b:
        b.eval(
            """document.head.innerHTML = '<script src="http://localhost/ext/external-script.js"></script>'"""
        )
        assert b.eval("window.__ran") == 1


# ---------------------------------------------------------------------------
# Page GC / cross-thread lifecycle (regression)
#
# jsrun's Runtime panics (Rust-level) if it is ever garbage-collected on a
# thread other than the one that created it. Page.close() defuses this,
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
            # Element/FormElement kept alive past Page.close() must not
            # crash when it's eventually garbage collected, even at
            # interpreter shutdown.
            "held_element.py",
            dedent("""\
                from miniclient.page import Page
                with Page() as b:
                    b.load("<button id='x'>hi</button>")
                    el = b.find('#x')
                    assert el is not None
                print('done')
            """),
        ),
        (
            # A Page that is never explicitly closed must still clean up
            # silently via __del__ when it's dropped or the process exits.
            "never_closed.py",
            dedent("""\
                from miniclient.page import Page
                b = Page()
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


async def test_page_rejects_running_event_loop() -> None:
    # Page drives its own event loop on the caller's thread; it can't nest
    # inside one that's already running (e.g. this test coroutine's own loop).
    with pytest.raises(RuntimeError, match="running event loop"):
        Page()
