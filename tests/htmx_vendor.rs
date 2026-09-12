mod common;

use std::path::{Path, PathBuf};

use common::htmx_fetch_mock::HtmxFetchMock;
use common::{EvalExt, Runtime};

const VENDOR_HTMX_SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/htmx/src");
const HTMX_TEST_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/htmx/test");

/// Not relevant to this runtime -- asserts htmx has no `package.json` dependencies.
const SKIP_FILES: &[&str] = &["package.js"];

/// (file stem, suite, test name) exempted from `runner.js`'s timer scaling -- real
/// elapsed-time assertions or a guard racing unscaled async work. Ported from
/// `test_htmx.py`'s `_UNSCALED_TESTS`, trimmed to entries whose file lives in `tests/unit/`.
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
        "morph",
        "htmx processing during morph",
        "processes new htmx attributes added during innerMorph",
    ),
    (
        "morph",
        "htmx processing during morph",
        "processes new htmx attributes added during outerMorph",
    ),
];

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

    let servers = format!(
        r#"[{{"url": "http://localhost/vendor/", "directory": {VENDOR_HTMX_SRC:?}}},
            {{"url": "http://localhost/test/", "directory": {HTMX_TEST_ROOT:?}}}]"#
    );
    let rt = Runtime::new("http://localhost/", &servers);
    let mock = HtmxFetchMock::new();
    rt.send_install_fetch_backend(Box::new(mock.clone()))
        .blocking_recv()
        .expect("fetch backend installed");

    rt.run(
        r#"
        document.head.innerHTML = '<script src="http://localhost/vendor/htmx.js"></script>';
        document.body.innerHTML = '<div id="test-playground"></div>';
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
    js.push_str("\nvoid 0;");

    let unscaled: Vec<String> = UNSCALED_TESTS
        .iter()
        .filter(|(file, _, _)| *file == stem)
        .map(|(_, suite, name)| format!("{suite}::{name}"))
        .collect();
    rt.run(&format!(
        "globalThis.__unscaledTests = new Set({}); void 0",
        serde_json::to_string(&unscaled).expect("unscaled test names are always JSON-safe")
    ));

    rt.run(&js);
    let results: Vec<serde_json::Value> = rt.eval_async("__runAllTests()");

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

#[test]
fn htmx_vendor_unit_suite() {
    let dir = Path::new(HTMX_TEST_ROOT).join("tests/unit");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{dir:?}: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("js"))
        .filter(|p| {
            let name = p.file_name().expect("readdir entries have a name");
            !SKIP_FILES.contains(
                &name
                    .to_str()
                    .expect("vendor htmx filenames are valid UTF-8"),
            )
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no unit test files found under {dir:?}");

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
            "{} htmx unit JS test(s) failed:\n{}",
            failures.len(),
            lines.join("\n")
        );
    }
}
