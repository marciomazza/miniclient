import json
import re

import pytest

from miniclient.runtime import open_runtime

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


async def test_script_with_external_file_src_executed(v8_snapshot, tmp_path):
    (tmp_path / "external-script.js").write_text("window.__ran = 1;")
    async with open_runtime(
        v8_snapshot=v8_snapshot,
        virtual_servers=[{"url": "http://localhost/ext/", "directory": str(tmp_path)}],
    ) as runtime:
        result = runtime.eval("""
            document.head.innerHTML = '<script src="http://localhost/ext/external-script.js"></script>';
            window.__ran;
        """)
    assert result == 1


async def test_window_crypto_is_available(runtime):
    # window.crypto used to be undefined: happy-dom's own BrowserWindow resolves it via
    # `import { webcrypto } from 'crypto'`, which node-crypto.js forwards from
    # globalThis.crypto -- and jsrun provides no native one, so it stayed unset unless
    # pre_globals.js polyfills it before happy-dom is imported.
    result = runtime.eval("""
        JSON.stringify({
            sameObject: window.crypto === globalThis.crypto,
            randomLength: window.crypto.getRandomValues(new Uint8Array(4)).length,
            uuid: window.crypto.randomUUID(),
        })
    """)
    parsed = json.loads(result)
    assert parsed["sameObject"] is True
    assert parsed["randomLength"] == 4
    assert re.fullmatch(
        r"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}", parsed["uuid"]
    )


async def test_inline_event_handler_attribute_is_compiled(runtime):
    # Parsing an element with an inline event handler attribute (onclick, ...)
    # compiles it via window[PropertySymbol.evaluateScript]/[dispatchError]
    # (ElementEventAttributeUtility.getEventListener), where `window` is read off
    # as `element[...][PropertySymbol.defaultView]` -- the real Window instance.
    result = runtime.eval("""
        document.body.innerHTML = '<button onclick="window.__ran = 1">hi</button>';
        "survived";
    """)
    assert result == "survived"
