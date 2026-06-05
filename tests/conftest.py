import asyncio

import pytest

from htmxclient.browser import build_browser


@pytest.fixture()
def browser():
    async def _build():
        return await build_browser()

    with asyncio.run(_build()) as r:
        yield r
