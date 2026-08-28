// The wheel ships prebuilt JS: happy-dom bundled by esbuild, plus the npm files read at
// runtime copied into the package as _vendor/. Cargo runs this on every build, and maturin
// drives cargo -- so there is nowhere else to hook this.
use std::path::{Path, PathBuf};
use std::process::Command;

// maturin's `include` cannot remap paths, so the copy into the package happens here and
// maturin picks the package dir up as-is. Narrower than the sdist list in pyproject.toml:
// the wheel ships a prebuilt bundle, so it needs happy-dom's licence but not its sources.
const VENDORED: &[&str] = &[
    "happy-dom/LICENSE",
    "xpath/xpath.js",
    "xpath/LICENSE",
    "entities/dist/esm",
    "entities/LICENSE",
];

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
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let js = root.join("python/miniclient/js");
    let node_modules = root.join("node_modules");

    println!("cargo::rerun-if-changed=package-lock.json");
    watch_js_sources(&js);

    if !node_modules.join(".package-lock.json").exists() {
        run("npm", &["ci"], &root);
    }
    run(
        "node",
        &[js.join("build-happydom-bundle.mjs").to_str().unwrap()],
        &root,
    );

    let vendor = root.join("python/miniclient/_vendor");
    for rel in VENDORED {
        copy_tree(&node_modules.join(rel), &vendor.join(rel));
    }

    build_default_snapshot(
        &node_modules,
        &js,
        &PathBuf::from(std::env::var("OUT_DIR").unwrap()),
    );
}

/// Builds mini's default V8 snapshot at build time and embeds it in the `.so`.
///
/// deno_core 0.410 records deno_web / deno_webidl's ESM only as build-machine disk paths,
/// read during `create_snapshot`. A wheel that ran `create_snapshot` at runtime would try to
/// read the build box's `/root/.cargo` and panic with EACCES. Building here -- where those
/// files exist -- and shipping the blob means the wheel never calls `create_snapshot`.
///
/// The script list mirrors `get_snapshot_scripts()` in `python/miniclient/runtime.py` and
/// `production_scripts()` in `src/snapshot.rs`; keep the three in sync.
fn build_default_snapshot(node_modules: &Path, js: &Path, out_dir: &Path) {
    let read = |p: &Path| {
        std::fs::read_to_string(p).unwrap_or_else(|e| panic!("snapshot input {p:?}: {e}"))
    };
    let xpath = read(&node_modules.join("xpath/xpath.js"));
    let scripts: Vec<(String, String)> = vec![
        (
            "xpath".into(),
            format!(
                "const __xpathLib = {{}};\n(function(exports){{{xpath}}})(__xpathLib);\nglobalThis.__xpathLib = __xpathLib;"
            ),
        ),
        ("pre_globals".into(), read(&js.join("pre_globals.js"))),
        ("formdata".into(), read(&js.join("formdata.js"))),
        (
            "element-registry".into(),
            read(&js.join("element_registry.js")),
        ),
        ("submit".into(), read(&js.join("submit.js"))),
        (
            "happy-dom-bundle".into(),
            read(&js.join("_generated/happy-dom-bundle.js")),
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
            ],
            extension_transpiler: None,
            with_runtime_cb: Some(Box::new(move |rt| {
                for (name, source) in &scripts {
                    rt.execute_script(name.clone(), source.clone())
                        .unwrap_or_else(|e| panic!("snapshot script `{name}` failed: {e}"));
                }
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
