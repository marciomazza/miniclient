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
    let ran = rt.eval_promise::<bool>(
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
    let rt = Runtime::new(&mounts);
    let ran = rt.eval_promise::<bool>(
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

/// Writes `dir/name` with `contents`, mounted at `http://localhost/ext/`.
fn runtime_with_mounted_file(name: &str, contents: &str) -> Runtime {
    let dir = std::env::temp_dir().join(format!(
        "miniclient-mount-{}-{}",
        std::process::id(),
        name.replace(['.', '/'], "_")
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), contents.as_bytes()).unwrap();
    let mounts = format!(
        r#"{{"http://localhost/ext/": {:?}}}"#,
        dir.to_str().unwrap()
    );
    Runtime::new(&mounts)
}

#[test]
fn dynamically_inserted_async_script_loads() {
    let rt = runtime_with_mounted_file("async-script.js", "window.__ran = 1;");
    let loaded = rt.eval_promise::<bool>(
        r#"
        new Promise((resolve, reject) => {
            const script = document.createElement('script');
            script.async = true;
            script.src = 'http://localhost/ext/async-script.js';
            script.onload = () => resolve(window.__ran === 1);
            script.onerror = () => reject(new Error('async script failed to load'));
            document.head.append(script);
        });
    "#,
    );
    assert!(loaded);
}

#[test]
fn parser_inserted_module_script_executes() {
    // document.write's script is parser-inserted, so it fetches and executes automatically.
    let rt = runtime_with_mounted_file("entry.js", "window.__ran = 1;");
    let ran = rt.eval_promise::<bool>(
        r#"
        const loaded = new Promise(resolve =>
          window.addEventListener('load', resolve, {once: true}),
        );
        document.open();
        document.write('<script type="module" src="http://localhost/ext/entry.js"></script>');
        document.close();
        loaded.then(() => window.__ran === 1);
    "#,
    );
    assert!(ran);
}

/// Known gap: jsdom does not resolve a relative `import` inside an external module script
/// under this runtime, so the import never completes and the importing script's own top-level
/// code never runs either.
#[test]
fn module_script_relative_import_does_not_execute() {
    let dir = std::env::temp_dir().join(format!("miniclient-relimport-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("helper.js"), b"export const value = 1;").unwrap();
    std::fs::write(
        dir.join("entry.js"),
        b"import { value } from './helper.js'; window.__ran = value;",
    )
    .unwrap();
    let mounts = format!(
        r#"{{"http://localhost/ext/": {:?}}}"#,
        dir.to_str().unwrap()
    );
    let rt = Runtime::new(&mounts);
    let ran = rt.eval_promise::<bool>(
        r#"
        new Promise((resolve) => {
            const script = document.createElement('script');
            script.type = 'module';
            script.src = 'http://localhost/ext/entry.js';
            script.onload = () => resolve(window.__ran === 1);
            script.onerror = () => resolve(false);
            document.head.append(script);
            // Neither onload nor onerror fires for this gap -- fall back after a beat.
            setTimeout(() => resolve(window.__ran === 1), 200);
        });
    "#,
    );
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        !ran,
        "relative import inside a module script unexpectedly executed"
    );
}

/// A `type="module"` script runs in strict mode with its own lexical scope -- top-level
/// `var`/`function` don't leak to `globalThis`, and `this` is `undefined` both at the module's
/// top level and inside a plain function it calls.
#[test]
fn module_script_scope_and_strictness() {
    let rt = runtime();
    // The script does not return a promise, so run the simpler synchronous eval.
    rt.run(
        r#"
        document.open();
        document.write(`<script type="module">
        var leaked = 1;
        function hoisted() { return 2; }
        window.__mod = { self: this, strict: (function () { return this; })(), h: hoisted() };
        try { undeclared = 1; window.__mod.err = "none"; }
        catch (e) { window.__mod.err = e.name; }
        </script>`);
        document.close();
    "#,
    );
    assert_eq!(
        rt.eval::<String>("typeof leaked + typeof hoisted"),
        "undefinedundefined"
    );
    assert!(rt.eval::<bool>("__mod.self === undefined && __mod.strict === undefined"));
    assert_eq!(rt.eval::<i64>("__mod.h"), 2);
    assert_eq!(rt.eval::<String>("__mod.err"), "ReferenceError");
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
