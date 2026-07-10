import json
from unittest.mock import patch

import pytest
import pytest_asyncio
from conftest import htmx_script_tag, htmx_virtual_server
from jsrun import JavaScriptError

from htmxclient.browser import Browser, Element, Response
from htmxclient.runtime import build_runtime

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


async def _htmx_browser(url: str, snapshot: bytes) -> Browser:
    """Build a Browser with the vendored htmx mounted and loaded via <script src>."""
    r = await build_runtime(url, snapshot=snapshot, virtual_servers=[htmx_virtual_server(url)])
    r.eval(f"""
        document.open();
        document.write({json.dumps(htmx_script_tag(url))});
        document.close();""")
    return Browser(r)


@pytest_asyncio.fixture(scope="module")
async def browser(browser_snapshot):
    b = await _htmx_browser("http://app.example.com/", browser_snapshot)
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# Window / document basics
# ---------------------------------------------------------------------------


async def test_window_instantiates(runtime):
    assert runtime.eval("typeof window") == "object"


async def test_document_basic(runtime):
    assert runtime.eval("document.createElement('div').tagName") == "DIV"


async def test_abort_controller(runtime):
    assert runtime.eval("new AbortController().signal.aborted") is False


# ---------------------------------------------------------------------------
# URL
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "js, expected",
    [
        ("new URL('http://example.com/path?q=1#anchor').protocol", "http:"),
        ("new URL('http://example.com/path?q=1#anchor').hostname", "example.com"),
        ("new URL('http://example.com/path?q=1#anchor').pathname", "/path"),
        ("new URL('http://example.com/path?q=1#anchor').search", "?q=1"),
        ("new URL('http://example.com/path?q=1#anchor').hash", "#anchor"),
        ("new URL('http://example.com/path?q=1#anchor').origin", "http://example.com"),
        # relative URL resolved against base
        ("new URL('/other', 'http://example.com/path').href", "http://example.com/other"),
        # URL.canParse static method
        ("URL.canParse('http://ok.com')", True),
        ("URL.canParse('not a url')", False),
        # happy-dom extends our polyfill; typeof still URL
        ("typeof window.URL", "function"),
    ],
)
async def test_url(runtime, js, expected):
    assert runtime.eval(js) == expected


# ---------------------------------------------------------------------------
# URLSearchParams
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "js, expected",
    [
        # basic get
        ("new URLSearchParams('a=1&b=2').get('a')", "1"),
        # missing key returns null
        ("new URLSearchParams('a=1').get('z')", None),
        # has
        ("new URLSearchParams('x=1').has('x')", True),
        # append + getAll
        (
            """
            const p = new URLSearchParams('k=1');
            p.append('k', '2');
            JSON.stringify(p.getAll('k'));""",
            '["1","2"]',
        ),
        # set replaces first, removes duplicates
        (
            "const p = new URLSearchParams('k=1&k=2'); p.set('k', '9'); p.toString()",
            "k=9",
        ),
        # + decoded as space per spec
        ("new URLSearchParams('q=hello+world').get('q')", "hello world"),
        # toString round-trips
        ("new URLSearchParams({a: '1', b: '2'}).toString()", "a=1&b=2"),
        # size property
        ("new URLSearchParams('a=1&b=2&c=3').size", 3),
    ],
)
async def test_url_search_params(runtime, js, expected):
    assert runtime.eval(js) == expected


# ---------------------------------------------------------------------------
# Buffer
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "js, expected",
    [
        # utf8 round-trip
        ("Buffer.from('hello', 'utf8').toString('utf8')", "hello"),
        # base64 encode
        ("Buffer.from('hello').toString('base64')", "aGVsbG8="),
        # base64 decode
        ("Buffer.from('aGVsbG8=', 'base64').toString('utf8')", "hello"),
        # hex encode
        ("Buffer.from('ab', 'utf8').toString('hex')", "6162"),
        # hex decode
        ("Buffer.from('6162', 'hex').toString('utf8')", "ab"),
        # isBuffer
        ("Buffer.isBuffer(Buffer.alloc(4))", True),
        ("Buffer.isBuffer(new Uint8Array(4))", False),
        # concat
        (
            "Buffer.concat([Buffer.from('foo'), Buffer.from('bar')]).toString()",
            "foobar",
        ),
        # alloc fills with zero by default
        ("Buffer.alloc(3).toString('hex')", "000000"),
        # from Array
        ("Buffer.from([0x68, 0x69]).toString()", "hi"),
    ],
)
async def test_buffer(runtime, js, expected):
    assert runtime.eval(js) == expected


# ---------------------------------------------------------------------------
# TextEncoder / TextDecoder
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "js, expected",
    [
        # encoding label
        ("new TextEncoder().encoding", "utf-8"),
        # round-trip ASCII
        (
            "new TextDecoder().decode(new TextEncoder().encode('hello'))",
            "hello",
        ),
        # round-trip multi-byte (2-byte UTF-8 codepoint: é = U+00E9)
        (
            "new TextDecoder().decode(new TextEncoder().encode('café'))",
            "café",
        ),
        # round-trip 3-byte UTF-8 codepoint: ☃ = U+2603
        (
            "new TextDecoder().decode(new TextEncoder().encode('☃'))",
            "☃",
        ),
        # encode returns Uint8Array
        ("new TextEncoder().encode('A') instanceof Uint8Array", True),
        # decode empty returns empty string
        ("new TextDecoder().decode(new Uint8Array(0))", ""),
    ],
)
async def test_text_encoder_decoder(runtime, js, expected):
    assert runtime.eval(js) == expected


# ---------------------------------------------------------------------------
# DOM manipulation
# ---------------------------------------------------------------------------


async def test_query_selector(runtime):
    runtime.eval("""\
        document.body.innerHTML = '<div id="x"><span class="y">hi</span></div>'
    """)
    assert runtime.eval("document.querySelector('#x .y').textContent") == "hi"


async def test_query_selector_all(runtime):
    runtime.eval("document.body.innerHTML = '<ul><li>a</li><li>b</li><li>c</li></ul>'")
    assert runtime.eval("document.querySelectorAll('li').length") == 3


async def test_inner_html_round_trip(runtime):
    runtime.eval("""\
        document.body.innerHTML = '<p id="p1">text</p>'
    """)
    assert runtime.eval("document.getElementById('p1').innerHTML") == "text"


async def test_create_element_attributes(runtime):
    result = runtime.eval("const a = document.createElement('a'); a.href = 'http://z.com'; a.href")
    assert "z.com" in result


# ---------------------------------------------------------------------------
# happy-dom globals exposed on window
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "js",
    [
        "typeof Headers",
        "typeof Request",
        "typeof Response",
        "typeof FormData",
        "typeof MutationObserver",
        "typeof CustomEvent",
        "typeof AbortController",
        # EventTarget and DOMParser are on window but not promoted to globalThis
        "typeof window.EventTarget",
        "typeof window.DOMParser",
    ],
)
async def test_globals_are_functions(runtime, js):
    assert runtime.eval(js) == "function"


async def test_headers_basic(runtime):
    result = runtime.eval(
        "const h = new Headers({'content-type': 'text/html'}); h.get('content-type')"
    )
    assert result == "text/html"


async def test_dom_parser(runtime):
    result = runtime.eval(
        "new window.DOMParser().parseFromString('<p>hi</p>', 'text/html')"
        ".querySelector('p').textContent"
    )
    assert result == "hi"


async def test_form_data_append(runtime):
    result = runtime.eval("const f = new FormData(); f.append('key', 'val'); f.get('key')")
    assert result == "val"


async def test_mutation_observer_callable(runtime):
    # Verifies MutationObserver can be instantiated without throwing
    result = runtime.eval("typeof new MutationObserver(() => {})")
    assert result == "object"


async def test_custom_event(runtime):
    result = runtime.eval("new CustomEvent('myevent', {detail: 42}).detail")
    assert result == 42


# ---------------------------------------------------------------------------
# atob / btoa
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "js, expected",
    [
        ("btoa('hello')", "aGVsbG8="),
        ("atob('aGVsbG8=')", "hello"),
        # round-trip
        ("atob(btoa('round trip'))", "round trip"),
    ],
)
async def test_atob_btoa(runtime, js, expected):
    assert runtime.eval(js) == expected


# ---------------------------------------------------------------------------
# setTimeout / clearTimeout / setInterval / clearInterval
# ---------------------------------------------------------------------------


async def test_settimeout_fires(runtime):
    result = await runtime.eval_async("new Promise(resolve => setTimeout(() => resolve('ok'), 10))")
    assert result == "ok"


async def test_settimeout_zero_fires(runtime):
    result = await runtime.eval_async("new Promise(resolve => setTimeout(() => resolve(42), 0))")
    assert result == 42


async def test_settimeout_passes_args(runtime):
    result = await runtime.eval_async(
        "new Promise(resolve => setTimeout((a, b) => resolve(a + b), 0, 3, 4))"
    )
    assert result == 7


async def test_cleartimeout_cancels(runtime):
    result = await runtime.eval_async("""
        new Promise(resolve => {
          let fired = false;
          const id = setTimeout(() => {
            fired = true;
          }, 50);
          clearTimeout(id);
          setTimeout(() => resolve(fired), 100);
        });
    """)
    assert result is False


async def test_settimeout_order(runtime):
    result = await runtime.eval_async("""
        new Promise(resolve => {
          const log = [];
          setTimeout(() => {
            log.push(1);
            if (log.length === 3) resolve(log);
          }, 10);
          setTimeout(() => {
            log.push(2);
            if (log.length === 3) resolve(log);
          }, 20);
          setTimeout(() => {
            log.push(3);
            if (log.length === 3) resolve(log);
          }, 30);
        });
    """)
    assert result == [1, 2, 3]


async def test_setinterval_fires_multiple_times(runtime):
    result = await runtime.eval_async("""
        new Promise(resolve => {
          let count = 0;
          const id = setInterval(() => {
            count++;
            if (count === 3) {
              clearInterval(id);
              resolve(count);
            }
          }, 10);
        });
    """)
    assert result == 3


async def test_clearinterval_stops_firing(runtime):
    result = await runtime.eval_async("""
        new Promise(resolve => {
          let count = 0;
          const id = setInterval(() => {
            count++;
          }, 10);
          setTimeout(() => {
            clearInterval(id);
            const snapshot = count;
            setTimeout(() => resolve(snapshot === count), 50);
          }, 35);
        });
    """)
    assert result is True


# ---------------------------------------------------------------------------
# Browser.goto
# ---------------------------------------------------------------------------


async def test_goto_head_and_title_body(browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/page",
        text="<html><head><title>T</title></head><body><span id='s'>ok</span></body></html>",
    )
    await browser.goto("http://app.example.com/page")
    assert browser.runtime.eval("document.title") == "T"
    assert browser.find("#s").innerHTML() == "ok"


@pytest.mark.parametrize(
    "status_code,ok",
    [(201, True), (404, False)],
)
async def test_goto_returns_response(browser, httpx_mock, status_code, ok):
    url, headers, text = "http://app.example.com/page", {"x-custom": "yes"}, "<p>hi</p>"
    httpx_mock.add_response(status_code=status_code, url=url, headers=headers, text=text)
    response = await browser.goto(url)
    assert response == Response(
        status=status_code,
        ok=ok,
        url=url,
        headers={
            "x-custom": "yes",
            "content-length": "9",
            "content-type": "text/plain; charset=utf-8",
        },
        text=text,
    )


async def test_goto_processes_htmx(browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/page",
        text="""\
        <html><body>
        <div id="out"><button hx-get="/frag" hx-target="#out" hx-swap="innerHTML">go</button></div>
        </body></html>""",
    )
    httpx_mock.add_response(url="http://app.example.com/frag", text="<b>done</b>")
    await browser.goto("http://app.example.com/page")
    btn = browser.find("button")
    await btn.click()
    assert browser.find("#out").innerHTML() == "<b>done</b>"


# ---------------------------------------------------------------------------
# Browser.load
# ---------------------------------------------------------------------------


async def test_load_sets_body(browser):
    await browser.load("<p id='msg'>hello</p>")
    assert browser.find("#msg").innerHTML() == "hello"


async def test_load_replaces_body(browser):
    await browser.load("<span id='a'>first</span>")
    await browser.load("<span id='b'>second</span>")
    assert browser.find("#b").innerHTML() == "second"
    # first load is gone
    result = browser.runtime.eval("document.querySelector('#a')")
    assert result is None


# ---------------------------------------------------------------------------
# Browser.find / Element queries
# ---------------------------------------------------------------------------


async def test_find_returns_element(browser):
    await browser.load("<p id='msg'>hello</p>")
    el = browser.find("#msg")
    assert isinstance(el, Element)
    assert el.innerHTML() == "hello"


async def test_find_returns_none_for_missing(browser):
    await browser.load("<p>hi</p>")
    assert browser.find("#does-not-exist") is None


async def test_element_text(browser):
    await browser.load("<div id='d'><span>inner</span> text</div>")
    el = browser.find("#d")
    assert el.text() == "inner text"


async def test_element_attr(browser):
    await browser.load("<a id='link' href='/path' data-x='42'>link</a>")
    el = browser.find("#link")
    assert el.attr("href") == "/path"
    assert el.attr("data-x") == "42"
    assert el.attr("missing") is None


async def test_element_fill(browser):
    await browser.load("<input id='inp' value='old'>")
    el = browser.find("#inp")
    el.fill("new")
    assert browser.runtime.eval("document.querySelector('#inp').value") == "new"


async def test_element_fill_textarea(browser):
    await browser.load("<textarea id='ta'>old</textarea>")
    el = browser.find("#ta")
    el.fill("new")
    assert browser.runtime.eval("document.querySelector('#ta').value") == "new"


async def test_find_all_returns_elements(browser):
    await browser.load("<ul><li>a</li><li>b</li><li>c</li></ul>")
    items = browser.find_all("li")
    assert len(items) == 3
    assert items[0].text() == "a"
    assert items[1].text() == "b"
    assert items[2].text() == "c"


async def test_find_all_empty(browser):
    await browser.load("<div>no items</div>")
    assert browser.find_all("li") == []


# ---------------------------------------------------------------------------
# Browser.trigger / Element.click / Element.dispatch_event
# ---------------------------------------------------------------------------


async def test_element_click_hx_get(browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/click-target",
        text="<b>clicked</b>",
    )
    await browser.load(
        '<div id="out">'
        '<button hx-get="/click-target" hx-target="#out" hx-swap="innerHTML">click</button>'
        "</div>"
    )
    btn = browser.find("button")
    await btn.click()
    assert browser.find("#out").innerHTML() == "<b>clicked</b>"


async def test_element_trigger_custom(browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/custom",
        text="<i>custom</i>",
    )
    await browser.load(
        '<div id="out">'
        '<button hx-get="/custom" hx-trigger="my-event" hx-target="#out" '
        'hx-swap="innerHTML">go</button>'
        "</div>"
    )
    btn = browser.find("button")
    await btn.trigger("my-event")
    assert browser.find("#out").innerHTML() == "<i>custom</i>"


# ---------------------------------------------------------------------------
# Element.submit
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("selector", ["#btn", "form"])
async def test_element_submit_form(browser, httpx_mock, selector):
    httpx_mock.add_response(
        url="http://app.example.com/form-action",
        text="<p>submitted</p>",
    )
    await browser.load(
        '<form id="f" hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">'
        '<input name="x" value="1">'
        '<button type="submit" id="btn">send</button>'
        "</form>"
        '<div id="result"></div>'
    )
    el = browser.find(selector)
    await el.submit()
    assert browser.find("#result").innerHTML() == "<p>submitted</p>"


async def test_element_submit_input(browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/form-action",
        text="<p>sent</p>",
    )
    await browser.load(
        '<form hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">'
        '<input type="text" name="q" value="search">'
        '<input type="submit" id="sub" value="go">'
        "</form>"
        '<div id="result"></div>'
    )
    sub = browser.find("#sub")
    await sub.submit()
    assert browser.find("#result").innerHTML() == "<p>sent</p>"


@pytest.mark.parametrize("selector", ["#btn", "form"])
@pytest.mark.parametrize(
    "method, expected_url, check_request",
    [
        (
            "post",
            "http://app.example.com/action",
            lambda req: (
                req.method == "POST"
                and req.content == b"x=42"
                and req.headers["content-type"] == "application/x-www-form-urlencoded"
            ),
        ),
        (
            "get",
            "http://app.example.com/action?x=42",
            lambda req: (
                req.method == "GET" and str(req.url) == "http://app.example.com/action?x=42"
            ),
        ),
    ],
    ids=["POST", "GET"],
)
async def test_element_submit_plain(
    browser, httpx_mock, selector, method, expected_url, check_request
):
    httpx_mock.add_response(url=expected_url, text="<body><p>done</p></body>")
    await browser.load(
        f'<form method="{method}" action="/action"><input name="x" value="42">'
        '<button type="submit" id="btn">go</button></form>'
    )
    response = await browser.find(selector).submit()
    assert check_request(httpx_mock.get_request())
    assert browser.find("p").text() == "done"
    assert response.status == 200
    assert response.ok is True
    assert response.url == expected_url
    assert response.text == "<body><p>done</p></body>"


async def test_element_submit_htmx_handled_returns_none(browser, httpx_mock):
    httpx_mock.add_response(url="http://app.example.com/form-action", text="<p>sent</p>")
    await browser.load(
        '<form hx-post="/form-action" hx-target="#result" hx-swap="innerHTML">'
        '<button type="submit" id="btn">go</button>'
        "</form>"
        '<div id="result"></div>'
    )
    response = await browser.find("#btn").submit()
    assert response is None


async def test_element_submit_no_form_raises(browser):
    await browser.load("<button id='btn'>orphan</button>")
    btn = browser.find("#btn")
    with pytest.raises(JavaScriptError):
        await btn.submit()


# ---------------------------------------------------------------------------
# Browser as context manager
# ---------------------------------------------------------------------------


async def test_browser_context_manager(httpx_mock, browser_snapshot):
    httpx_mock.add_response(url="http://app.example.com/hi", text="<b>hi</b>")
    b = await _htmx_browser("http://app.example.com/", browser_snapshot)
    with b:
        await b.load('<div id="r"><button hx-get="/hi" hx-target="#r">go</button></div>')
        btn = b.find("button")
        assert btn is not None
        await btn.click()
        result = b.find("#r")
        assert result is not None
        assert result.innerHTML() == "<b>hi</b>"


async def test_browser_async_context_manager(httpx_mock, browser_snapshot):
    httpx_mock.add_response(url="http://app.example.com/hi", text="<b>hi</b>")
    b = await _htmx_browser("http://app.example.com/", browser_snapshot)
    with patch.object(b, "close", wraps=b.close) as close_mock:
        async with b:
            await b.load('<div id="r"><button hx-get="/hi" hx-target="#r">go</button></div>')
            btn = b.find("button")
            assert btn is not None
            await btn.click()
            result = b.find("#r")
            assert result is not None
            assert result.innerHTML() == "<b>hi</b>"
    close_mock.assert_called_once()


# ---------------------------------------------------------------------------
# Browser virtual servers (external <script src>)
# ---------------------------------------------------------------------------


async def test_browser_create_with_virtual_servers(browser_snapshot, tmp_path):
    (tmp_path / "external-script.js").write_text("window.__ran = 1;")
    b = await Browser.create(
        snapshot=browser_snapshot,
        mounts={"http://localhost/ext/": tmp_path},
    )
    b.runtime.eval(
        """document.head.innerHTML = '<script src="http://localhost/ext/external-script.js"></script>'"""
    )
    assert b.runtime.eval("window.__ran") == 1
