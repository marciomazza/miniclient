from collections.abc import Iterator
from pathlib import Path

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
# Browser.goto / Browser.load
# ---------------------------------------------------------------------------


def test_goto_head_and_title_body(browser: Browser, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(
        url="http://localhost/page",
        text="<html><head><title>T</title></head><body><span id='s'>ok</span></body></html>",
    )
    browser.goto("http://localhost/page")
    assert browser.eval("document.title") == "T"
    el = browser.find("#s")
    assert el and el.innerHTML() == "ok"


def test_load_sets_body(browser: Browser) -> None:
    browser.load("<p id='msg'>hello</p>")
    el = browser.find("body")
    assert el and el.innerHTML() == '<p id="msg">hello</p>'


# ---------------------------------------------------------------------------
# Browser.find / Element queries
# ---------------------------------------------------------------------------


def test_find_returns_element(browser: Browser) -> None:
    browser.load("<p id='msg'>hello</p>")
    el = browser.find("#msg")
    assert isinstance(el, Element) and el.innerHTML() == "hello"


def test_find_returns_none_for_missing(browser: Browser) -> None:
    browser.load("<p>hi</p>")
    assert browser.find("#does-not-exist") is None


def test_find_all_returns_elements(browser: Browser) -> None:
    browser.load("<ul><li>a</li><li>b</li></ul>")
    items = browser.find_all("li")
    assert [i.text() for i in items] == ["a", "b"]


def test_element_text_and_attr(browser: Browser) -> None:
    browser.load("<a id='link' href='/path'>hi there</a>")
    el = browser.find("#link")
    assert el and el.text() == "hi there"
    assert el.attr("href") == "/path"
    assert el.attr("missing") is None


def test_element_fill(browser: Browser) -> None:
    browser.load("<input id='inp' value='old'>")
    el = browser.find("#inp")
    assert el is not None
    el.fill("new")
    assert browser.eval("document.querySelector('#inp').value") == "new"


# ---------------------------------------------------------------------------
# Element.click (htmx)
# ---------------------------------------------------------------------------


def test_element_click_hx_get(htmx_browser: Browser, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(url="http://localhost/click-target", text="<b>clicked</b>")
    htmx_browser.load("""\
        <div id="out">
        <button hx-get="/click-target" hx-target="#out" hx-swap="innerHTML">click</button>
        </div>
    """)
    btn = htmx_browser.find("button")
    assert btn is not None
    btn.click()
    el = htmx_browser.find("#out")
    assert el and el.innerHTML() == "<b>clicked</b>"


# ---------------------------------------------------------------------------
# FormElement.requestSubmit
# ---------------------------------------------------------------------------


def test_form_request_submit(htmx_browser: Browser, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(url="http://localhost/form-action", text="<p>submitted</p>")
    htmx_browser.load("""\
        <form id="f" hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">
        <input name="x" value="1">
        <button type="submit">send</button>
        </form>
        <div id="result"></div>
    """)
    form = htmx_browser.find("form")
    assert isinstance(form, FormElement)
    form.requestSubmit()
    result = htmx_browser.find("#result")
    assert result and result.innerHTML() == "<p>submitted</p>"


# ---------------------------------------------------------------------------
# Browser lifecycle
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
            """document.head.innerHTML =
                '<script src="http://localhost/ext/external-script.js"></script>'"""
        )
        assert b.eval("window.__ran") == 1
