from collections.abc import AsyncIterator
from unittest.mock import patch

import pytest
import pytest_asyncio
from conftest import HTMX_SCRIPT_TAG
from jsrun import Runtime
from pytest_httpx2 import HTTPXMock

from miniclient.page import AsyncFormElement, AsyncPage

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest_asyncio.fixture
async def htmx_page(runtime: Runtime) -> AsyncIterator[AsyncPage]:
    """A fresh AsyncPage, closed automatically unless the test closes it first. Each
    load()/goto() is a real navigation, so htmx must be (re-)loaded per page via
    HTMX_SCRIPT_TAG, same as a real page."""
    b = AsyncPage(runtime=runtime)
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# AsyncPage.goto
# ---------------------------------------------------------------------------


async def test_goto_processes_htmx(htmx_page: AsyncPage, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(
        url="http://localhost/page",
        text=f"""\
        <html><head>{HTMX_SCRIPT_TAG}</head><body>
        <div id="out"><button hx-get="/frag" hx-target="#out" hx-swap="innerHTML">go</button></div>
        </body></html>""",
    )
    httpx_mock.add_response(url="http://localhost/frag", text="<b>done</b>")
    await htmx_page.goto("http://localhost/page")
    btn = htmx_page.find("button")
    assert btn is not None
    await btn.click()
    el = htmx_page.find("#out")
    assert el is not None
    assert el.innerHTML == "<b>done</b>"


# ---------------------------------------------------------------------------
# AsyncPage.trigger / AsyncElement.click / AsyncElement.dispatch_event
# ---------------------------------------------------------------------------


async def test_element_click_hx_get(htmx_page: AsyncPage, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(
        url="http://localhost/click-target",
        text="<b>clicked</b>",
    )
    await htmx_page.load(
        f"""\
        {HTMX_SCRIPT_TAG}
        <div id="out">
        <button hx-get="/click-target" hx-target="#out" hx-swap="innerHTML">click</button>
        </div>"""
    )
    btn = htmx_page.find("button")
    assert btn is not None
    await btn.click()
    el = htmx_page.find("#out")
    assert el is not None
    assert el.innerHTML == "<b>clicked</b>"


async def test_element_trigger_custom(htmx_page: AsyncPage, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(
        url="http://localhost/custom",
        text="<i>custom</i>",
    )
    await htmx_page.load(
        f"""\
        {HTMX_SCRIPT_TAG}
        <div id="out">
        <button hx-get="/custom" hx-trigger="my-event" hx-target="#out"
        hx-swap="innerHTML">go</button>
        </div>"""
    )
    btn = htmx_page.find("button")
    assert btn is not None
    await btn.trigger("my-event")
    el = htmx_page.find("#out")
    assert el is not None
    assert el.innerHTML == "<i>custom</i>"


# ---------------------------------------------------------------------------
# AsyncFormElement.requestSubmit / submit-via-click
# ---------------------------------------------------------------------------


async def test_element_request_submit_form(htmx_page: AsyncPage, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(
        url="http://localhost/form-action",
        text="<p>submitted</p>",
    )
    await htmx_page.load(
        f"""\
        {HTMX_SCRIPT_TAG}
        <form id="f" hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">
        <input name="x" value="1">
        <button type="submit" id="btn">send</button>
        </form>
        <div id="result"></div>"""
    )
    form = htmx_page.find("form")
    assert isinstance(form, AsyncFormElement)
    await form.requestSubmit()
    result = htmx_page.find("#result")
    assert result is not None
    assert result.innerHTML == "<p>submitted</p>"


@pytest.mark.parametrize(
    "submitter_html",
    [
        '<input type="text" name="q" value="search"><input type="submit" id="sub" value="go">',
        '<input type="text" name="q" value="search"><button type="submit" id="sub">go</button>',
    ],
    ids=["input-submit", "button-submit"],
)
async def test_submit_via_submitter_click(
    htmx_page: AsyncPage, httpx_mock: HTTPXMock, submitter_html: str
) -> None:
    httpx_mock.add_response(
        url="http://localhost/form-action",
        text="<p>sent</p>",
    )
    await htmx_page.load(
        f"""\
        {HTMX_SCRIPT_TAG}
        <form hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">
        {submitter_html}
        </form>
        <div id="result"></div>"""
    )
    sub = htmx_page.find("#sub")
    assert sub is not None
    await sub.click()
    result = htmx_page.find("#result")
    assert result is not None
    assert result.innerHTML == "<p>sent</p>"


# ---------------------------------------------------------------------------
# AsyncPage as context manager
# ---------------------------------------------------------------------------


async def test_page_context_manager(httpx_mock: HTTPXMock, htmx_page: AsyncPage) -> None:
    httpx_mock.add_response(url="http://localhost/hi", text="<b>hi</b>")
    with htmx_page as b:
        await b.load(
            HTMX_SCRIPT_TAG + '<div id="r"><button hx-get="/hi" hx-target="#r">go</button></div>'
        )
        btn = b.find("button")
        assert btn is not None
        await btn.click()
        result = b.find("#r")
        assert result is not None
        assert result.innerHTML == "<b>hi</b>"


async def test_page_async_context_manager(httpx_mock: HTTPXMock, htmx_page: AsyncPage) -> None:
    httpx_mock.add_response(url="http://localhost/hi", text="<b>hi</b>")
    with patch.object(htmx_page, "close", wraps=htmx_page.close) as close_mock:
        async with htmx_page as b:
            await b.load(
                HTMX_SCRIPT_TAG
                + '<div id="r"><button hx-get="/hi" hx-target="#r">go</button></div>'
            )
            btn = b.find("button")
            assert btn is not None
            await btn.click()
            result = b.find("#r")
            assert result is not None
            assert result.innerHTML == "<b>hi</b>"
    close_mock.assert_called_once()
