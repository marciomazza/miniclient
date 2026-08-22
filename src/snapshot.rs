use std::sync::atomic::{AtomicBool, Ordering};

use deno_core::snapshot::{CreateSnapshotOptions, create_snapshot as deno_create_snapshot};

use crate::runtime::{extension, init_platform, lifecycle_lock};

/// deno_core wants `&'static` for the warmup source, but ours is read from disk at call
/// time. Leaking is bounded: a snapshot is built at most a couple of times per process, and
/// the isolate holding the leaked text dies with the process anyway.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Runs `scripts` in order into a fresh isolate and serializes it. `warmup` is deno_core's
/// second pass: the cold snapshot is restored, the script is run, and the *exercised*
/// context is what gets serialized, so its lazily-compiled functions ship compiled.
pub fn create_snapshot(
    scripts: Vec<(String, String)>,
    warmup: Option<String>,
) -> Result<Box<[u8]>, deno_core::error::CoreError> {
    init_platform();
    // deno_core is explicit that a process is either snapshotting or not, and V8 in
    // snapshot mode is single-threaded: building here while another thread constructs a
    // runtime kills the process inside V8's own init. The lifecycle lock already serializes
    // exactly those two moments against each other.
    let _lock = lifecycle_lock();
    // Not `Extension::js_files`: those must be 7-bit ASCII, and both text-encoding and the
    // happy-dom bundle are not. `with_runtime_cb` runs once per pass, so the cold pass is
    // gated -- the warm pass must not replay every script over the context it just restored.
    let cold_pass = AtomicBool::new(true);
    let output = deno_create_snapshot(
        CreateSnapshotOptions {
            cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
            startup_snapshot: None,
            skip_op_registration: false,
            extensions: vec![extension()],
            extension_transpiler: None,
            with_runtime_cb: Some(Box::new(move |js| {
                if !cold_pass.swap(false, Ordering::Relaxed) {
                    return;
                }
                for (name, source) in &scripts {
                    js.execute_script(name.clone(), source.clone())
                        .unwrap_or_else(|e| panic!("snapshot script `{name}` failed: {e}"));
                }
            })),
        },
        warmup.map(leak),
    )?;
    Ok(output.output)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use deno_core::{JsRuntime, RuntimeOptions};

    use super::create_snapshot;
    use crate::runtime::extension;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn read(rel: &str) -> String {
        std::fs::read_to_string(root().join(rel)).unwrap()
    }

    /// Mirrors `get_snapshot_scripts()` in runtime.py, which owns the canonical list.
    fn production_scripts() -> Vec<(String, String)> {
        let xpath = read("node_modules/xpath/xpath.js");
        vec![
            (
                "text-encoding".into(),
                read("node_modules/text-encoding/lib/encoding.js"),
            ),
            (
                "xpath".into(),
                format!(
                    "const __xpathLib = {{}};\n(function(exports){{{xpath}}})(__xpathLib);\nglobalThis.__xpathLib = __xpathLib;"
                ),
            ),
            (
                "pre_globals".into(),
                read("python/miniclient/js/pre_globals.js"),
            ),
            ("formdata".into(), read("python/miniclient/js/formdata.js")),
            (
                "element-registry".into(),
                read("python/miniclient/js/element_registry.js"),
            ),
            ("submit".into(), read("python/miniclient/js/submit.js")),
            (
                "happy-dom-bundle".into(),
                read("python/miniclient/js/_generated/happy-dom-bundle.js"),
            ),
        ]
    }

    /// Boots a snapshot and reads one expression out of it -- the only proof that a blob is
    /// restorable, not merely non-empty.
    fn eval_in_snapshot(blob: Box<[u8]>, expr: &'static str) -> String {
        let mut js = JsRuntime::new(RuntimeOptions {
            startup_snapshot: Some(Box::leak(blob)),
            extensions: vec![extension()],
            ..Default::default()
        });
        let value = js.execute_script("<test>", expr).unwrap();
        deno_core::scope!(scope, js);
        deno_core::v8::Local::new(scope, value).to_rust_string_lossy(scope)
    }

    #[test]
    fn production_scripts_and_warmup_produce_a_bootable_snapshot() {
        let warmup = read("python/miniclient/js/snapshot_warmup.js");
        let blob = create_snapshot(production_scripts(), Some(warmup)).unwrap();
        assert_eq!(
            eval_in_snapshot(blob, "[typeof FormData, typeof __happyDomBundle].join()"),
            "function,object"
        );
    }

    /// tests/test_htmx.py's shape: the same list with extra scripts appended.
    #[test]
    fn appended_scripts_produce_a_distinct_bootable_snapshot() {
        let mut scripts = production_scripts();
        scripts.push(("marker".into(), "globalThis.__marker = 'chai';".into()));
        let warmup = read("python/miniclient/js/snapshot_warmup.js");
        let blob = create_snapshot(scripts, Some(warmup)).unwrap();
        assert_eq!(eval_in_snapshot(blob, "__marker"), "chai");
    }
}
