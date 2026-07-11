import json
from collections.abc import AsyncIterator
from unittest.mock import patch

import pytest
import pytest_asyncio
from conftest import htmx_script_tag, htmx_virtual_server

from htmxclient.browser import Browser
from htmxclient.runtime import build_runtime

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest_asyncio.fixture
async def htmx_browser(snapshot) -> AsyncIterator[Browser]:
    """A fresh htmx-loaded Browser, closed automatically unless the test closes it first."""
    url = "http://app.example.com/"
    r = await build_runtime(url, snapshot=snapshot, virtual_servers=[htmx_virtual_server(url)])
    r.eval(f"""
        document.open();
        document.write({json.dumps(htmx_script_tag(url))});
        document.close();""")
    b = Browser(r)
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# Browser.goto
# ---------------------------------------------------------------------------


async def test_goto_processes_htmx(htmx_browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/page",
        text="""\
        <html><body>
        <div id="out"><button hx-get="/frag" hx-target="#out" hx-swap="innerHTML">go</button></div>
        </body></html>""",
    )
    httpx_mock.add_response(url="http://app.example.com/frag", text="<b>done</b>")
    await htmx_browser.goto("http://app.example.com/page")
    btn = htmx_browser.find("button")
    await btn.click()
    assert htmx_browser.find("#out").innerHTML() == "<b>done</b>"


# ---------------------------------------------------------------------------
# Browser.trigger / Element.click / Element.dispatch_event
# ---------------------------------------------------------------------------


async def test_element_click_hx_get(htmx_browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/click-target",
        text="<b>clicked</b>",
    )
    await htmx_browser.load(
        '<div id="out">'
        '<button hx-get="/click-target" hx-target="#out" hx-swap="innerHTML">click</button>'
        "</div>"
    )
    btn = htmx_browser.find("button")
    await btn.click()
    assert htmx_browser.find("#out").innerHTML() == "<b>clicked</b>"


async def test_element_trigger_custom(htmx_browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/custom",
        text="<i>custom</i>",
    )
    await htmx_browser.load(
        '<div id="out">'
        '<button hx-get="/custom" hx-trigger="my-event" hx-target="#out" '
        'hx-swap="innerHTML">go</button>'
        "</div>"
    )
    btn = htmx_browser.find("button")
    await btn.trigger("my-event")
    assert htmx_browser.find("#out").innerHTML() == "<i>custom</i>"


# ---------------------------------------------------------------------------
# Element.submit
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("selector", ["#btn", "form"])
async def test_element_submit_form(htmx_browser, httpx_mock, selector):
    httpx_mock.add_response(
        url="http://app.example.com/form-action",
        text="<p>submitted</p>",
    )
    await htmx_browser.load(
        '<form id="f" hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">'
        '<input name="x" value="1">'
        '<button type="submit" id="btn">send</button>'
        "</form>"
        '<div id="result"></div>'
    )
    el = htmx_browser.find(selector)
    await el.submit()
    assert htmx_browser.find("#result").innerHTML() == "<p>submitted</p>"


async def test_element_submit_input(htmx_browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/form-action",
        text="<p>sent</p>",
    )
    await htmx_browser.load(
        '<form hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">'
        '<input type="text" name="q" value="search">'
        '<input type="submit" id="sub" value="go">'
        "</form>"
        '<div id="result"></div>'
    )
    sub = htmx_browser.find("#sub")
    await sub.submit()
    assert htmx_browser.find("#result").innerHTML() == "<p>sent</p>"


async def test_element_submit_htmx_handled(htmx_browser, httpx_mock):
    httpx_mock.add_response(url="http://app.example.com/form-action", text="<p>sent</p>")
    await htmx_browser.load(
        '<form hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">'
        '<button type="submit" id="btn">go</button>'
        "</form>"
        '<div id="result"></div>'
    )
    await htmx_browser.find("#btn").submit()
    assert htmx_browser.find("#result").innerHTML() == "<p>sent</p>"


# ---------------------------------------------------------------------------
# Browser as context manager
# ---------------------------------------------------------------------------


async def test_browser_context_manager(httpx_mock, htmx_browser):
    httpx_mock.add_response(url="http://app.example.com/hi", text="<b>hi</b>")
    with htmx_browser as b:
        await b.load('<div id="r"><button hx-get="/hi" hx-target="#r">go</button></div>')
        btn = b.find("button")
        assert btn is not None
        await btn.click()
        result = b.find("#r")
        assert result is not None
        assert result.innerHTML() == "<b>hi</b>"


async def test_browser_async_context_manager(httpx_mock, htmx_browser):
    httpx_mock.add_response(url="http://app.example.com/hi", text="<b>hi</b>")
    with patch.object(htmx_browser, "close", wraps=htmx_browser.close) as close_mock:
        async with htmx_browser as b:
            await b.load('<div id="r"><button hx-get="/hi" hx-target="#r">go</button></div>')
            btn = b.find("button")
            assert btn is not None
            await btn.click()
            result = b.find("#r")
            assert result is not None
            assert result.innerHTML() == "<b>hi</b>"
    close_mock.assert_called_once()
