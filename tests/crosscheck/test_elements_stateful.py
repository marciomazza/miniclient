import pytest
from hypothesis import HealthCheck, Verbosity, given, settings, strategies as st
from playwright.async_api import Page

from crosscheck.crosscheck import CrossCheck
from crosscheck.strategies import (
    st_some_text_maybe_empty,
    st_wsgi_rich_page,
)

pytestmark = pytest.mark.hypo

_EVENTS = ["click", "mouseenter", "focus", "dblclick", "mouseover", "change", "blur"]


@given(data=st.data())
@settings(
    max_examples=50,
    deadline=None,
    suppress_health_check=[HealthCheck.too_slow, HealthCheck.function_scoped_fixture],
    verbosity=Verbosity.debug,
)
async def test_elements_stateful(page: Page, data):
    app, pg = data.draw(st_wsgi_rich_page())
    cc = await CrossCheck.create(app, page, mode="htmx")
    try:
        await cc.goto("/")
        n_steps = data.draw(st.integers(0, 5))
        for _ in range(n_steps):
            choices = ["event"]
            if pg.fillable_ids:
                choices.append("fill")
            action = data.draw(st.sampled_from(choices))
            if action == "fill":
                el_id = data.draw(st.sampled_from(pg.fillable_ids))
                await cc.fill(f"#{el_id}", data.draw(st_some_text_maybe_empty))
            else:
                el_id = data.draw(st.sampled_from(pg.all_ids))
                event = data.draw(st.sampled_from(_EVENTS))
                if not cc.has_element(f"#{el_id}"):
                    continue
                await cc.dispatch_event(f"#{el_id}", event)
    finally:
        await cc.stop()
