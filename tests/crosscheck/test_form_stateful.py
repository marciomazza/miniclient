import pytest
from hypothesis import HealthCheck, given, settings, strategies as st
from playwright.async_api import Page

from crosscheck.crosscheck import CrossCheck
from crosscheck.strategies import st_html_form, st_some_text_maybe_empty, st_wsgi_app

pytestmark = pytest.mark.cross


@given(data=st.data())
@settings(
    max_examples=3,
    deadline=None,
    suppress_health_check=[HealthCheck.too_slow, HealthCheck.function_scoped_fixture],
)
async def test_form_stateful(page: Page, data):
    app, form = data.draw(st_wsgi_app(st_html_form))
    cc = await CrossCheck.create(app, page)
    try:
        await cc.goto("/")
        ids = form.ids_by_interaction

        n_steps = data.draw(st.integers(0, 5))
        for _ in range(n_steps):
            available = {k: ids[k] for k in ("fill", "click", "submit") if ids.get(k)}
            if not available:
                break
            kind = data.draw(st.sampled_from(list(available.keys())))
            el_id = data.draw(st.sampled_from(available[kind]))
            if kind == "fill":
                await cc.fill(f"#{el_id}", data.draw(st_some_text_maybe_empty))
            else:
                await cc.click(f"#{el_id}")
            if kind == "submit":
                break
    finally:
        await cc.stop()
