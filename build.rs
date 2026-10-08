// A build script that fails the build on any I/O error is the intended behaviour, so the
// unwrap/expect lints from Cargo.toml add nothing here.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

// maturin's `include` cannot remap paths, so the copy into the package happens here and
// maturin picks the package dir up as-is. Only licences: the wheel embeds the bundle and the
// snapshot, so it redistributes this code but never reads these files at runtime.
const VENDORED: &[&str] = &["jsdom/LICENSE.txt", "entities/LICENSE"];

fn run(program: &str, args: &[&str], cwd: &Path) {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `{program}` (is it installed?): {e}"));
    assert!(status.success(), "`{program} {}` failed", args.join(" "));
}

fn copy_tree(from: &Path, to: &Path) {
    if from.is_dir() {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        }
    } else {
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(from, to).unwrap();
    }
}

/// Watch the bundle's inputs, skipping _generated/ -- this build script writes there.
fn watch_js_sources(js: &Path) {
    for entry in std::fs::read_dir(js).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|n| n == "_generated") {
            continue;
        }
        println!("cargo::rerun-if-changed={}", path.display());
    }
}

fn main() {
    // cargo test strips DYLD_*; bake libpython's rpath so non-system Python (mise/pyenv/brew) loads.
    pyo3_build_config::add_libpython_rpath_link_args();

    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let js = root.join("js");
    let node_modules = root.join("node_modules");

    println!("cargo::rerun-if-changed=package-lock.json");
    println!("cargo::rerun-if-changed=patches");
    watch_js_sources(&js);

    if !node_modules.join(".package-lock.json").exists() {
        run("npm", &["ci"], &root);
    }
    // Idempotent: re-applies cleanly even if `npm ci` (via its postinstall) already did.
    // Needed on its own when node_modules was restored from a cache that predates a patch
    // (the `npm ci` above is skipped in that case) -- without this, an edit to patches/
    // would rebuild the bundle from a stale, unpatched jsdom and never notice.
    // `patch-package` exits 0 even when it fails to apply a patch unless told otherwise --
    // `--error-on-fail` is what makes the `assert!` in `run()` actually catch it.
    run("npx", &["patch-package", "--error-on-fail"], &root);
    run(
        "node",
        &[js.join("build-jsdom-bundle.mjs").to_str().unwrap()],
        &root,
    );

    let vendor = root.join("python/miniclient/_vendor");
    // Wipe first so a dropped entry (e.g. a swapped-out dependency) can't linger on disk.
    std::fs::remove_dir_all(&vendor).ok();
    for rel in VENDORED {
        copy_tree(&node_modules.join(rel), &vendor.join(rel));
    }

    build_default_snapshot(&js, &PathBuf::from(std::env::var("OUT_DIR").unwrap()));
}

/// Builds mini's default V8 snapshot at build time and embeds it in the `.so`.
///
/// deno_core 0.410 records deno_web / deno_webidl's ESM only as build-machine disk paths,
/// read while the snapshot is built. Doing it at runtime would try to read the build box's
/// `/root/.cargo` and panic with EACCES -- so it happens here, where those files exist, and
/// only the finished blob ships.
fn build_default_snapshot(js: &Path, out_dir: &Path) {
    let read = |p: &Path| {
        std::fs::read_to_string(p).unwrap_or_else(|e| panic!("snapshot input {p:?}: {e}"))
    };
    let scripts: Vec<(String, String)> = vec![
        ("pre_globals".into(), read(&js.join("pre_globals.js"))),
        (
            "element-registry".into(),
            read(&js.join("element_registry.js")),
        ),
        ("submit".into(), read(&js.join("submit.js"))),
        ("fetch".into(), read(&js.join("fetch.js"))),
        (
            "jsdom-bundle".into(),
            read(&js.join("_generated/jsdom-bundle.js")),
        ),
        ("warmup".into(), read(&js.join("snapshot_warmup.js"))),
    ];

    let snapshot = deno_core::snapshot::create_snapshot(
        deno_core::snapshot::CreateSnapshotOptions {
            cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
            startup_snapshot: None,
            skip_op_registration: false,
            extensions: vec![
                deno_webidl::deno_webidl::init(),
                deno_web::deno_web::init(
                    deno_web::BlobStore::default_arc(),
                    None,
                    false,
                    <_>::default(),
                ),
                deno_net::deno_net::init(None, None),
                deno_fetch::deno_fetch::init(deno_fetch::Options::default()),
            ],
            extension_transpiler: None,
            with_runtime_cb: Some(Box::new(move |rt| {
                for (name, source) in &scripts {
                    rt.execute_script(name.clone(), source.clone())
                        .unwrap_or_else(|e| panic!("snapshot script `{name}` failed: {e}"));
                }
                // Do not serialize callbacks whose host runtime will be gone after the build.
                rt.v8_isolate().perform_microtask_checkpoint();
            })),
        },
        None,
    )
    .expect("failed to build the default V8 snapshot");

    std::fs::write(out_dir.join("DEFAULT_SNAPSHOT.bin"), &snapshot.output).unwrap();
    for path in &snapshot.files_loaded_during_snapshot {
        println!("cargo::rerun-if-changed={}", path.display());
    }
}
