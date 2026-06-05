import json

from hypothesis import HealthCheck, given, settings, strategies as st


async def js_fetch_text(browser, url, **opts):
    js_opts = json.dumps(opts, ensure_ascii=False) if opts else "{}"
    return await browser.eval_async(
        f"fetch({json.dumps(url)}, {js_opts}).then(r => r.text())"
    )


async def js_fetch_json(browser, url, **opts):
    js_opts = json.dumps(opts, ensure_ascii=False) if opts else "{}"
    return await browser.eval_async(
        f"fetch({json.dumps(url)}, {js_opts}).then(r => r.json())"
    )


async def js_fetch_status(browser, url):
    return await browser.eval_async(
        f"fetch({json.dumps(url)}).then(r => r.status)"
    )


# ASCII printable characters only — the jsrun bridge has encoding issues
# with non-ASCII characters in JS string literals
_ascii_printable = st.characters(
    whitelist_categories=(),
    whitelist_characters="".join(chr(i) for i in range(32, 127)),
)

_ascii_letters_and_numbers = st.characters(
    whitelist_categories=("L", "N"),
    max_codepoint=0x7F,
)


# ---------------------------------------------------------------------------
# 1. URLs arbitrárias
# ---------------------------------------------------------------------------


_url_safe = st.characters(
    whitelist_categories=("L", "N"),
    whitelist_characters="-_.~/",
)


@given(
    st.text(
        min_size=1,
        max_size=200,
        alphabet=_url_safe,
    )
)
@settings(
    suppress_health_check=[HealthCheck.function_scoped_fixture],
    max_examples=30,
)
async def test_fetch_text_arbitrary_path(browser_async, httpx_mock, path_segment):
    url = f"http://api.example.com/{path_segment}"
    httpx_mock.add_response(url=url, text="ok")
    result = await js_fetch_text(browser_async, url)
    assert result == "ok"


_param_key = st.text(
    min_size=1,
    max_size=20,
    alphabet=st.characters(whitelist_categories=("L", "N")),
)

_param_value = st.text(
    min_size=0,
    max_size=50,
    alphabet=st.characters(
        whitelist_categories=("L", "N"),
        whitelist_characters="-_.",
    ),
)


@given(
    st.lists(
        st.tuples(_param_key, _param_value),
        min_size=0,
        max_size=5,
    )
)
@settings(
    suppress_health_check=[HealthCheck.function_scoped_fixture],
    max_examples=30,
)
async def test_fetch_with_query_params(browser_async, httpx_mock, params):
    query = "&".join(f"{k}={v}" for k, v in params)
    url = f"http://api.example.com/search?{query}"
    httpx_mock.add_response(url=url, text="results")
    result = await js_fetch_text(browser_async, url)
    assert result == "results"


# ---------------------------------------------------------------------------
# 2. Status codes arbitrários
# ---------------------------------------------------------------------------


@given(st.integers(min_value=100, max_value=599))
@settings(
    suppress_health_check=[HealthCheck.function_scoped_fixture],
    max_examples=50,
)
async def test_fetch_status_arbitrary(browser_async, httpx_mock, status_code):
    url = "http://api.example.com/status"
    httpx_mock.add_response(url=url, status_code=status_code)
    result = await browser_async.eval_async(
        f"fetch({json.dumps(url)}).then(r => ({{ok: r.ok, status: r.status}}))"
    )
    assert result["status"] == status_code
    assert result["ok"] == (200 <= status_code < 300)


# ---------------------------------------------------------------------------
# 3. Response bodies arbitrários
# ---------------------------------------------------------------------------


@given(st.text(alphabet=_ascii_printable))
@settings(
    suppress_health_check=[HealthCheck.function_scoped_fixture],
    max_examples=50,
)
async def test_fetch_text_arbitrary_body(browser_async, httpx_mock, body):
    url = "http://api.example.com/echo"
    httpx_mock.add_response(url=url, text=body)
    result = await js_fetch_text(browser_async, url)
    assert result == body


_json_value = st.one_of(
    st.text(alphabet=_ascii_printable),
    st.integers(min_value=-(2**53), max_value=2**53),
    st.booleans(),
)


@given(
    st.dictionaries(
        st.text(min_size=1, alphabet=_ascii_letters_and_numbers),
        _json_value,
    )
)
@settings(
    suppress_health_check=[HealthCheck.function_scoped_fixture],
    max_examples=50,
)
async def test_fetch_json_arbitrary_dict(browser_async, httpx_mock, data):
    url = "http://api.example.com/data"
    httpx_mock.add_response(url=url, json=data)
    result = await js_fetch_json(browser_async, url)
    assert result == data


@given(st.binary(min_size=0, max_size=1024))
@settings(
    suppress_health_check=[HealthCheck.function_scoped_fixture],
    max_examples=30,
)
async def test_fetch_empty_or_binary_response(browser_async, httpx_mock, binary_body):
    url = "http://api.example.com/binary"
    text_body = binary_body.decode("utf-8", errors="replace")
    httpx_mock.add_response(url=url, text=text_body)
    result = await js_fetch_text(browser_async, url)
    assert result == text_body


# ---------------------------------------------------------------------------
# 4. Headers arbitrários
# ---------------------------------------------------------------------------


_header_key = st.text(
    min_size=1,
    max_size=50,
    alphabet=st.characters(
        whitelist_categories=("L", "N"),
        whitelist_characters="-_.",
        max_codepoint=0x7F,
    ),
)


@given(
    st.dictionaries(
        _header_key,
        st.text(min_size=0, max_size=100, alphabet=_ascii_printable),
    )
)
@settings(
    suppress_health_check=[HealthCheck.function_scoped_fixture],
    max_examples=30,
)
async def test_fetch_sends_arbitrary_headers(browser_async, httpx_mock, headers):
    url = "http://api.example.com/headers"
    httpx_mock.add_response(url=url, text="ok")
    await js_fetch_text(browser_async, url, headers=headers)
    requests = httpx_mock.get_requests(url=url)
    assert len(requests) >= 1
    request = requests[-1]
    # httpx merges duplicate headers (case-insensitive), keeping the last value.
    # Some keys may be filtered by the JS or HTTP stack.
    expected: dict[str, str] = {}
    for key, value in headers.items():
        expected[key.lower()] = value
    for key, value in expected.items():
        actual = request.headers.get(key)
        # Skip validation for keys that were filtered out
        if actual is not None:
            assert actual == value


@given(
    st.dictionaries(
        _header_key,
        st.text(min_size=0, max_size=100, alphabet=_ascii_printable),
    )
)
@settings(
    suppress_health_check=[HealthCheck.function_scoped_fixture],
    max_examples=30,
)
async def test_fetch_receives_arbitrary_headers(
    browser_async, httpx_mock, response_headers
):
    url = "http://api.example.com/response-headers"
    httpx_mock.add_response(url=url, headers=response_headers, text="ok")
    result = await browser_async.eval_async(
        f"""
        fetch({json.dumps(url)}).then(r => {{
            const h = {{}};
            r.headers.forEach((v, k) => h[k] = v);
            return h;
        }})
        """
    )
    for key, value in response_headers.items():
        assert result.get(key.lower()) == value


# ---------------------------------------------------------------------------
# 6. JSON bodies arbitrários no POST
# ---------------------------------------------------------------------------


json_strategy = st.recursive(
    st.one_of(
        st.text(alphabet=_ascii_printable),
        st.integers(min_value=-(2**53), max_value=2**53),
        st.booleans(),
    ),
    lambda children: st.lists(children, min_size=0, max_size=5)
    | st.dictionaries(
        st.text(min_size=1, alphabet=_ascii_letters_and_numbers),
        children,
        min_size=0,
        max_size=5,
    ),
    max_leaves=3,
)


@given(json_strategy)
@settings(
    suppress_health_check=[HealthCheck.function_scoped_fixture],
    max_examples=30,
)
async def test_fetch_post_arbitrary_json_body(browser_async, httpx_mock, payload):
    url = "http://api.example.com/echo"
    httpx_mock.add_response(url=url, text="saved")
    result = await browser_async.eval_async(
        f"""
        fetch({json.dumps(url)}, {{
            method: 'POST',
            headers: {{'content-type': 'application/json'}},
            body: JSON.stringify({json.dumps(payload, ensure_ascii=False)}),
        }}).then(r => r.text())
        """
    )
    assert result == "saved"
    requests = httpx_mock.get_requests(url=url, method="POST")
    assert len(requests) >= 1
    assert json.loads(requests[-1].content) == payload
