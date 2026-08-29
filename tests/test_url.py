"""Pinning tests for URL / URLSearchParams behaviour.

These formerly guarded patch-happy-dom-url.js; the patch is gone and deno_web's native
00_url.js provides the same behaviour. Kept as a regression guard for the swap.
"""

import pytest

# ---------------------------------------------------------------------------
# URL.searchParams mutations propagate back to url.search / url.href
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "start_url, mutation_js, expected_search",
    [
        ("http://ex.com/", "u.searchParams.set('k', 'v')", "?k=v"),
        ("http://ex.com/", "u.searchParams.append('k', 'v')", "?k=v"),
        ("http://ex.com/?k=v", "u.searchParams.delete('k')", ""),
        ("http://ex.com/?b=2&a=1", "u.searchParams.sort()", "?a=1&b=2"),
        # chained mutations
        (
            "http://ex.com/",
            "u.searchParams.set('a', '1'); u.searchParams.set('b', '2')",
            "?a=1&b=2",
        ),
    ],
    ids=["set", "append", "delete", "sort", "chained"],
)
async def test_searchparams_mutation_propagates_to_search(
    runtime, start_url, mutation_js, expected_search
):
    result = runtime.eval(f"""
        const u = new URL({start_url!r});
        {mutation_js};
        u.search;
    """)
    assert result == expected_search


async def test_searchparams_mutation_propagates_to_href(runtime):
    result = runtime.eval("""
        const u = new URL('http://ex.com/path');
        u.searchParams.set('k', 'v');
        u.href;
    """)
    assert result == "http://ex.com/path?k=v"


# ---------------------------------------------------------------------------
# URLSearchParams constructor accepts FormData and URLSearchParams as init
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "setup_js, init_js, expected_str",
    [
        # FormData as init — setup_js creates FormData, init_js constructs URLSearchParams from it
        (
            "const fd = new FormData(); fd.append('a', '1'); fd.append('b', '2')",
            "new URLSearchParams(fd)",
            "a=1&b=2",
        ),
        (
            "const fd = new FormData(); fd.append('x', 'y')",
            "new URLSearchParams(fd)",
            "x=y",
        ),
        # URLSearchParams as init — no setup needed
        ("", "new URLSearchParams(new URLSearchParams('a=1&b=2'))", "a=1&b=2"),
    ],
    ids=["formdata-multi", "formdata-single", "urlsearchparams"],
)
async def test_urlsearchparams_accepts_iterable_init(runtime, setup_js, init_js, expected_str):
    result = runtime.eval(f"{setup_js}; init = {init_js}; init.toString()")
    assert result == expected_str


# ---------------------------------------------------------------------------
# URL.createObjectURL / revokeObjectURL — used by the hx-download extension
# ---------------------------------------------------------------------------


async def test_create_and_revoke_object_url(runtime):
    result = runtime.eval("""
        const url = URL.createObjectURL(new Blob(['hi']));
        const ok = url.startsWith('blob:') && URL.revokeObjectURL(url) === true;
        ok;
    """)
    assert result is True
