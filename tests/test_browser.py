from collections.abc import AsyncIterator, Callable
from pathlib import Path

import httpx2 as httpx
import pytest
import pytest_asyncio
from conftest import HTMX_BASE_HTML
from jsrun import JavaScriptError, Runtime
from pytest_httpx2 import HTTPXMock

from htmxclient.browser import Browser, Element

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest_asyncio.fixture
async def browser(runtime: Runtime) -> AsyncIterator[Browser]:
    """A fresh htmx-loaded Browser, closed automatically unless the test closes it first."""
    runtime.eval(f"__document_write(`{HTMX_BASE_HTML}`)")
    b = Browser(runtime)
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# Browser.goto
# ---------------------------------------------------------------------------


async def test_goto_head_and_title_body(browser: Browser, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(
        url="http://localhost/page",
        text="<html><head><title>T</title></head><body><span id='s'>ok</span></body></html>",
    )
    await browser.goto("http://localhost/page")
    assert browser.runtime.eval("document.title") == "T"
    el = browser.find("#s")
    assert el is not None
    assert el.innerHTML() == "ok"


# ---------------------------------------------------------------------------
# Browser.load
# ---------------------------------------------------------------------------


async def test_load_sets_body(browser: Browser) -> None:
    await browser.load("<p id='msg'>hello</p>")
    el = browser.find("body")
    assert el and el.innerHTML() == '<p id="msg">hello</p>'


async def test_load_replaces_body(browser: Browser) -> None:
    await browser.load("<span id='a'>first</span>")
    await browser.load("<span id='b'>second</span>")
    el = browser.find("#b")
    assert el is not None
    assert el.innerHTML() == "second"
    # first load is gone
    result = browser.runtime.eval("document.querySelector('#a')")
    assert result is None


# ---------------------------------------------------------------------------
# Browser.find / Element queries
# ---------------------------------------------------------------------------


async def test_find_returns_element(browser: Browser) -> None:
    await browser.load("<p id='msg'>hello</p>")
    el = browser.find("#msg")
    assert isinstance(el, Element)
    assert el.innerHTML() == "hello"


async def test_find_returns_none_for_missing(browser: Browser) -> None:
    await browser.load("<p>hi</p>")
    assert browser.find("#does-not-exist") is None


async def test_element_text(browser: Browser) -> None:
    await browser.load("<div id='d'><span>inner</span> text</div>")
    el = browser.find("#d")
    assert el is not None
    assert el.text() == "inner text"


async def test_element_attr(browser: Browser) -> None:
    await browser.load("<a id='link' href='/path' data-x='42'>link</a>")
    el = browser.find("#link")
    assert el is not None
    assert el.attr("href") == "/path"
    assert el.attr("data-x") == "42"
    assert el.attr("missing") is None


async def test_element_fill(browser: Browser) -> None:
    await browser.load("<input id='inp' value='old'>")
    el = browser.find("#inp")
    assert el is not None
    el.fill("new")
    assert browser.runtime.eval("document.querySelector('#inp').value") == "new"


async def test_element_fill_textarea(browser: Browser) -> None:
    await browser.load("<textarea id='ta'>old</textarea>")
    el = browser.find("#ta")
    assert el is not None
    el.fill("new")
    assert browser.runtime.eval("document.querySelector('#ta').value") == "new"


async def test_find_all_returns_elements(browser: Browser) -> None:
    await browser.load("<ul><li>a</li><li>b</li><li>c</li></ul>")
    items = browser.find_all("li")
    assert len(items) == 3
    assert items[0].text() == "a"
    assert items[1].text() == "b"
    assert items[2].text() == "c"


async def test_find_all_empty(browser: Browser) -> None:
    await browser.load("<div>no items</div>")
    assert browser.find_all("li") == []


# ---------------------------------------------------------------------------
# Element.submit
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("selector", ["#btn", "form"])
@pytest.mark.parametrize(
    "method, expected_url, check_request",
    [
        (
            "post",
            "http://localhost/action",
            lambda req: (
                req.method == "POST"
                and req.content == b"x=42"
                and req.headers["content-type"] == "application/x-www-form-urlencoded"
            ),
        ),
        (
            "get",
            "http://localhost/action?x=42",
            lambda req: req.method == "GET" and str(req.url) == "http://localhost/action?x=42",
        ),
    ],
    ids=["POST", "GET"],
)
async def test_element_submit_plain(
    browser: Browser,
    httpx_mock: HTTPXMock,
    selector: str,
    method: str,
    expected_url: str,
    check_request: Callable[[httpx.Request], bool],
) -> None:
    httpx_mock.add_response(url=expected_url, text="<body><p>done</p></body>")
    await browser.load(
        f'<form method="{method}" action="/action"><input name="x" value="42">'
        '<button type="submit" id="btn">go</button></form>'
    )
    el = browser.find(selector)
    assert el is not None
    await el.submit()
    request = httpx_mock.get_request()
    assert request is not None
    assert check_request(request)
    el = browser.find("p")
    assert el is not None
    assert el.text() == "done"


async def test_element_submit_no_form_raises(browser: Browser) -> None:
    await browser.load("<button id='btn'>orphan</button>")
    btn = browser.find("#btn")
    assert btn is not None
    with pytest.raises(JavaScriptError):
        await btn.submit()


# ---------------------------------------------------------------------------
# Browser virtual servers (external <script src>)
# ---------------------------------------------------------------------------


async def test_browser_create_with_virtual_servers(snapshot: bytes, tmp_path: Path) -> None:
    (tmp_path / "external-script.js").write_text("window.__ran = 1;")
    b = await Browser.create(
        snapshot=snapshot,
        mounts={"http://localhost/ext/": tmp_path},
    )
    b.runtime.eval(
        """document.head.innerHTML = '<script src="http://localhost/ext/external-script.js"></script>'"""
    )
    assert b.runtime.eval("window.__ran") == 1
