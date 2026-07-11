import json
from unittest.mock import patch

import pytest
import pytest_asyncio
from conftest import htmx_script_tag, htmx_virtual_server

from htmxclient.browser import Browser
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


async def test_goto_processes_htmx(browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/page",
        text="""\
        <html><body>
        <div id="out"><button hx-get="/frag" hx-target="#out" hx-swap="innerHTML">go</button></div>
        </body></html>""",
    )
    httpx_mock.add_response(url="http://app.example.com/frag", text="<b>done</b>")
    await browser.goto("http://app.example.com/page")
    btn = browser.find("button")
    await btn.click()
    assert browser.find("#out").innerHTML() == "<b>done</b>"


# ---------------------------------------------------------------------------
# Browser.trigger / Element.click / Element.dispatch_event
# ---------------------------------------------------------------------------


async def test_element_click_hx_get(browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/click-target",
        text="<b>clicked</b>",
    )
    await browser.load(
        '<div id="out">'
        '<button hx-get="/click-target" hx-target="#out" hx-swap="innerHTML">click</button>'
        "</div>"
    )
    btn = browser.find("button")
    await btn.click()
    assert browser.find("#out").innerHTML() == "<b>clicked</b>"


async def test_element_trigger_custom(browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/custom",
        text="<i>custom</i>",
    )
    await browser.load(
        '<div id="out">'
        '<button hx-get="/custom" hx-trigger="my-event" hx-target="#out" '
        'hx-swap="innerHTML">go</button>'
        "</div>"
    )
    btn = browser.find("button")
    await btn.trigger("my-event")
    assert browser.find("#out").innerHTML() == "<i>custom</i>"


# ---------------------------------------------------------------------------
# Element.submit
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("selector", ["#btn", "form"])
async def test_element_submit_form(browser, httpx_mock, selector):
    httpx_mock.add_response(
        url="http://app.example.com/form-action",
        text="<p>submitted</p>",
    )
    await browser.load(
        '<form id="f" hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">'
        '<input name="x" value="1">'
        '<button type="submit" id="btn">send</button>'
        "</form>"
        '<div id="result"></div>'
    )
    el = browser.find(selector)
    await el.submit()
    assert browser.find("#result").innerHTML() == "<p>submitted</p>"


async def test_element_submit_input(browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/form-action",
        text="<p>sent</p>",
    )
    await browser.load(
        '<form hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">'
        '<input type="text" name="q" value="search">'
        '<input type="submit" id="sub" value="go">'
        "</form>"
        '<div id="result"></div>'
    )
    sub = browser.find("#sub")
    await sub.submit()
    assert browser.find("#result").innerHTML() == "<p>sent</p>"


async def test_element_submit_htmx_handled(browser, httpx_mock):
    httpx_mock.add_response(url="http://app.example.com/form-action", text="<p>sent</p>")
    await browser.load(
        '<form hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">'
        '<button type="submit" id="btn">go</button>'
        "</form>"
        '<div id="result"></div>'
    )
    await browser.find("#btn").submit()
    assert browser.find("#result").innerHTML() == "<p>sent</p>"


# ---------------------------------------------------------------------------
# Browser as context manager
# ---------------------------------------------------------------------------


async def test_browser_context_manager(httpx_mock, browser_snapshot):
    httpx_mock.add_response(url="http://app.example.com/hi", text="<b>hi</b>")
    b = await _htmx_browser("http://app.example.com/", browser_snapshot)
    with b:
        await b.load('<div id="r"><button hx-get="/hi" hx-target="#r">go</button></div>')
        btn = b.find("button")
        assert btn is not None
        await btn.click()
        result = b.find("#r")
        assert result is not None
        assert result.innerHTML() == "<b>hi</b>"


async def test_browser_async_context_manager(httpx_mock, browser_snapshot):
    httpx_mock.add_response(url="http://app.example.com/hi", text="<b>hi</b>")
    b = await _htmx_browser("http://app.example.com/", browser_snapshot)
    with patch.object(b, "close", wraps=b.close) as close_mock:
        async with b:
            await b.load('<div id="r"><button hx-get="/hi" hx-target="#r">go</button></div>')
            btn = b.find("button")
            assert btn is not None
            await btn.click()
            result = b.find("#r")
            assert result is not None
            assert result.innerHTML() == "<b>hi</b>"
    close_mock.assert_called_once()
