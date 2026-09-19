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
    // document.write's script is parser-inserted, so it fetches and executes automatically.
    let rt = runtime();
    let ran = rt.eval_async::<bool>(
        r#"
        const loaded = new Promise(resolve =>
          window.addEventListener('load', resolve, {once: true}),
        );
        document.open();
        document.write('<script src="data:text/javascript,window.__ran=1"></script>');
        document.close();
        loaded.then(() => window.__ran === 1);
    "#,
    );
    assert!(ran);
}

#[test]
fn script_with_external_file_src_executed() {
    let dir = std::env::temp_dir().join(format!("miniclient-ext-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("external-script.js"), b"window.__ran = 1;").unwrap();
    let mounts = format!(
        r#"{{"http://localhost/ext/": {:?}}}"#,
        dir.to_str().unwrap()
    );
    let rt = Runtime::new("http://localhost/", &mounts);
    let ran = rt.eval_async::<bool>(
        r#"
        const loaded = new Promise(resolve =>
          window.addEventListener('load', resolve, {once: true}),
        );
        document.open();
        document.write('<script src="http://localhost/ext/external-script.js"></script>');
        document.close();
        loaded.then(() => window.__ran === 1);
    "#,
    );
    std::fs::remove_dir_all(&dir).ok();
    assert!(ran);
}

#[test]
fn dynamically_inserted_defer_script_loads() {
    let dir = std::env::temp_dir().join(format!("miniclient-defer-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("deferred-script.js"), b"window.__ran = 1;").unwrap();
    let mounts = format!(
        r#"{{"http://localhost/ext/": {:?}}}"#,
        dir.to_str().unwrap()
    );
    let rt = Runtime::new("http://localhost/", &mounts);
    let loaded = rt.eval_async::<bool>(
        r#"
        new Promise((resolve, reject) => {
            const script = document.createElement('script');
            script.defer = true;
            script.src = 'http://localhost/ext/deferred-script.js';
            script.onload = () => resolve(window.__ran === 1);
            script.onerror = () => reject(new Error('deferred script failed to load'));
            document.head.append(script);
        });
    "#,
    );
    std::fs::remove_dir_all(&dir).ok();
    assert!(loaded);
}

#[test]
fn window_crypto_is_available() {
    // window.crypto used to be undefined: jsdom resolves it via `require('crypto')`'s
    // webcrypto, which node-crypto.js forwards from globalThis.crypto -- unset unless
    // pre_globals.js polyfills it before jsdom is imported.
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
