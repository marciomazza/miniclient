"""Tests for happy-dom behavioral quirks patched in happydom_tweaks.js."""


async def test_innerhtml_radio_mutual_exclusion(runtime):
    # happy-dom bug: multiple checked radios in same name group — only last should stay checked
    runtime.eval("""
        document.body.innerHTML =
            '<input type="radio" name="g" checked><input type="radio" name="g" checked>';
    """)
    result = runtime.eval("[...document.querySelectorAll('input[type=radio]')].map(r => r.checked)")
    assert result == [False, True]


async def test_innerhtml_selected_reflected(runtime):
    # happy-dom bug: `selected` HTML attribute not reflected to .selected IDL property via innerHTML
    runtime.eval("""
        document.body.innerHTML =
            '<select><option>a</option><option selected>b</option></select>';
    """)
    assert runtime.eval("document.querySelectorAll('option')[1].selected") is True
