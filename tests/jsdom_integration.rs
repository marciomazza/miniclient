//! Engine behaviour mini leans on: `<script>` runs when its host is inserted into a connected
//! tree, `<script src>` fetches (`data:` URI and resource-mount file), `window.crypto` is
//! polyfilled before jsdom imports it, and inline `on*` attributes compile on parse.
#![allow(clippy::unwrap_used)]

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
fn strict_script_declares_globals() {
    // A classic script is global code even under "use strict", unlike a strict eval.
    let rt = runtime();
    let js = r#"
        const script = document.createElement('script');
        script.textContent = "'use strict'; function strictFn() {} var strictVar = 1;";
        document.body.append(script);
        typeof strictFn + ' ' + typeof strictVar;
    "#;
    assert_eq!(rt.eval::<String>(js), "function number");
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

fn mounted_fetch_status(rt: &Runtime, path: &str) -> u16 {
    rt.eval_promise::<u16>(&format!(
        "fetch('http://localhost/ext/{path}').then(r => r.status)"
    ))
}

#[test]
fn mounted_file_ignores_query_string() {
    let rt = runtime_with_mounted_file("query.js", "1;");
    assert_eq!(mounted_fetch_status(&rt, "query.js?v=3"), 200);
}

#[test]
fn mounted_file_name_is_percent_decoded() {
    let rt = runtime_with_mounted_file("my app.js", "1;");
    assert_eq!(mounted_fetch_status(&rt, "my%20app.js"), 200);
}

#[test]
fn mounted_path_cannot_escape_via_encoded_parent_segment() {
    let rt = runtime_with_mounted_file("inside.js", "1;");
    std::fs::write(
        std::env::temp_dir().join("miniclient-outside.txt"),
        "secret",
    )
    .unwrap();
    let status = mounted_fetch_status(&rt, "..%2Fminiclient-outside.txt");
    std::fs::remove_file(std::env::temp_dir().join("miniclient-outside.txt")).ok();
    assert_eq!(status, 404);
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
        undefined; // close() returns a completion promise that `run` cannot marshal
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

/// Mounts `prefix -> dir` for each pair, with `dir/file` written into the given dir. Any URL no
/// mount serves goes to a backend that fails it.
fn runtime_with_mounts(mounts: &[(&str, &str, &str)]) -> Runtime {
    let root = std::env::temp_dir().join(format!("miniclient-mounts-{}", std::process::id()));
    let entries: Vec<String> = mounts
        .iter()
        .map(|(prefix, dir, file)| {
            let dir = root.join(dir);
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"1;").unwrap();
            format!("{prefix:?}: {:?}", dir.to_str().unwrap())
        })
        .collect();
    let rt = Runtime::new(&format!("{{{}}}", entries.join(", ")));
    rt.send_install_fetch_backend(std::sync::Arc::new(
        common::fetch_mock::MockFetchBackend::new(),
    ))
    .blocking_recv()
    .expect("fetch backend installed");
    rt
}

fn fetch_status(rt: &Runtime, url: &str) -> u16 {
    rt.eval_promise::<u16>(&format!("fetch('{url}').then(r => r.status, () => 0)"))
}

#[test]
fn mount_prefix_matches_whole_path_segments() {
    let rt = runtime_with_mounts(&[("http://localhost/static", "segments", "foo/a.js")]);
    assert_eq!(fetch_status(&rt, "http://localhost/static/foo/a.js"), 200);
    assert_ne!(fetch_status(&rt, "http://localhost/staticfoo/a.js"), 200);
}

#[test]
fn nested_mount_wins_over_its_parent() {
    let rt = runtime_with_mounts(&[
        ("http://localhost/a/", "parent", "other.js"),
        ("http://localhost/a/b/", "nested", "x.js"),
    ]);
    assert_eq!(fetch_status(&rt, "http://localhost/a/b/x.js"), 200);
}
