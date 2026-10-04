import gc
from collections.abc import Iterator
from pathlib import Path

import pytest
from conftest import HTMX_RESOURCE_MOUNT, HTMX_SCRIPT_TAG
from pytest_httpx2 import HTTPXMock

from miniclient.browser import Element, Page

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def page() -> Iterator[Page]:
    """A fresh Page, closed automatically unless the test closes it first."""
    b = Page()
    try:
        yield b
    finally:
        b.close()


@pytest.fixture
def htmx_page() -> Iterator[Page]:
    """A fresh Page that can reach the vendored htmx.js, closed automatically unless
    the test closes it first. Each load()/goto() is a real navigation, so htmx must be
    (re-)loaded per page via HTMX_SCRIPT_TAG, same as a real page."""
    b = Page(mounts={url: Path(directory) for url, directory in HTMX_RESOURCE_MOUNT.items()})
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# Bridge relays calls/results correctly.
#
# JS-generation correctness (query selectors, attribute handling, htmx
# settle logic, ...) is already covered exhaustively by cargo test
# (tests/browser_queries.rs, tests/browser_navigation.rs, tests/browser_htmx.rs) against
# browser::Page/Element, and by test_htmx_integration.py against AsyncPage. These only check
# that going through the sync facade's run_until_complete bridge doesn't change the result —
# so one broad test per bridge "shape" (plain queries/mutations, htmx interactions) rather than
# one test per method.
# ---------------------------------------------------------------------------


def test_marshalling_smoke(page: Page, httpx_mock: HTTPXMock) -> None:
    """Marshalling smoke test only: goto/find/find_all/attr/parent/fill each return the right
    Python shape (Element | None, list[Element], str | None) across the pyo3 boundary. The
    query/mutation behavior itself is already covered natively by tests/browser_queries.rs and
    tests/browser_navigation.rs against browser::Page/Element."""
    httpx_mock.add_response(
        url="http://localhost/page",
        text="<span id='s' data-x='1'>ok</span><input id='inp' value='old'><ul><li>a</li></ul>",
    )
    page.goto("http://localhost/page")

    el = page.find("#s")
    assert isinstance(el, Element)
    assert el.attr("data-x") == "1"
    assert el.attr("missing") is None
    assert el.parent and el.parent.find("#inp") is not None
    assert page.find("#does-not-exist") is None

    inp = page.find("#inp")
    assert inp is not None
    inp.fill("new")

    assert [i.text for i in page.find_all("li")] == ["a"]


def test_click_and_form_submit_via_htmx(htmx_page: Page, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(url="http://localhost/click-target", text="<b>clicked</b>")
    httpx_mock.add_response(url="http://localhost/form-action", text="<p>submitted</p>")
    htmx_page.load(
        f"""\
        {HTMX_SCRIPT_TAG}
        <div id="out">
        <button hx-get="/click-target" hx-target="#out" hx-swap="innerHTML">click</button>
        </div>
        <form id="f" hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">
        <input name="x" value="1">
        <button type="submit">send</button>
        </form>
        <div id="result"></div>
    """
    )
    btn = htmx_page.find("button")
    assert btn is not None
    btn.click()
    out = htmx_page.find("#out")
    assert out and out.inner_html == "<b>clicked</b>"

    form = htmx_page.find("form")
    assert form is not None and form.tag_name == "FORM"
    form.request_submit()
    result = htmx_page.find("#result")
    assert result and result.inner_html == "<p>submitted</p>"


# ---------------------------------------------------------------------------
# Sync-specific behavior — no AsyncPage equivalent to mirror.
# ---------------------------------------------------------------------------


def test_page_context_manager_closes() -> None:
    with Page() as b:
        b.load("<p>hi</p>")
        assert b.find("p") is not None
    assert b._closed


# ---------------------------------------------------------------------------
# Page GC lifecycle (regression)
#
# Runtime has no thread affinity (it owns its own dedicated OS thread and
# dispatches over a channel — see src/runtime.rs), and close() is idempotent,
# so dropping it from anywhere is safe by construction. These guard that a
# Page/Element still held past close(), or never closed at all, tears down
# silently instead of raising.
# ---------------------------------------------------------------------------


def test_gc_regression_held_element() -> None:
    # Element kept alive past Page.close() must not raise when
    # it's eventually garbage collected.
    with Page() as b:
        b.load("<button id='x'>hi</button>")
        el = b.find("#x")
        assert el is not None
    del el
    gc.collect()


def test_gc_regression_never_closed() -> None:
    # A Page that is never explicitly closed must still clean up silently
    # via __del__ when it's dropped.
    b = Page()
    b.load("<button id='x'>hi</button>")
    el = b.find("#x")
    assert el is not None
    el.click()
    del b, el
    gc.collect()


async def test_page_rejects_running_event_loop() -> None:
    # Page drives its own event loop on the caller's thread; it can't nest
    # inside one that's already running (e.g. this test coroutine's own loop).
    with pytest.raises(RuntimeError, match="running event loop"):
        Page()


def test_goto_after_hash_change_does_not_hang(page: Page, httpx_mock: HTTPXMock) -> None:
    """The hash change queues a timer; goto()'s timer sweep must not leave it counted as live."""
    for path in ("/a", "/b"):
        httpx_mock.add_response(url=f"http://localhost{path}", text=f"<p>{path}</p>")
    page.goto("http://localhost/a")
    page.eval("location.hash = '#x'")
    page.goto("http://localhost/b")
    assert page.eval("location.href") == "http://localhost/b"
