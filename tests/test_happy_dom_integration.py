import pytest

from htmxclient.runtime import build_runtime

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


async def test_script_with_data_uri_src_executed(runtime):
    # Buffer.from(data, "ascii") from a data: URI must decode to real bytes,
    # not zero-filled garbage, or the fetched script source is empty/invalid.
    result = runtime.eval("""
        const src = 'data:text/javascript,' + encodeURIComponent('window.__ran = 1;');
        document.head.innerHTML = `<script src="${src}"></script>`;
        window.__ran;
    """)
    assert result == 1


async def test_script_with_external_file_src_executed(snapshot, tmp_path):
    (tmp_path / "external-script.js").write_text("window.__ran = 1;")
    runtime = await build_runtime(
        snapshot=snapshot,
        virtual_servers=[{"url": "http://localhost/ext/", "directory": str(tmp_path)}],
    )
    result = runtime.eval("""
        document.head.innerHTML = '<script src="http://localhost/ext/external-script.js"></script>';
        window.__ran;
    """)
    assert result == 1
