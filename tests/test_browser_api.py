import pytest
import pytest_asyncio
from jsrun import JavaScriptError

from htmxclient.browser import Browser, build_browser


@pytest_asyncio.fixture(scope="module")
async def app_browser(browser_snapshot):
    r = await build_browser("http://app.example.com/", snapshot=browser_snapshot)
    b = Browser(r)
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# Browser.load — sets HTML and initialises htmx on the content
# ---------------------------------------------------------------------------


async def test_load_sets_body(app_browser):
    await app_browser.load("<p id='msg'>hello</p>")
    assert app_browser.query("#msg") == "hello"


async def test_load_replaces_body(app_browser):
    await app_browser.load("<span id='a'>first</span>")
    await app_browser.load("<span id='b'>second</span>")
    assert app_browser.query("#b") == "second"
    # first load is gone
    result = app_browser.runtime.eval("document.querySelector('#a')")
    assert result is None


# ---------------------------------------------------------------------------
# Browser.query
# ---------------------------------------------------------------------------


async def test_query_inner_html(app_browser):
    await app_browser.load("<ul><li>a</li><li>b</li></ul>")
    assert "<li>a</li>" in app_browser.query("ul")


async def test_query_missing_element_raises(app_browser):
    await app_browser.load("<p>hi</p>")
    with pytest.raises(JavaScriptError):
        app_browser.query("#does-not-exist")


# ---------------------------------------------------------------------------
# Browser.trigger — hx-get swaps content from mocked server
# ---------------------------------------------------------------------------


async def test_trigger_hx_get(app_browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/fragment",
        text="<span>loaded</span>",
    )
    await app_browser.load(
        '<div id="target">'
        '<button hx-get="/fragment" hx-target="#target" hx-swap="innerHTML">load</button>'
        "</div>"
    )
    await app_browser.trigger("button")
    assert app_browser.query("#target") == "<span>loaded</span>"


async def test_trigger_hx_post(app_browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/submit",
        text="<p>saved</p>",
    )
    await app_browser.load(
        '<div id="result">'
        '<button hx-post="/submit" hx-target="#result" hx-swap="innerHTML">save</button>'
        "</div>"
    )
    await app_browser.trigger("button")
    assert app_browser.query("#result") == "<p>saved</p>"


async def test_trigger_request_sends_correct_method(app_browser, httpx_mock):
    httpx_mock.add_response(url="http://app.example.com/api", text="ok")
    await app_browser.load(
        '<div id="out"><button hx-post="/api" hx-target="#out">go</button></div>'
    )
    await app_browser.trigger("button")
    assert httpx_mock.get_request().method == "POST"


async def test_trigger_url_resolves_against_base(app_browser, httpx_mock):
    # hx-get="/path" should resolve to http://app.example.com/path
    httpx_mock.add_response(url="http://app.example.com/path", text="ok")
    await app_browser.load('<div id="r"><button hx-get="/path" hx-target="#r">go</button></div>')
    await app_browser.trigger("button")
    assert httpx_mock.get_request().url == "http://app.example.com/path"


# ---------------------------------------------------------------------------
# Browser as context manager
# ---------------------------------------------------------------------------


async def test_browser_context_manager(httpx_mock, browser_snapshot):
    httpx_mock.add_response(url="http://app.example.com/hi", text="<b>hi</b>")
    r = await build_browser("http://app.example.com/", snapshot=browser_snapshot)
    with Browser(r) as b:
        await b.load('<div id="r"><button hx-get="/hi" hx-target="#r">go</button></div>')
        await b.trigger("button")
        assert b.query("#r") == "<b>hi</b>"
