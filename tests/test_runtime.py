import pytest

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
# deno_web APIs — spec behaviour is deno's; we only check pre_globals.js wired
# them onto globalThis and they survived the cold snapshot pass. Behaviour that
# mini actually patches is guarded in test_url.py.
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "name",
    [
        "URL",
        "URLSearchParams",
        "TextEncoder",
        "TextDecoder",
        "atob",
        "btoa",
        "performance",
        "PerformanceObserver",
        "ReadableStream",
    ],
)
async def test_deno_web_globals_wired(runtime, name):
    assert runtime.eval(f"typeof globalThis.{name} !== 'undefined'")


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
        """\
        new window.DOMParser().parseFromString('<p>hi</p>', 'text/html').querySelector('p')
          .textContent"""
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


async def test_performance_real_impl(runtime):
    # deno_web's performance: monotonic now(), working marks/measures/entries.
    assert runtime.eval("performance.now() >= 0 && typeof performance.now() === 'number'")
    assert runtime.eval("performance.timeOrigin > 0")
    result = runtime.eval("""\
        performance.mark('a');
        performance.measure('m', 'a');
        JSON.stringify([
            performance.getEntriesByType('mark').length,
            performance.getEntriesByName('m', 'measure').length,
        ])
    """)
    assert result == "[1,1]"


async def test_performance_observer_fires(runtime):
    result = await runtime.eval_async("""\
        (async () => {
            const seen = [];
            const po = new PerformanceObserver((list) => {
                for (const e of list.getEntries()) seen.push(e.entryType);
            });
            po.observe({ entryTypes: ['mark'] });
            performance.mark('watched');
            await new Promise((r) => setTimeout(r, 10));
            return seen.join(',');
        })()
    """)
    assert result == "mark"


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
            const current_count = count;
            setTimeout(() => resolve(current_count === count), 50);
          }, 35);
        });
    """)
    assert result is True
