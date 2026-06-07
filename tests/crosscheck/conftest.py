from collections.abc import AsyncGenerator

import pytest
from playwright.async_api import Page, async_playwright


@pytest.fixture
async def page() -> AsyncGenerator[Page, None]:
    async with async_playwright() as pw:
        browser = await pw.chromium.launch()
        context = await browser.new_context()
        p = await context.new_page()
        yield p
        await p.close()
        await context.close()
        await browser.close()
