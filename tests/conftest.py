import asyncio

import pytest
import pytest_asyncio

from htmxclient.browser import build_browser


@pytest.fixture()
def browser():
    async def _build():
        return await build_browser()

    with asyncio.run(_build()) as r:
        yield r


@pytest_asyncio.fixture
async def browser_async():
    r = await build_browser()
    try:
        yield r
    finally:
        r.close()
