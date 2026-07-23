"""Tests for happy-dom fixes and tweaks in patch-happy-dom.js."""

import pytest


async def test_innerhtml_radio_mutual_exclusion(runtime):
    # happy-dom bug: multiple checked radios in same name group — only last should stay checked
    runtime.eval("""
        document.body.innerHTML =
          '<input type="radio" name="g" checked><input type="radio" name="g" checked>'
    """)
    result = runtime.eval("[...document.querySelectorAll('input[type=radio]')].map(r => r.checked)")
    assert result == [False, True]


async def test_innerhtml_selected_reflected(runtime):
    # happy-dom bug: `selected` HTML attribute not reflected to .selected IDL property via innerHTML
    runtime.eval("""
        document.body.innerHTML = '<select><option>a</option><option selected>b</option></select>'
    """)
    assert runtime.eval("document.querySelectorAll('option')[1].selected") is True


# ---------------------------------------------------------------------------
# :checked pseudo-class also matches a selected <option>
# ---------------------------------------------------------------------------


async def test_option_checked_pseudo_class_matches_selected_option(runtime):
    # happy-dom bug: :checked only matches <input>, but per spec it also applies to a
    # selected <option> — querySelectorAll (not just .matches()) must see this.
    runtime.eval("""
        document.body.innerHTML =
          '<select><option value="a">a</option><option value="b" selected>b</option></select>'
    """)
    assert runtime.eval("document.querySelectorAll('option:checked').length") == 1
    assert runtime.eval("document.querySelector('option:checked').value") == "b"


async def test_option_checked_pseudo_class_updates_after_value_assignment(runtime):
    runtime.eval("""
        document.body.innerHTML =
          '<select id="s"><option value="a">a</option><option value="b">b</option></select>';
        document.getElementById('s').value = 'b';
    """)
    assert runtime.eval("document.querySelector('option:checked').value") == "b"


async def test_input_checked_pseudo_class_still_works(runtime):
    # regression: the <option> fix must not affect <input type=checkbox/radio>
    runtime.eval("""
        document.body.innerHTML = '<input type="checkbox" checked><input type="checkbox">'
    """)
    assert runtime.eval("document.querySelectorAll('input:checked').length") == 1


async def test_select_value_setter_invalidates_stale_checked_query_cache(runtime):
    # happy-dom bug: querySelectorAll caches its result per selector string, invalidated
    # by a node's [clearCache](). HTMLInputElement's checked setter calls it; the <select>
    # value setter mutates each <option>'s selectedness directly and never did — so a
    # ":checked" query evaluated once (e.g. by hx-live on initial render, before anything
    # is selected) stayed stuck at its stale result even after a real selection happened.
    runtime.eval("""
        document.body.innerHTML =
          '<select id="s"><option value="a">a</option><option value="b">b</option></select>'
    """)
    # warm the querySelectorAll cache for this exact selector string while nothing is selected
    assert runtime.eval("document.querySelectorAll(\"option[value='b']:checked\").length") == 0
    runtime.eval("document.getElementById('s').value = 'b'")
    # same selector string as above — must reflect the new selection, not the stale cached result
    assert runtime.eval("document.querySelectorAll(\"option[value='b']:checked\").length") == 1


# ---------------------------------------------------------------------------
# history.pushState / replaceState update location
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("method", ["pushState", "replaceState"])
async def test_history_state_updates_location(runtime, method):
    runtime.eval(f"history.{method}(null, '', '/new-path')")
    assert runtime.eval("location.pathname") == "/new-path"


@pytest.mark.parametrize(
    "url, expected_pathname, expected_search",
    [
        ("/page", "/page", ""),
        ("/page?q=1", "/page", "?q=1"),
        ("/a/b/c", "/a/b/c", ""),
    ],
)
async def test_history_pushstate_url_parts(runtime, url, expected_pathname, expected_search):
    runtime.eval(f"history.pushState(null, '', {url!r})")
    assert runtime.eval("location.pathname") == expected_pathname
    assert runtime.eval("location.search") == expected_search


# ---------------------------------------------------------------------------
# Element.matches :disabled propagated from <fieldset disabled>
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("tag", ["input", "button", "select", "textarea"])
async def test_disabled_propagates_from_fieldset(runtime, tag):
    runtime.eval(f"""\
        document.body.innerHTML = "<fieldset disabled><{tag} id='x'></{tag}></fieldset>"
    """)
    assert runtime.eval("document.querySelector('#x').matches(':disabled')") is True


async def test_non_disabled_fieldset_does_not_disable_children(runtime):
    runtime.eval("""\
        document.body.innerHTML = "<fieldset><input id='x'></fieldset>"
    """)
    assert runtime.eval("document.querySelector('#x').matches(':disabled')") is False


# ---------------------------------------------------------------------------
# HTMLElement.attachInternals polyfill
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "value_js, expected",
    [
        ("'hello'", "hello"),
        ("'42'", "42"),
        ("null", None),  # null clears the value
    ],
)
async def test_attach_internals_set_form_value(runtime, value_js, expected):
    result = runtime.eval(f"""
        const el = document.createElement('div');
        const internals = el.attachInternals();
        internals.setFormValue({value_js});
        el.__internalsFormValue;
    """)
    assert result == expected


# ---------------------------------------------------------------------------
# document.getElementById respects tree order with duplicate IDs
# ---------------------------------------------------------------------------


async def test_getelementbyid_returns_first_in_tree_order(runtime):
    # Simulate htmx pantry: a second element with same ID appended after <body>
    runtime.eval("""
        document.body.innerHTML = '<div id="x">first</div>';
        const extra = document.createElement('div');
        extra.id = 'x';
        extra.textContent = 'second';
        document.documentElement.appendChild(extra);
    """)
    assert runtime.eval("document.getElementById('x').textContent") == "first"


# ---------------------------------------------------------------------------
# EventTarget.dispatchEvent sets globalThis.event
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("event_type", ["click", "input", "change", "custom-event"])
async def test_dispatch_event_sets_global_event(runtime, event_type):
    result = runtime.eval(f"""
        const el = document.createElement('div');
        let captured = null;
        el.addEventListener('{event_type}', () => {{
          captured = globalThis.event;
        }});
        const evt = new Event('{event_type}');
        el.dispatchEvent(evt);
        captured === evt;
    """)
    assert result is True


async def test_dispatch_event_restores_global_event_after(runtime):
    result = runtime.eval("""
        const el = document.createElement('div');
        el.addEventListener('click', () => {});
        globalThis.event = 'sentinel';
        el.dispatchEvent(new Event('click'));
        globalThis.event;
    """)
    assert result == "sentinel"
