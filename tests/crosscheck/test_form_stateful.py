import pytest
from hypothesis import HealthCheck, Verbosity, given, settings, strategies as st
from playwright.async_api import Page

from crosscheck.crosscheck import CrossCheck
from crosscheck.strategies import (
    st_html_form,
    st_plain_html_form,
    st_some_text_maybe_empty,
    st_wsgi_app,
)

pytestmark = pytest.mark.cross


@pytest.mark.parametrize(
    "form_strategy,mode",
    [(st_html_form, "htmx"), (st_plain_html_form, "plain")],
)
@given(data=st.data())
@settings(
    max_examples=20,
    deadline=None,
    suppress_health_check=[HealthCheck.too_slow, HealthCheck.function_scoped_fixture],
    verbosity=Verbosity.debug,
)
async def test_form_stateful(page: Page, data, form_strategy, mode):
    app, form = data.draw(st_wsgi_app(form_strategy))
    cc = await CrossCheck.create(app, page, mode=mode)
    try:
        await cc.goto("/")
        ids = form.ids_by_interaction

        n_steps = data.draw(st.integers(0, 5))
        for _ in range(n_steps):
            available = {}
            for k in ("fill", "click", "submit"):
                if ids.get(k):
                    existing = [eid for eid in ids[k] if cc.has_element(f"#{eid}")]
                    if existing:
                        available[k] = existing
            if not available:
                break
            kind = data.draw(st.sampled_from(list(available.keys())))
            el_id = data.draw(st.sampled_from(available[kind]))
            if kind == "fill":
                await cc.fill(f"#{el_id}", data.draw(st_some_text_maybe_empty))
            else:
                await cc.click(f"#{el_id}", is_submit=(kind == "submit"))
            if kind == "submit":
                break
    finally:
        await cc.stop()
