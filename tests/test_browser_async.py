from collections.abc import AsyncIterator, Callable
from pathlib import Path

import httpx2 as httpx
import pytest
import pytest_asyncio
from pytest_httpx2 import HTTPXMock

from miniclient.browser import AsyncBrowser, AsyncElement, AsyncFormElement
from miniclient.runtime import build_runtime

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest_asyncio.fixture
async def browser(snapshot: bytes) -> AsyncIterator[AsyncBrowser]:
    """A fresh htmx-loaded AsyncBrowser, closed automatically unless the test closes it first."""
    runtime = await build_runtime(snapshot=snapshot)
    runtime.eval("""__document_write(`
        <!DOCTYPE html>
        <html>
          <body>
            <div id="test-playground"></div>
          </body>
        </html>
    `)""")
    assert runtime.eval("typeof htmx") == "undefined"  # make sure there is no htmx here
    b = AsyncBrowser(runtime=runtime)
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# AsyncBrowser.goto
# ---------------------------------------------------------------------------


async def test_goto_head_and_title_body(browser: AsyncBrowser, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(
        url="http://localhost/page",
        text="<html><head><title>T</title></head><body><span id='s'>ok</span></body></html>",
    )
    await browser.goto("http://localhost/page")
    assert browser.runtime.eval("document.title") == "T"
    el = browser.find("#s")
    assert el is not None
    assert el.innerHTML() == "ok"


async def test_goto_navigates_to_different_domain(
    browser: AsyncBrowser, httpx_mock: HTTPXMock
) -> None:
    httpx_mock.add_response(
        url="http://localhost/start",
        text="<html><body><p>start</p></body></html>",
    )
    httpx_mock.add_response(
        url="http://example.com/other",
        text="<html><body><p>other</p></body></html>",
    )
    await browser.goto("http://localhost/start")
    assert browser.runtime.eval("location.href") == "http://localhost/start"
    assert browser.runtime.eval("document.baseURI") == "http://localhost/start"

    await browser.goto("http://example.com/other")
    assert browser.runtime.eval("location.href") == "http://example.com/other"
    assert browser.runtime.eval("document.baseURI") == "http://example.com/other"


@pytest.mark.parametrize(
    "initial_url, relative_url, expected_url",
    [
        ("http://localhost/start/page", "other", "http://localhost/start/other"),
        ("http://localhost/start/page", "/", "http://localhost/"),
        (None, "/", "http://localhost/"),
    ],
    ids=["relative_to_current_page", "root_relative_after_nav", "root_relative_no_prior_nav"],
)
async def test_goto_resolves_relative_urls(
    browser: AsyncBrowser,
    httpx_mock: HTTPXMock,
    initial_url: str | None,
    relative_url: str,
    expected_url: str,
) -> None:
    if initial_url is not None:
        httpx_mock.add_response(url=initial_url, text="<html><body><p>start</p></body></html>")
        await browser.goto(initial_url)
    httpx_mock.add_response(url=expected_url, text="<html><body><p>target</p></body></html>")
    await browser.goto(relative_url)
    assert browser.runtime.eval("location.href") == expected_url


# ---------------------------------------------------------------------------
# AsyncBrowser.load
# ---------------------------------------------------------------------------


async def test_load_sets_body(browser: AsyncBrowser) -> None:
    await browser.load("<p id='msg'>hello</p>")
    el = browser.find("body")
    assert el and el.innerHTML() == '<p id="msg">hello</p>'


async def test_load_replaces_body(browser: AsyncBrowser) -> None:
    await browser.load("<span id='a'>first</span>")
    await browser.load("<span id='b'>second</span>")
    el = browser.find("#b")
    assert el is not None
    assert el.innerHTML() == "second"
    # first load is gone
    result = browser.runtime.eval("document.querySelector('#a')")
    assert result is None


# ---------------------------------------------------------------------------
# AsyncBrowser.find / AsyncElement queries
# ---------------------------------------------------------------------------


async def test_find_returns_element(browser: AsyncBrowser) -> None:
    await browser.load("<p id='msg'>hello</p>")
    el = browser.find("#msg")
    assert isinstance(el, AsyncElement)
    assert el.innerHTML() == "hello"


async def test_find_returns_none_for_missing(browser: AsyncBrowser) -> None:
    await browser.load("<p>hi</p>")
    assert browser.find("#does-not-exist") is None


async def test_element_html(browser: AsyncBrowser) -> None:
    await browser.load("<div id='d'><span>inner</span> text</div>")
    el = browser.find("#d")
    assert el is not None
    assert el.html() == '<div id="d"><span>inner</span> text</div>'


async def test_element_text(browser: AsyncBrowser) -> None:
    await browser.load("<div id='d'><span>inner</span> text</div>")
    el = browser.find("#d")
    assert el is not None
    assert el.text() == "inner text"


async def test_element_attr(browser: AsyncBrowser) -> None:
    await browser.load("<a id='link' href='/path' data-x='42'>link</a>")
    el = browser.find("#link")
    assert el is not None
    assert el.attr("href") == "/path"
    assert el.attr("data-x") == "42"
    assert el.attr("missing") is None


async def test_element_fill(browser: AsyncBrowser) -> None:
    await browser.load("<input id='inp' value='old'>")
    el = browser.find("#inp")
    assert el is not None
    el.fill("new")
    assert browser.runtime.eval("document.querySelector('#inp').value") == "new"


async def test_element_fill_textarea(browser: AsyncBrowser) -> None:
    await browser.load("<textarea id='ta'>old</textarea>")
    el = browser.find("#ta")
    assert el is not None
    el.fill("new")
    assert browser.runtime.eval("document.querySelector('#ta').value") == "new"


async def test_find_all_returns_elements(browser: AsyncBrowser) -> None:
    await browser.load("<ul><li>a</li><li>b</li><li>c</li></ul>")
    items = browser.find_all("li")
    assert len(items) == 3
    assert items[0].text() == "a"
    assert items[1].text() == "b"
    assert items[2].text() == "c"


async def test_find_all_empty(browser: AsyncBrowser) -> None:
    await browser.load("<div>no items</div>")
    assert browser.find_all("li") == []


# ---------------------------------------------------------------------------
# AsyncFormElement.requestSubmit
# ---------------------------------------------------------------------------


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
async def test_form_request_submit_plain(
    browser: AsyncBrowser,
    httpx_mock: HTTPXMock,
    method: str,
    expected_url: str,
    check_request: Callable[[httpx.Request], bool],
) -> None:
    httpx_mock.add_response(url=expected_url, text="<body><p>done</p></body>")
    await browser.load(
        f'<form method="{method}" action="/action"><input name="x" value="42">'
        '<button type="submit" id="btn">go</button></form>'
    )
    form = browser.find("form")
    assert isinstance(form, AsyncFormElement)
    await form.requestSubmit()
    request = httpx_mock.get_request()
    assert request is not None
    assert check_request(request)
    el = browser.find("p")
    assert el is not None
    assert el.text() == "done"


# ---------------------------------------------------------------------------
# AsyncBrowser virtual servers (external <script src>)
# ---------------------------------------------------------------------------


async def test_browser_create_with_virtual_servers(snapshot: bytes, tmp_path: Path) -> None:
    (tmp_path / "external-script.js").write_text("window.__ran = 1;")
    b = await AsyncBrowser(
        snapshot=snapshot,
        mounts={"http://localhost/ext/": tmp_path},
    )
    b.runtime.eval(
        """document.head.innerHTML = '<script src="http://localhost/ext/external-script.js"></script>'"""
    )
    assert b.runtime.eval("window.__ran") == 1
