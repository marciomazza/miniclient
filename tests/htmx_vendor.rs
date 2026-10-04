mod common;

use std::path::Path;

use common::htmx_fetch_mock::HtmxFetchMock;
use common::{EvalExt, Runtime};

const VENDOR_HTMX_SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/htmx/src");
const HTMX_TEST_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/htmx/test");

/// Not relevant to this runtime -- asserts htmx has no `package.json` dependencies.
const SKIP_FILES: &[&str] = &["package.js"];

/// (file stem, suite, test name) exempted from `runner.js`'s timer scaling -- real
/// elapsed-time assertions or a guard racing unscaled async work. Ported from
/// `test_htmx.py`'s `_UNSCALED_TESTS`.
const UNSCALED_TESTS: &[(&str, &str, &str)] = &[
    (
        "timeout",
        "timeout() unit tests",
        "returns promise that resolves after milliseconds",
    ),
    (
        "timeout",
        "timeout() unit tests",
        "accepts string time format",
    ),
    ("timeout", "timeout() unit tests", "accepts seconds format"),
    (
        "hx-swap",
        "hx-swap modifiers",
        "main swap with delay respects blocking behavior",
    ),
    (
        "hx-ws",
        "Message Sending",
        "queues a message until the initial connection opens",
    ),
    (
        "hx-live",
        "hx-live extension",
        "debounce(ms) supersedes prior calls",
    ),
    (
        "hx-live",
        "hx-live extension",
        "debounce(ms, fn) runs the closure after the delay",
    ),
    (
        "hx-live",
        "hx-live extension",
        "debounce(ms, fn) supersedes across separate hx-on events on the same element",
    ),
];

/// Tests whose `htmx.timeout()` waits need slack beyond the timers they exercise.
const TIMER_SLACK_TESTS: &[(&str, &str, &str)] = &[(
    "hx-ws",
    "Error Handling and Reconnection",
    "reconnects after every default close code",
)];

/// (file stem, suite, test name) rewritten to `it.skip(...)` before the file runs -- not just
/// filtered from results, since some failures are fatal to the whole `eval_promise` call and
/// never produce a result to filter. Ported from `test_htmx.py`'s `_SKIP_TESTS`.
const SKIP_TESTS: &[(&str, &str, &str)] = &[(
    "hx-swap",
    "hx-swap modifiers",
    "swap with scroll:bottom modifier scrolls to bottom",
)];

struct TestFailure {
    file: String,
    suite: String,
    name: String,
    error: String,
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn harness_js() -> String {
    let chai = read(&format!(
        "{}/node_modules/chai/chai.js",
        env!("CARGO_MANIFEST_DIR")
    ));
    let bridge = read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/htmx_fetch_mock_bridge.js"
    ));
    let runner = read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/runner.js"));
    // Trailing `void 0`: these files are loaded purely for effect, and `EvalExt::run` wraps
    // every script in a `{ }` block whose own completion value (some intermediate assignment's
    // right-hand side, e.g. a bound function) would otherwise fail marshaling.
    format!(
        "{chai};\n\
         globalThis.assert = globalThis.chai.assert;\n\
         globalThis.should = globalThis.chai.should();\n\
         {bridge};\n\
         {runner};\n\
         void 0;"
    )
}

fn run_file(js_file: &Path) -> Vec<TestFailure> {
    let stem = js_file
        .file_stem()
        .expect("a .js file has a stem")
        .to_str()
        .expect("vendor htmx filenames are valid UTF-8")
        .to_string();

    let mounts = format!(
        r#"{{"http://localhost/vendor/": {VENDOR_HTMX_SRC:?},
            "http://localhost/test/": {HTMX_TEST_ROOT:?}}}"#
    );
    let rt = Runtime::new(&mounts);
    let mock = HtmxFetchMock::new();
    rt.send_install_fetch_backend(std::sync::Arc::new(mock.clone()))
        .blocking_recv()
        .expect("fetch backend installed");

    // document.write's script is parser-inserted, so it fetches and executes automatically.
    rt.run_promise(
        r#"
        const loaded = new Promise(resolve =>
          window.addEventListener('load', resolve, {once: true}),
        );
        document.open();
        document.write('<script src="http://localhost/vendor/htmx.js"></script>');
        document.close();
        loaded.then(() => {
          document.body.innerHTML = '<div id="test-playground"></div>';
        });
    "#,
    );
    rt.run(&harness_js());
    mock.install(&rt);
    rt.run(&format!(
        "{}\nvoid 0;",
        read(&format!("{HTMX_TEST_ROOT}/lib/helpers.js"))
    ));

    let mut js = read(js_file.to_str().expect("vendor htmx paths are valid UTF-8"));
    // ext-style relative <script src> rewrites -- harmless no-ops for files that never use them.
    js = js.replace("'../src/ext/", "'http://localhost/vendor/ext/");
    js = js.replace("'../test/lib/", "'http://localhost/test/lib/");
    for (_, _, name) in SKIP_TESTS.iter().filter(|(file, _, _)| *file == stem) {
        js = js.replace(&format!("it('{name}'"), &format!("it.skip('{name}'"));
    }
    js.push_str("\nvoid 0;");

    let unscaled: Vec<String> = UNSCALED_TESTS
        .iter()
        .filter(|(file, _, _)| *file == stem)
        .map(|(_, suite, name)| format!("{suite}::{name}"))
        .collect();
    let timer_slack: Vec<String> = TIMER_SLACK_TESTS
        .iter()
        .filter(|(file, _, _)| *file == stem)
        .map(|(_, suite, name)| format!("{suite}::{name}"))
        .collect();
    rt.run(&format!(
        "globalThis.__unscaledTests = new Set({});\n\
         globalThis.__timerSlackTests = new Set({});\n\
         void 0",
        serde_json::to_string(&unscaled).expect("unscaled test names are always JSON-safe"),
        serde_json::to_string(&timer_slack).expect("timer-slack test names are always JSON-safe"),
    ));

    rt.run(&js);
    let results: Vec<serde_json::Value> = rt.eval_promise("__runAllTests()");

    results
        .into_iter()
        .filter(|r| !r["passed"].as_bool().unwrap_or(false))
        .map(|r| TestFailure {
            file: stem.clone(),
            suite: r["suite"].as_str().unwrap_or_default().to_string(),
            name: r["name"].as_str().unwrap_or_default().to_string(),
            error: r["error"].as_str().unwrap_or_default().to_string(),
        })
        .collect()
}

fn run_dir(pattern: &str, skip_files: &[&str]) {
    let path_pattern = format!("{HTMX_TEST_ROOT}/tests/{pattern}");
    let mut files: Vec<_> = glob::glob(&path_pattern)
        .unwrap_or_else(|e| panic!("{path_pattern}: {e}"))
        .map(|entry| entry.unwrap_or_else(|e| panic!("{path_pattern}: {e}")))
        .filter(|path| {
            let name = path
                .file_name()
                .expect("glob entries have a filename")
                .to_str()
                .expect("vendor htmx filenames are valid UTF-8");
            !SKIP_FILES.contains(&name) && !skip_files.contains(&name)
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no test files found for {pattern}");

    let mut failures = Vec::new();
    for file in &files {
        failures.extend(run_file(file));
    }

    if !failures.is_empty() {
        let lines: Vec<String> = failures
            .iter()
            .map(|f| format!("  [{}] {} :: {}: {}", f.file, f.suite, f.name, f.error))
            .collect();
        panic!(
            "{} htmx {pattern} JS test(s) failed:\n{}",
            failures.len(),
            lines.join("\n")
        );
    }
}

// DataTransfer, innerText, and form-associated custom elements are polyfilled in bootstrap.js;
// chai's `should` was never actually broken -- those failures were a downstream effect of
// missing innerText.
#[test]
fn htmx_vendor_unit_suite() {
    run_dir("unit/*.js", &["__issueRequest.js"]);
}

#[test]
fn htmx_vendor_unit_issue_request_suite() {
    run_dir("unit/__issueRequest.js", &[]);
}

#[test]
fn htmx_vendor_attributes_suite() {
    run_dir("attributes/*.js", &[]);
}

#[test]
fn htmx_vendor_end2end_suite() {
    run_dir("end2end/*.js", &[]);
}

#[test]
fn htmx_vendor_ext_suite() {
    run_dir("ext/*.js", &["hx-sse.js", "hx-ws.js"]);
}

#[test]
fn htmx_vendor_ext_sse_suite() {
    run_dir("ext/hx-sse.js", &[]);
}

#[test]
fn htmx_vendor_ext_ws_suite() {
    run_dir("ext/hx-ws.js", &[]);
}
