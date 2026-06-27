from collections.abc import AsyncGenerator
from pathlib import Path

import pytest
from playwright.async_api import Page, async_playwright

_SETTLE_INIT_SCRIPT = Path(__file__).parent / "settle_init.js"


@pytest.fixture
async def page() -> AsyncGenerator[Page, None]:
    async with async_playwright() as pw:
        browser = await pw.chromium.launch()
        context = await browser.new_context()
        p = await context.new_page()
        await p.add_init_script(path=str(_SETTLE_INIT_SCRIPT))
        yield p
        await p.close()
        await context.close()
        await browser.close()
