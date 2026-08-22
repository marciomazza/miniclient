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
    "text-encoding/lib/encoding.js",
    "text-encoding/LICENSE.md",
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
}
