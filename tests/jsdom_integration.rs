//! Engine behaviour mini leans on: `<script>` runs when its host is inserted into a connected
//! tree, `<script src>` fetches (`data:` URI and resource-mount file), `window.crypto` is
//! polyfilled before jsdom imports it, and inline `on*` attributes compile on parse.

mod common;

use common::{EvalExt, runtime};

use _miniclient::runtime::Runtime;

#[test]
fn script_executed_on_dom_insertion() {
    let rt = runtime();
    for insertion_js in [
        "host.append(script)",
        "host.prepend(script)",
        "host.replaceChildren(script)",
        "host.before(script)",
        "host.after(script)",
        "host.insertBefore(script, null)",
        "host.replaceWith(script)",
    ] {
        let js = format!(
            r#"
            window.__ran = 0;
            const script = document.createElement('script');
            script.textContent = 'window.__ran = 1';
            const host = document.createElement('div');
            document.body.append(host); // host must be connected
            {insertion_js};
            window.__ran === 1;
        "#
        );
        assert!(rt.eval::<bool>(&js), "{insertion_js}");
    }
}

#[test]
fn script_with_data_uri_src_executed() {
    // Buffer.from(data, "ascii") from a data: URI must decode to real bytes, not zero-filled
    // garbage, or the fetched script source is empty/invalid.
    //
    // The injection and the read are two separate evals, not one: `<script src>` loads over a
    // chain of microtasks (the resource-mount interceptor is itself async), so a completion
    // value computed in the same synchronous script as the insertion always observes the
    // pre-load state. run_async only pumps the event loop while its own completion value is a
    // pending Promise (deno_core skips pumping entirely for a non-Promise value), so the
    // injection must itself await the script's load/error event.
    //
    // Inserted via createElement+appendChild, not innerHTML: a script parsed by the fragment
    // parsing algorithm (innerHTML/outerHTML) is spec-flagged "already started" and never
    // fetches at all -- true in a real browser too, not a mini-specific gap.
    let rt = runtime();
    rt.run_async(
        r#"
        (async () => {
            const src = 'data:text/javascript,' + encodeURIComponent('window.__ran = 1;');
            const s = document.createElement('script');
            s.src = src;
            const loaded = new Promise((resolve) => {
                s.addEventListener('load', resolve, { once: true });
                s.addEventListener('error', resolve, { once: true });
            });
            document.head.appendChild(s);
            await loaded;
        })()
    "#,
    );
    assert!(rt.eval::<bool>("window.__ran === 1"));
}

#[test]
fn script_with_external_file_src_executed() {
    let dir = std::env::temp_dir().join(format!("miniclient-ext-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("external-script.js"), b"window.__ran = 1;").unwrap();
    let mounts = format!(r#"{{"http://localhost/ext/": {:?}}}"#, dir.to_str().unwrap());
    let rt = Runtime::new("http://localhost/", &mounts);
    rt.run_async(
        r#"
        (async () => {
            const s = document.createElement('script');
            s.src = 'http://localhost/ext/external-script.js';
            const loaded = new Promise((resolve) => {
                s.addEventListener('load', resolve, { once: true });
                s.addEventListener('error', resolve, { once: true });
            });
            document.head.appendChild(s);
            await loaded;
        })()
    "#,
    );
    let ran = rt.eval::<bool>("window.__ran === 1");
    std::fs::remove_dir_all(&dir).ok();
    assert!(ran);
}

#[test]
fn window_crypto_is_available() {
    // window.crypto used to be undefined: happy-dom resolves it via `import { webcrypto } from
    // 'crypto'`, which node-crypto.js forwards from globalThis.crypto -- unset unless
    // pre_globals.js polyfills it before happy-dom is imported.
    let rt = runtime();
    assert!(rt.eval::<bool>(
        r#"
        const uuid = window.crypto.randomUUID();
        window.crypto === globalThis.crypto
            && window.crypto.getRandomValues(new Uint8Array(4)).length === 4
            && /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(uuid)
    "#,
    ));
}

#[test]
fn inline_event_handler_attribute_is_compiled() {
    // Parsing an element with an inline handler attribute compiles it via
    // window[PropertySymbol.evaluateScript], where `window` is read off the element as the
    // real Window instance -- a missing defaultView used to throw here.
    let rt = runtime();
    assert!(rt.eval::<bool>(
        r#"
        document.body.innerHTML = '<button onclick="window.__ran = 1">hi</button>';
        true
    "#,
    ));
}
