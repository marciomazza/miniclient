import json

from hypothesis import HealthCheck, assume, given, settings, strategies as st


def _js_str(s: str) -> str:
    """Serialize a Python string as a JS string literal (handles astral chars)."""
    return json.dumps(s, ensure_ascii=False)


# ---------------------------------------------------------------------------
# Round-trip: atob(btoa(s)) == s
# btoa accepts Latin-1 (codepoints 0-255)


@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
@given(st.text(alphabet=st.characters(max_codepoint=255)))
def test_atob_btoa_round_trip(runtime, s):
    encoded = runtime.eval(f"btoa({_js_str(s)})")
    decoded = runtime.eval(f"atob({_js_str(encoded)})")
    assert decoded == s


# ---------------------------------------------------------------------------
# Round-trip: TextEncoder / TextDecoder


@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
@given(st.text())
def test_text_encoder_decoder_round_trip(runtime, s):
    result = runtime.eval(f"new TextDecoder().decode(new TextEncoder().encode({_js_str(s)}))")
    assert result == s


# ---------------------------------------------------------------------------
# Round-trip: Buffer hex


@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
@given(st.text())
def test_buffer_hex_round_trip(runtime, s):
    hex_str = runtime.eval(f"Buffer.from({_js_str(s)}, 'utf8').toString('hex')")
    result = runtime.eval(f"Buffer.from({_js_str(hex_str)}, 'hex').toString('utf8')")
    assert result == s


# ---------------------------------------------------------------------------
# Round-trip: Buffer base64


@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
@given(st.text())
def test_buffer_base64_round_trip(runtime, s):
    b64 = runtime.eval(f"Buffer.from({_js_str(s)}, 'utf8').toString('base64')")
    result = runtime.eval(f"Buffer.from({_js_str(b64)}, 'base64').toString('utf8')")
    assert result == s


# ---------------------------------------------------------------------------
# Round-trip: URLSearchParams toString -> parse


@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
@given(st.dictionaries(st.text(), st.text()))
def test_url_search_params_round_trip(runtime, params):
    # URLSearchParams encodes + as space and &/= as delimiters
    safe = {
        k: v
        for k, v in params.items()
        if "+" not in k
        and "+" not in v
        and "&" not in k
        and "&" not in v
        and "=" not in k
        and "=" not in v
    }
    pairs = ", ".join(f"[{_js_str(k)}, {_js_str(v)}]" for k, v in safe.items())
    js = f"""(() => {{
    const p = new URLSearchParams([{pairs}]);
    const p2 = new URLSearchParams(p.toString());
    return JSON.stringify(Array.from(p2.entries()));
    }})()"""
    result = runtime.eval(js)
    parsed = json.loads(result)
    assert parsed == [[k, v] for k, v in safe.items()]


# ---------------------------------------------------------------------------
# Round-trip: URL href


@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
@given(
    st.text(alphabet=st.characters(whitelist_categories=("Lu", "Ll", "Nd")), min_size=1),
    st.text(
        alphabet=st.characters(whitelist_categories=("Lu", "Ll", "Nd", "Po", "Zs")), min_size=1
    ),
    st.text(
        alphabet=st.characters(whitelist_categories=("Lu", "Ll", "Nd", "Po", "Zs")), min_size=1
    ),
)
def test_url_href_round_trip(runtime, host, path, query):
    safe_path = path.replace(" ", "%20").replace("?", "%3F")
    safe_query = query.replace(" ", "%20").replace("&", "%26")
    href = f"http://{host}.test/{safe_path}?q={safe_query}"
    result = runtime.eval(f"new URL({_js_str(href)}).href")
    assert result == href


# ---------------------------------------------------------------------------
# Property: Buffer concat length


@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
@given(st.text(), st.text())
def test_buffer_concat_length(runtime, a, b):
    js = f"""(() => {{
    const a = Buffer.from({_js_str(a)}, 'utf8');
    const b = Buffer.from({_js_str(b)}, 'utf8');
    const ab = Buffer.concat([a, b]);
    return [a.length, b.length, ab.length];
    }})()"""
    result = runtime.eval(js)
    la, lb, lab = result
    assert lab == la + lb


# ---------------------------------------------------------------------------
# Property: URLSearchParams size matches unique keys


@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
@given(st.dictionaries(st.text(), st.text(), min_size=1))
def test_url_search_params_size(runtime, params):
    pairs = ", ".join(f"[{_js_str(k)}, {_js_str(v)}]" for k, v in params.items())
    js = f"""(() => {{
    const p = new URLSearchParams([{pairs}]);
    return p.size;
    }})()"""
    result = runtime.eval(js)
    assert result == len(params)


# ---------------------------------------------------------------------------
# Property: Headers append/get returns last value


@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
@given(st.text(min_size=1), st.text(min_size=1), st.text(min_size=1))
def test_headers_append_has(runtime, key, val1, val2):
    assume(key.lower() != "__proto__")  # this reveals a bug in happy-dom, but is not releant to us
    js = f"""(() => {{
    const h = new Headers();
    h.append({_js_str(key)}, {_js_str(val1)});
    h.append({_js_str(key)}, {_js_str(val2)});
    return h.has({_js_str(key)});
    }})()"""
    result = runtime.eval(js)
    assert result is True
