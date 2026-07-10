import pytest


# ---------------------------------------------------------------------------
# <script> elements executed on DOM insertion
# ---------------------------------------------------------------------------
# fixme: This test is probably not necessary anymore since we started to use native happy-dom script
# evaluation. In other words, this is just testing Happy-dom itself.
@pytest.mark.parametrize(
    "js",
    [
        "host.append(script)",
        "host.prepend(script)",
        "host.replaceChildren(script)",
        "host.before(script)",
        "host.after(script)",
        "host.insertBefore(script, null)",
        "host.replaceWith(script)",
    ],
    ids=["append", "prepend", "replaceChildren", "before", "after", "insertBefore", "replaceWith"],
)
async def test_script_executed_on_dom_insertion(runtime, js):
    result = runtime.eval(f"""
        const script = document.createElement('script');
        script.textContent = 'window.__ran = 1';
        const host = document.createElement('div');
        document.body.append(host); // host must be connected
        {js};
        window.__ran;
    """)
    assert result == 1
