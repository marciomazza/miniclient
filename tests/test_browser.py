import pytest
import pytest_asyncio
from jsrun import JavaScriptError

from htmxclient.browser import Browser, build_browser

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest_asyncio.fixture(scope="module")
async def app_browser(browser_snapshot):
    r = await build_browser("http://app.example.com/", snapshot=browser_snapshot)
    b = Browser(r)
    try:
        yield b
    finally:
        b.close()


# ---------------------------------------------------------------------------
# Window / document basics
# ---------------------------------------------------------------------------


def test_window_instantiates(browser):
    assert browser.eval("typeof window") == "object"


def test_document_basic(browser):
    assert browser.eval("document.createElement('div').tagName") == "DIV"


def test_abort_controller(browser):
    assert browser.eval("new AbortController().signal.aborted") is False


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
def test_url(browser, js, expected):
    assert browser.eval(js) == expected


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
            "const p = new URLSearchParams('k=1'); "
            + "p.append('k', '2'); JSON.stringify(p.getAll('k'))",
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
def test_url_search_params(browser, js, expected):
    assert browser.eval(js) == expected


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
def test_buffer(browser, js, expected):
    assert browser.eval(js) == expected


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
def test_text_encoder_decoder(browser, js, expected):
    assert browser.eval(js) == expected


# ---------------------------------------------------------------------------
# DOM manipulation
# ---------------------------------------------------------------------------


def test_query_selector(browser):
    browser.eval('document.body.innerHTML = \'<div id="x"><span class="y">hi</span></div>\'')
    assert browser.eval("document.querySelector('#x .y').textContent") == "hi"


def test_query_selector_all(browser):
    browser.eval("document.body.innerHTML = '<ul><li>a</li><li>b</li><li>c</li></ul>'")
    assert browser.eval("document.querySelectorAll('li').length") == 3


def test_inner_html_round_trip(browser):
    browser.eval("document.body.innerHTML = '<p id=\"p1\">text</p>'")
    assert browser.eval("document.getElementById('p1').innerHTML") == "text"


def test_create_element_attributes(browser):
    result = browser.eval("const a = document.createElement('a'); a.href = 'http://z.com'; a.href")
    assert "z.com" in result


# ---------------------------------------------------------------------------
# happy-dom globals exposed on window
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "expression",
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
def test_globals_are_functions(browser, expression):
    assert browser.eval(expression) == "function"


def test_headers_basic(browser):
    result = browser.eval(
        "const h = new Headers({'content-type': 'text/html'}); h.get('content-type')"
    )
    assert result == "text/html"


def test_dom_parser(browser):
    result = browser.eval(
        "new window.DOMParser().parseFromString('<p>hi</p>', 'text/html')"
        ".querySelector('p').textContent"
    )
    assert result == "hi"


def test_form_data_append(browser):
    result = browser.eval("const f = new FormData(); f.append('key', 'val'); f.get('key')")
    assert result == "val"


def test_mutation_observer_callable(browser):
    # Verifies MutationObserver can be instantiated without throwing
    result = browser.eval("typeof new MutationObserver(() => {})")
    assert result == "object"


def test_custom_event(browser):
    result = browser.eval("new CustomEvent('myevent', {detail: 42}).detail")
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
def test_atob_btoa(browser, js, expected):
    assert browser.eval(js) == expected


# ---------------------------------------------------------------------------
# setTimeout / clearTimeout / setInterval / clearInterval
# ---------------------------------------------------------------------------


async def test_settimeout_fires(browser_async):
    result = await browser_async.eval_async(
        "new Promise(resolve => setTimeout(() => resolve('ok'), 10))"
    )
    assert result == "ok"


async def test_settimeout_zero_fires(browser_async):
    result = await browser_async.eval_async(
        "new Promise(resolve => setTimeout(() => resolve(42), 0))"
    )
    assert result == 42


async def test_settimeout_passes_args(browser_async):
    result = await browser_async.eval_async(
        "new Promise(resolve => setTimeout((a, b) => resolve(a + b), 0, 3, 4))"
    )
    assert result == 7


async def test_cleartimeout_cancels(browser_async):
    result = await browser_async.eval_async("""
        new Promise(resolve => {
            let fired = false;
            const id = setTimeout(() => { fired = true; }, 50);
            clearTimeout(id);
            setTimeout(() => resolve(fired), 100);
        })
    """)
    assert result is False


async def test_settimeout_order(browser_async):
    result = await browser_async.eval_async("""
        new Promise(resolve => {
            const log = [];
            setTimeout(() => { log.push(1); if (log.length === 3) resolve(log); }, 10);
            setTimeout(() => { log.push(2); if (log.length === 3) resolve(log); }, 20);
            setTimeout(() => { log.push(3); if (log.length === 3) resolve(log); }, 30);
        })
    """)
    assert result == [1, 2, 3]


async def test_setinterval_fires_multiple_times(browser_async):
    result = await browser_async.eval_async("""
        new Promise(resolve => {
            let count = 0;
            const id = setInterval(() => {
                count++;
                if (count === 3) {
                    clearInterval(id);
                    resolve(count);
                }
            }, 10);
        })
    """)
    assert result == 3


async def test_clearinterval_stops_firing(browser_async):
    result = await browser_async.eval_async("""
        new Promise(resolve => {
            let count = 0;
            const id = setInterval(() => { count++; }, 10);
            setTimeout(() => {
                clearInterval(id);
                const snapshot = count;
                setTimeout(() => resolve(snapshot === count), 50);
            }, 35);
        })
    """)
    assert result is True


# ---------------------------------------------------------------------------
# Browser.load — sets HTML and initialises htmx on the content
# ---------------------------------------------------------------------------


async def test_load_sets_body(app_browser):
    await app_browser.load("<p id='msg'>hello</p>")
    assert app_browser.query("#msg") == "hello"


async def test_load_replaces_body(app_browser):
    await app_browser.load("<span id='a'>first</span>")
    await app_browser.load("<span id='b'>second</span>")
    assert app_browser.query("#b") == "second"
    # first load is gone
    result = app_browser.runtime.eval("document.querySelector('#a')")
    assert result is None


# ---------------------------------------------------------------------------
# Browser.query
# ---------------------------------------------------------------------------


async def test_query_inner_html(app_browser):
    await app_browser.load("<ul><li>a</li><li>b</li></ul>")
    assert "<li>a</li>" in app_browser.query("ul")


async def test_query_missing_element_raises(app_browser):
    await app_browser.load("<p>hi</p>")
    with pytest.raises(JavaScriptError):
        app_browser.query("#does-not-exist")


# ---------------------------------------------------------------------------
# Browser.trigger — hx-get swaps content from mocked server
# ---------------------------------------------------------------------------


async def test_trigger_hx_get(app_browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/fragment",
        text="<span>loaded</span>",
    )
    await app_browser.load(
        '<div id="target">'
        '<button hx-get="/fragment" hx-target="#target" hx-swap="innerHTML">load</button>'
        "</div>"
    )
    await app_browser.trigger("button")
    assert app_browser.query("#target") == "<span>loaded</span>"


async def test_trigger_hx_post(app_browser, httpx_mock):
    httpx_mock.add_response(
        url="http://app.example.com/submit",
        text="<p>saved</p>",
    )
    await app_browser.load(
        '<div id="result">'
        '<button hx-post="/submit" hx-target="#result" hx-swap="innerHTML">save</button>'
        "</div>"
    )
    await app_browser.trigger("button")
    assert app_browser.query("#result") == "<p>saved</p>"


async def test_trigger_request_sends_correct_method(app_browser, httpx_mock):
    httpx_mock.add_response(url="http://app.example.com/api", text="ok")
    await app_browser.load(
        '<div id="out"><button hx-post="/api" hx-target="#out">go</button></div>'
    )
    await app_browser.trigger("button")
    assert httpx_mock.get_request().method == "POST"


async def test_trigger_url_resolves_against_base(app_browser, httpx_mock):
    # hx-get="/path" should resolve to http://app.example.com/path
    httpx_mock.add_response(url="http://app.example.com/path", text="ok")
    await app_browser.load('<div id="r"><button hx-get="/path" hx-target="#r">go</button></div>')
    await app_browser.trigger("button")
    assert httpx_mock.get_request().url == "http://app.example.com/path"


# ---------------------------------------------------------------------------
# Browser as context manager
# ---------------------------------------------------------------------------


async def test_browser_context_manager(httpx_mock, browser_snapshot):
    httpx_mock.add_response(url="http://app.example.com/hi", text="<b>hi</b>")
    r = await build_browser("http://app.example.com/", snapshot=browser_snapshot)
    with Browser(r) as b:
        await b.load('<div id="r"><button hx-get="/hi" hx-target="#r">go</button></div>')
        await b.trigger("button")
        assert b.query("#r") == "<b>hi</b>"
