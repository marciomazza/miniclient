//! happy-dom behaviour mini leans on: `<script>` runs when its host is inserted into a
//! connected tree, `<script src>` fetches (`data:` URI and virtual-server file), `window.crypto`
//! is polyfilled before happy-dom imports it, and inline `on*` attributes compile on parse.

mod common;

use common::{EvalExt, boolean, runtime};

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
        let src_js = format!(
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
        assert!(boolean(rt.eval(&src_js)), "{insertion_js}");
    }
}

#[test]
fn script_with_data_uri_src_executed() {
    // Buffer.from(data, "ascii") from a data: URI must decode to real bytes, not zero-filled
    // garbage, or the fetched script source is empty/invalid.
    let rt = runtime();
    assert!(boolean(rt.eval(
        r#"
        const src = 'data:text/javascript,' + encodeURIComponent('window.__ran = 1;');
        document.head.innerHTML = `<script src="${src}"></script>`;
        window.__ran === 1
    "#,
    )));
}

#[test]
fn script_with_external_file_src_executed() {
    let dir = std::env::temp_dir().join(format!("miniclient-ext-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("external-script.js"), b"window.__ran = 1;").unwrap();
    let servers = format!(
        r#"[{{"url": "http://localhost/ext/", "directory": {:?}}}]"#,
        dir.to_str().unwrap()
    );
    let rt = Runtime::new("http://localhost/", &servers);
    let ran = boolean(rt.eval(
        r#"
        document.head.innerHTML =
          '<script src="http://localhost/ext/external-script.js"></script>';
        window.__ran === 1;
    "#,
    ));
    std::fs::remove_dir_all(&dir).ok();
    assert!(ran);
}

#[test]
fn window_crypto_is_available() {
    // window.crypto used to be undefined: happy-dom resolves it via `import { webcrypto } from
    // 'crypto'`, which node-crypto.js forwards from globalThis.crypto -- unset unless
    // pre_globals.js polyfills it before happy-dom is imported.
    let rt = runtime();
    assert!(boolean(rt.eval(
        r#"
        const uuid = window.crypto.randomUUID();
        window.crypto === globalThis.crypto
            && window.crypto.getRandomValues(new Uint8Array(4)).length === 4
            && /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(uuid)
    "#,
    )));
}

#[test]
fn inline_event_handler_attribute_is_compiled() {
    // Parsing an element with an inline handler attribute compiles it via
    // window[PropertySymbol.evaluateScript], where `window` is read off the element as the
    // real Window instance -- a missing defaultView used to throw here.
    let rt = runtime();
    assert!(boolean(rt.eval(
        r#"
        document.body.innerHTML = '<button onclick="window.__ran = 1">hi</button>';
        true
    "#,
    )));
}
