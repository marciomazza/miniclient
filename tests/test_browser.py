import json

import pytest
import pytest_asyncio
from conftest import htmx_script_tag, htmx_virtual_server
from jsrun import JavaScriptError

from htmxclient.browser import Browser, Element
from htmxclient.runtime import build_runtime

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


async def _htmx_browser(url: str, snapshot: bytes) -> Browser:
    """Build a Browser with the vendored htmx mounted and loaded via <script src>."""
    r = await build_runtime(url, snapshot=snapshot, virtual_servers=[htmx_virtual_server(url)])
    r.eval(f"""
        document.open();
        document.write({json.dumps(htmx_script_tag(url))});
        document.close();""")
    return Browser(r)


@pytest_asyncio.fixture(scope="module")
async def browser(browser_snapshot):
    b = await _htmx_browser("http://app.example.com/", browser_snapshot)
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# Browser.goto
# ---------------------------------------------------------------------------


async def test_goto_head_and_title_body(browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/page",
        text="<html><head><title>T</title></head><body><span id='s'>ok</span></body></html>",
    )
    await browser.goto("http://app.example.com/page")
    assert browser.runtime.eval("document.title") == "T"
    assert browser.find("#s").innerHTML() == "ok"


# ---------------------------------------------------------------------------
# Browser.load
# ---------------------------------------------------------------------------


async def test_load_sets_body(browser):
    await browser.load("<p id='msg'>hello</p>")
    assert browser.find("#msg").innerHTML() == "hello"


async def test_load_replaces_body(browser):
    await browser.load("<span id='a'>first</span>")
    await browser.load("<span id='b'>second</span>")
    assert browser.find("#b").innerHTML() == "second"
    # first load is gone
    result = browser.runtime.eval("document.querySelector('#a')")
    assert result is None


# ---------------------------------------------------------------------------
# Browser.find / Element queries
# ---------------------------------------------------------------------------


async def test_find_returns_element(browser):
    await browser.load("<p id='msg'>hello</p>")
    el = browser.find("#msg")
    assert isinstance(el, Element)
    assert el.innerHTML() == "hello"


async def test_find_returns_none_for_missing(browser):
    await browser.load("<p>hi</p>")
    assert browser.find("#does-not-exist") is None


async def test_element_text(browser):
    await browser.load("<div id='d'><span>inner</span> text</div>")
    el = browser.find("#d")
    assert el.text() == "inner text"


async def test_element_attr(browser):
    await browser.load("<a id='link' href='/path' data-x='42'>link</a>")
    el = browser.find("#link")
    assert el.attr("href") == "/path"
    assert el.attr("data-x") == "42"
    assert el.attr("missing") is None


async def test_element_fill(browser):
    await browser.load("<input id='inp' value='old'>")
    el = browser.find("#inp")
    el.fill("new")
    assert browser.runtime.eval("document.querySelector('#inp').value") == "new"


async def test_element_fill_textarea(browser):
    await browser.load("<textarea id='ta'>old</textarea>")
    el = browser.find("#ta")
    el.fill("new")
    assert browser.runtime.eval("document.querySelector('#ta').value") == "new"


async def test_find_all_returns_elements(browser):
    await browser.load("<ul><li>a</li><li>b</li><li>c</li></ul>")
    items = browser.find_all("li")
    assert len(items) == 3
    assert items[0].text() == "a"
    assert items[1].text() == "b"
    assert items[2].text() == "c"


async def test_find_all_empty(browser):
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
            "http://app.example.com/action",
            lambda req: (
                req.method == "POST"
                and req.content == b"x=42"
                and req.headers["content-type"] == "application/x-www-form-urlencoded"
            ),
        ),
        (
            "get",
            "http://app.example.com/action?x=42",
            lambda req: (
                req.method == "GET" and str(req.url) == "http://app.example.com/action?x=42"
            ),
        ),
    ],
    ids=["POST", "GET"],
)
async def test_element_submit_plain(
    browser, httpx_mock, selector, method, expected_url, check_request
):
    httpx_mock.add_response(url=expected_url, text="<body><p>done</p></body>")
    await browser.load(
        f'<form method="{method}" action="/action"><input name="x" value="42">'
        '<button type="submit" id="btn">go</button></form>'
    )
    await browser.find(selector).submit()
    assert check_request(httpx_mock.get_request())
    assert browser.find("p").text() == "done"


async def test_element_submit_no_form_raises(browser):
    await browser.load("<button id='btn'>orphan</button>")
    btn = browser.find("#btn")
    with pytest.raises(JavaScriptError):
        await btn.submit()


# ---------------------------------------------------------------------------
# Browser virtual servers (external <script src>)
# ---------------------------------------------------------------------------


async def test_browser_create_with_virtual_servers(browser_snapshot, tmp_path):
    (tmp_path / "external-script.js").write_text("window.__ran = 1;")
    b = await Browser.create(
        snapshot=browser_snapshot,
        mounts={"http://localhost/ext/": tmp_path},
    )
    b.runtime.eval(
        """document.head.innerHTML = '<script src="http://localhost/ext/external-script.js"></script>'"""
    )
    assert b.runtime.eval("window.__ran") == 1
