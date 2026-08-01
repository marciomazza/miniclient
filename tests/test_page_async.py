from collections.abc import AsyncIterator, Callable
from pathlib import Path

import httpx2 as httpx
import pytest
import pytest_asyncio
from pytest_httpx2 import HTTPXMock

from miniclient.page import AsyncElement, AsyncFormElement, AsyncPage

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest_asyncio.fixture
async def page(v8_snapshot: bytes) -> AsyncIterator[AsyncPage]:
    """A fresh htmx-loaded AsyncPage, closed automatically unless the test closes it first."""
    b = await AsyncPage(v8_snapshot=v8_snapshot)
    await b.runtime.eval_async("""__document_write(`
        <!DOCTYPE html>
        <html>
          <body>
            <div id="test-playground"></div>
          </body>
        </html>
    `)""")
    assert b.runtime.eval("typeof htmx") == "undefined"  # make sure there is no htmx here
    try:
        yield b
    finally:
        await b.aclose()


# ---------------------------------------------------------------------------
# AsyncPage.goto
# ---------------------------------------------------------------------------


async def test_goto_head_and_title_body(page: AsyncPage, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(
        url="http://localhost/page",
        text="<html><head><title>T</title></head><body><span id='s'>ok</span></body></html>",
    )
    await page.goto("http://localhost/page")
    assert page.runtime.eval("document.title") == "T"
    el = page.find("#s")
    assert el is not None
    assert el.innerHTML == "ok"


async def test_goto_navigates_to_different_domain(page: AsyncPage, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(
        url="http://localhost/start",
        text="<html><body><p>start</p></body></html>",
    )
    httpx_mock.add_response(
        url="http://example.com/other",
        text="<html><body><p>other</p></body></html>",
    )
    await page.goto("http://localhost/start")
    assert page.runtime.eval("location.href") == "http://localhost/start"
    assert page.runtime.eval("document.baseURI") == "http://localhost/start"

    await page.goto("http://example.com/other")
    assert page.runtime.eval("location.href") == "http://example.com/other"
    assert page.runtime.eval("document.baseURI") == "http://example.com/other"


@pytest.mark.parametrize(
    "initial_url, relative_url, expected_url",
    [
        ("http://localhost/start/page", "other", "http://localhost/start/other"),
        ("http://localhost/start/page", "/", "http://localhost/"),
        (None, "/", "http://localhost/"),
    ],
    ids=["relative_to_current_page", "root_relative_after_nav", "root_relative_no_prior_nav"],
)
async def test_goto_resolves_relative_urls(
    page: AsyncPage,
    httpx_mock: HTTPXMock,
    initial_url: str | None,
    relative_url: str,
    expected_url: str,
) -> None:
    if initial_url is not None:
        httpx_mock.add_response(url=initial_url, text="<html><body><p>start</p></body></html>")
        await page.goto(initial_url)
    httpx_mock.add_response(url=expected_url, text="<html><body><p>target</p></body></html>")
    await page.goto(relative_url)
    assert page.runtime.eval("location.href") == expected_url


async def test_goto_isolates_globals_across_navigations(
    page: AsyncPage, httpx_mock: HTTPXMock
) -> None:
    """A script's global writes and pending timers from one page must not survive
    navigation to the next: bootstrap.js's registerWindowGlobals() resets globalThis
    to a clean baseline on every goto()."""
    httpx_mock.add_response(
        url="http://localhost/a",
        text="""<html><body><script>
            window.__foo = 1;
            setTimeout(() => { window.__leaked = true; }, 20);
        </script></body></html>""",
    )
    httpx_mock.add_response(url="http://localhost/b", text="<html><body><p>b</p></body></html>")

    await page.goto("http://localhost/a")
    assert page.runtime.eval("window.__foo") == 1

    await page.goto("http://localhost/b")
    assert page.runtime.eval("typeof window.__foo") == "undefined"

    # give page A's pending timer a chance to fire; goto() must have cleared it
    await page.runtime.eval_async("new Promise(resolve => setTimeout(resolve, 50))")
    assert page.runtime.eval("typeof window.__leaked") == "undefined"


# ---------------------------------------------------------------------------
# AsyncPage.load
# ---------------------------------------------------------------------------


async def test_load_sets_body(page: AsyncPage) -> None:
    await page.load("<p id='msg'>hello</p>")
    el = page.find("body")
    assert el and el.innerHTML == '<p id="msg">hello</p>'


async def test_load_replaces_body(page: AsyncPage) -> None:
    await page.load("<span id='a'>first</span>")
    await page.load("<span id='b'>second</span>")
    el = page.find("#b")
    assert el is not None
    assert el.innerHTML == "second"
    # first load is gone
    result = page.runtime.eval("document.querySelector('#a')")
    assert result is None


# ---------------------------------------------------------------------------
# AsyncPage.find / AsyncElement queries
# ---------------------------------------------------------------------------


async def test_find_returns_element(page: AsyncPage) -> None:
    await page.load("<p id='msg'>hello</p>")
    el = page.find("#msg")
    assert isinstance(el, AsyncElement)
    assert el.innerHTML == "hello"


async def test_find_returns_none_for_missing(page: AsyncPage) -> None:
    await page.load("<p>hi</p>")
    assert page.find("#does-not-exist") is None


async def test_element_html(page: AsyncPage) -> None:
    await page.load("<div id='d'><span>inner</span> text</div>")
    el = page.find("#d")
    assert el is not None
    assert el.html == '<div id="d"><span>inner</span> text</div>'


async def test_element_text(page: AsyncPage) -> None:
    await page.load("<div id='d'><span>inner</span> text</div>")
    el = page.find("#d")
    assert el is not None
    assert el.text == "inner text"


async def test_element_attr(page: AsyncPage) -> None:
    await page.load("<a id='link' href='/path' data-x='42'>link</a>")
    el = page.find("#link")
    assert el is not None
    assert el.attr("href") == "/path"
    assert el.attr("data-x") == "42"
    assert el.attr("missing") is None


async def test_element_parent(page: AsyncPage) -> None:
    await page.load("<div id='d'><span id='s'>hi</span></div>")
    el = page.find("#s")
    assert el and el.parent and el.parent.attr("id") == "d"


async def test_element_parent_is_none_for_root_html_element(page: AsyncPage) -> None:
    # <html>'s parent is the document node, not an Element, so parentElement is null
    el = page.find("html")
    assert el is not None
    assert el.parent is None


async def test_element_fill(page: AsyncPage) -> None:
    await page.load("<input id='inp' value='old'>")
    el = page.find("#inp")
    assert el is not None
    await el.fill("new")
    assert page.runtime.eval("document.querySelector('#inp').value") == "new"


async def test_element_fill_textarea(page: AsyncPage) -> None:
    await page.load("<textarea id='ta'>old</textarea>")
    el = page.find("#ta")
    assert el is not None
    await el.fill("new")
    assert page.runtime.eval("document.querySelector('#ta').value") == "new"


async def test_find_all_returns_elements(page: AsyncPage) -> None:
    await page.load("<ul><li>a</li><li>b</li><li>c</li></ul>")
    items = page.find_all("li")
    assert len(items) == 3
    assert items[0].text == "a"
    assert items[1].text == "b"
    assert items[2].text == "c"


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        (None, "apple"),  # no filter: first match wins
        ("ap", "apple"),  # substring matches multiple: first one wins
        ("banana", "banana"),  # substring matches a single, non-first element
        ("xyz", None),  # substring matches nothing
    ],
)
async def test_find_with_text(page: AsyncPage, text: str | None, expected: str | None) -> None:
    await page.load("<ul><li>apple</li><li>banana</li><li>apricot</li></ul>")
    el = page.find("li", text=text)
    assert (el.text if el is not None else None) == expected


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        (None, ["apple", "banana", "apricot"]),  # no filter: every match
        ("ap", ["apple", "apricot"]),  # substring matches some
        ("xyz", []),  # substring matches none
    ],
)
async def test_find_all_with_text(page: AsyncPage, text: str | None, expected: list[str]) -> None:
    await page.load("<ul><li>apple</li><li>banana</li><li>apricot</li></ul>")
    items = page.find_all("li", text=text)
    assert [item.text for item in items] == expected


async def test_find_all_empty(page: AsyncPage) -> None:
    await page.load("<div>no items</div>")
    assert page.find_all("li") == []


# ---------------------------------------------------------------------------
# AsyncElement.find / find_all — scoped to the element, not the whole document
# ---------------------------------------------------------------------------


async def test_element_find_returns_descendant(page: AsyncPage) -> None:
    await page.load("<div id='d'><p id='msg'>hello</p></div>")
    d = page.find("#d")
    assert d is not None
    el = d.find("#msg")
    assert isinstance(el, AsyncElement)
    assert el.innerHTML == "hello"


async def test_element_find_ignores_matches_outside_itself(page: AsyncPage) -> None:
    await page.load("<div id='d'><p class='x'>inside</p></div><p class='x'>outside</p>")
    d = page.find("#d")
    assert d is not None
    items = d.find_all(".x")
    assert [item.text for item in items] == ["inside"]


async def test_element_find_returns_none_for_missing(page: AsyncPage) -> None:
    await page.load("<div id='d'><p>hi</p></div>")
    d = page.find("#d")
    assert d is not None
    assert d.find("#does-not-exist") is None


async def test_element_find_all_returns_elements(page: AsyncPage) -> None:
    await page.load("<ul id='list'><li>a</li><li>b</li></ul>")
    ul = page.find("#list")
    assert ul is not None
    items = ul.find_all("li")
    assert [item.text for item in items] == ["a", "b"]


async def test_element_find_returns_form_element(page: AsyncPage) -> None:
    await page.load("<div id='d'><form id='f'></form></div>")
    d = page.find("#d")
    assert d is not None
    form = d.find("form")
    assert isinstance(form, AsyncFormElement)


# ---------------------------------------------------------------------------
# AsyncFormElement.requestSubmit
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "method, expected_url, check_request",
    [
        (
            "post",
            "http://localhost/action",
            lambda req: (
                req.method == "POST"
                and req.content == b"x=42"
                and req.headers["content-type"] == "application/x-www-form-urlencoded"
            ),
        ),
        (
            "get",
            "http://localhost/action?x=42",
            lambda req: req.method == "GET" and str(req.url) == "http://localhost/action?x=42",
        ),
    ],
    ids=["POST", "GET"],
)
async def test_form_request_submit_plain(
    page: AsyncPage,
    httpx_mock: HTTPXMock,
    method: str,
    expected_url: str,
    check_request: Callable[[httpx.Request], bool],
) -> None:
    httpx_mock.add_response(url=expected_url, text="<body><p>done</p></body>")
    await page.load(
        f"""\
        <form method="{method}" action="/action"><input name="x" value="42">
        <button type="submit" id="btn">go</button></form>"""
    )
    form = page.find("form")
    assert isinstance(form, AsyncFormElement)
    await form.requestSubmit()
    request = httpx_mock.get_request()
    assert request is not None
    assert check_request(request)
    el = page.find("p")
    assert el is not None
    assert el.text == "done"


# ---------------------------------------------------------------------------
# AsyncPage virtual servers (external <script src>)
# ---------------------------------------------------------------------------


async def test_page_create_with_virtual_servers(v8_snapshot: bytes, tmp_path: Path) -> None:
    (tmp_path / "external-script.js").write_text("window.__ran = 1;")
    b = await AsyncPage(
        v8_snapshot=v8_snapshot,
        mounts={"http://localhost/ext/": tmp_path},
    )
    b.runtime.eval(
        """document.head.innerHTML = '<script src="http://localhost/ext/external-script.js"></script>'"""
    )
    assert b.runtime.eval("window.__ran") == 1


def _load_script_js(attr_setup_js: str, url: str) -> str:
    """JS that appends a <script src=url>, applying attr_setup_js first, awaits its load/error."""
    return f"""
        new Promise((resolve, reject) => {{
            const script = document.createElement('script');
            {attr_setup_js}
            script.src = {url!r};
            script.onload = resolve;
            script.onerror = () => reject(new Error('script failed to load'));
            document.head.appendChild(script);
        }})
    """


@pytest.mark.parametrize(
    "attr_setup_js",
    ["script.async = true;", "script.defer = true;", "script.type = 'module';"],
    ids=["async", "defer", "module"],
)
async def test_page_script_src_virtual_server(
    v8_snapshot: bytes, tmp_path: Path, attr_setup_js: str
) -> None:
    (tmp_path / "external-script.js").write_text("window.__ran = 1;")
    b = await AsyncPage(
        v8_snapshot=v8_snapshot,
        mounts={"http://localhost/ext/": tmp_path},
    )
    await b.runtime.eval_async(
        _load_script_js(attr_setup_js, "http://localhost/ext/external-script.js")
    )
    assert b.runtime.eval("window.__ran") == 1


async def test_page_module_script_relative_import(v8_snapshot: bytes, tmp_path: Path) -> None:
    (tmp_path / "helper.js").write_text("export const value = 1;")
    (tmp_path / "entry.js").write_text("import { value } from './helper.js'; window.__ran = value;")
    b = await AsyncPage(
        v8_snapshot=v8_snapshot,
        mounts={"http://localhost/ext/": tmp_path},
    )
    await b.runtime.eval_async(
        _load_script_js("script.type = 'module';", "http://localhost/ext/entry.js")
    )
    assert b.runtime.eval("window.__ran") == 1


async def test_page_load_fires_dom_content_loaded_after_module_script(
    v8_snapshot: bytes, tmp_path: Path
) -> None:
    (tmp_path / "entry.js").write_text(
        "document.addEventListener('DOMContentLoaded', () => { window.__dclFired = true; });"
    )
    b = await AsyncPage(v8_snapshot=v8_snapshot, mounts={"http://localhost/ext/": tmp_path})
    await b.load('<script type="module" src="http://localhost/ext/entry.js"></script>')
    assert b.runtime.eval("window.__dclFired") is True
