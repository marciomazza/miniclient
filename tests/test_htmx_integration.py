from collections.abc import AsyncIterator
from pathlib import Path
from unittest.mock import patch

import pytest_asyncio
from conftest import HTMX_RESOURCE_MOUNT, HTMX_SCRIPT_TAG
from pytest_httpx2 import HTTPXMock

from miniclient.page import AsyncPage

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest_asyncio.fixture
async def htmx_page() -> AsyncIterator[AsyncPage]:
    """A fresh AsyncPage that can reach the vendored htmx.js, closed automatically
    unless the test closes it first. Each load()/goto() is a real navigation, so
    htmx must be (re-)loaded per page via HTMX_SCRIPT_TAG, same as a real page."""
    async with AsyncPage(
        mounts={url: Path(directory) for url, directory in HTMX_RESOURCE_MOUNT.items()}
    ) as b:
        yield b


# ---------------------------------------------------------------------------
# AsyncPage as context manager -- pyo3/Python facade's own `with`/`async with` protocol, not
# reachable through browser::Page/Element (cargo test coverage of the underlying htmx/click/
# goto behavior lives in tests/browser_htmx.rs and tests/browser_navigation.rs).
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
        assert result.inner_html == "<b>hi</b>"


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
            assert result.inner_html == "<b>hi</b>"
    close_mock.assert_called_once()
