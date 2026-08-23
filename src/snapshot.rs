use deno_core::snapshot::{CreateSnapshotOptions, create_snapshot as deno_create_snapshot};

use crate::runtime::{extensions, init_platform, lifecycle_lock};

/// Runs `scripts` in order (warmup last, if given) into a single isolate and serializes it.
///
/// `warmup` is appended as an ordinary script rather than passed to deno_core's own
/// `warmup_script` parameter: that parameter restores the cold snapshot into a *second*
/// isolate, runs the script there, and reserializes -- and that restore-then-reserialize
/// round trip corrupts a snapshot that also used `Deno.core.loadExtScript()` (as
/// `pre_globals.js` does for the console), reliably segfaulting V8 on the next ordinary
/// deserialize. See `zz/deno-core-issue-draft-loadextscript-warmup-snapshot-segfault.md` for
/// the upstream report and minimal repro. Running warmup content in the same single pass
/// still exercises and precompiles it, without ever triggering that path.
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
    // happy-dom bundle are not.
    let scripts: Vec<_> = scripts
        .into_iter()
        .chain(warmup.map(|w| ("warmup".to_string(), w)))
        .collect();
    let output = deno_create_snapshot(
        CreateSnapshotOptions {
            cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
            startup_snapshot: None,
            skip_op_registration: false,
            extensions: extensions(),
            extension_transpiler: None,
            with_runtime_cb: Some(Box::new(move |js| {
                for (name, source) in &scripts {
                    js.execute_script(name.clone(), source.clone())
                        .unwrap_or_else(|e| panic!("snapshot script `{name}` failed: {e}"));
                }
            })),
        },
        None,
    )?;
    Ok(output.output)
}

/// Test-only helpers shared with `runtime.rs`'s tests: both need a real, bootable snapshot
/// (mini's actual production scripts), not just `snapshot.rs`'s own round-trip checks.
#[cfg(test)]
pub(crate) mod support {
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};

    /// `cargo test`'s default parallelism segfaults V8 when `create_snapshot` overlaps with
    /// concurrent isolate construction elsewhere -- test-only, since production never builds a
    /// snapshot concurrently with itself the way this binary's parallel test threads do.
    pub(crate) fn v8_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    pub(crate) fn read(rel: &str) -> String {
        std::fs::read_to_string(root().join(rel)).unwrap()
    }

    /// Mirrors `get_snapshot_scripts()` in runtime.py, which owns the canonical list.
    pub(crate) fn production_scripts() -> Vec<(String, String)> {
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

    pub(crate) fn warmup_script() -> String {
        read("python/miniclient/js/snapshot_warmup.js")
    }
}

#[cfg(test)]
mod tests {
    use deno_core::{JsRuntime, RuntimeOptions};

    use super::create_snapshot;
    use super::support::{production_scripts, v8_test_lock, warmup_script};
    use crate::runtime::extensions;

    /// Boots a snapshot and reads one expression out of it -- the only proof that a blob is
    /// restorable, not merely non-empty.
    fn eval_in_snapshot(blob: Box<[u8]>, expr: &'static str) -> String {
        let mut js = JsRuntime::new(RuntimeOptions {
            startup_snapshot: Some(Box::leak(blob)),
            extensions: extensions(),
            ..Default::default()
        });
        let value = js.execute_script("<test>", expr).unwrap();
        deno_core::scope!(scope, js);
        deno_core::v8::Local::new(scope, value).to_rust_string_lossy(scope)
    }

    #[test]
    fn production_scripts_and_warmup_produce_a_bootable_snapshot() {
        let _guard = v8_test_lock();
        let warmup = warmup_script();
        let blob = create_snapshot(production_scripts(), Some(warmup)).unwrap();
        assert_eq!(
            eval_in_snapshot(blob, "[typeof FormData, typeof __happyDomBundle].join()"),
            "function,object"
        );
    }

    /// tests/test_htmx.py's shape: the same list with extra scripts appended.
    #[test]
    fn appended_scripts_produce_a_distinct_bootable_snapshot() {
        let _guard = v8_test_lock();
        let mut scripts = production_scripts();
        scripts.push(("marker".into(), "globalThis.__marker = 'chai';".into()));
        let blob = create_snapshot(scripts, Some(warmup_script())).unwrap();
        assert_eq!(eval_in_snapshot(blob, "__marker"), "chai");
    }
}
