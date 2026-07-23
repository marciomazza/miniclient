import asyncio
import json

import pytest
from hypothesis import HealthCheck, given, settings, strategies as st
from playwright.async_api import Page

from crosscheck.crosscheck import _JS_SERIALIZE
from crosscheck.strategies import st_html_form, st_some_text_maybe_empty
from miniclient.browser import AsyncBrowser

pytestmark = pytest.mark.cross


def _wrap(form_html: str) -> str:
    return f"<html><head></head><body>\n{form_html}\n<div id='result'></div>\n</body></html>"


class DomCheck:
    """Compares DOM state between V8/happy-dom and a real browser after each interaction.

    No network, no WSGI server, no htmx — HTML is injected directly.
    """

    def __init__(self, browser: AsyncBrowser, page: Page) -> None:
        self._browser = browser
        self._page = page

    @classmethod
    async def create(cls, html: str, page: Page) -> "DomCheck":
        browser = await AsyncBrowser()
        # Set innerHTML directly without htmx.process so that neither side adds
        # htmx-specific attributes (e.g. data-htmx-powered) during setup.
        browser.runtime.eval(f"document.documentElement.innerHTML = {json.dumps(html)}")
        await page.set_content(html, wait_until="domcontentloaded")
        return cls(browser, page)

    async def fill(self, selector: str, value: str) -> None:
        el = self._browser.find(selector)
        if el is None:
            raise LookupError(f"No element matches {selector!r}")
        await el.fill(value)
        await self._page.locator(selector).fill(value)

    async def click(self, selector: str) -> None:
        el = self._browser.find(selector)
        if el is None:
            raise LookupError(f"No element matches {selector!r}")
        await asyncio.gather(
            el.click(),
            self._page.locator(selector).click(),
        )

    async def assert_same_dom(self) -> None:
        client = self._browser.runtime.eval(f"f = {_JS_SERIALIZE}; f()")
        browser = await self._page.evaluate(_JS_SERIALIZE)
        assert client == browser

    async def aclose(self) -> None:
        await self._browser.aclose()


@given(form=st_html_form(), data=st.data())
@settings(
    max_examples=20,
    deadline=None,
    suppress_health_check=[HealthCheck.too_slow, HealthCheck.function_scoped_fixture],
)
async def test_dom_stateful(page: Page, form, data):
    dc = await DomCheck.create(_wrap(form.html), page)
    await dc.assert_same_dom()
    try:
        n_steps = data.draw(st.integers(0, 5))
        for _ in range(n_steps):
            interactable = {
                k: v for k, v in form.ids_by_interaction.items() if k in ("fill", "click")
            }
            if not interactable:
                break
            kind = data.draw(st.sampled_from(list(interactable.keys())))
            el_id = data.draw(st.sampled_from(interactable[kind]))
            if kind == "fill":
                value = data.draw(st_some_text_maybe_empty)
                await dc.fill(f"#{el_id}", value)
            elif kind == "click":
                await dc.click(f"#{el_id}")
            await dc.assert_same_dom()
    finally:
        await dc.aclose()
        del dc
        import gc

        gc.collect()
