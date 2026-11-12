//! Bundles every checklist (`data/routes/*.json`) and layout-graph file
//! (`data/layouts/ggg/*.json`) into the app: adding a file to those folders is enough.

use std::fmt::Write as _;
use std::path::Path;

/// `pub const NAME: &[(&str, &str)]`: (file stem, file text) for every `.json` in `dir`.
fn bundle(out: &mut String, name: &str, dir: &Path) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|path: &std::path::PathBuf| {
        path.extension().is_some_and(|e| e == "json") && path.file_stem().is_some_and(|s| s != "index")
    });
    files.sort();
    writeln!(out, "pub const {name}: &[(&str, &str)] = &[").unwrap();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let stem = path.file_stem().unwrap().to_string_lossy();
        writeln!(out, "    ({stem:?}, include_str!({:?})),", path.display().to_string()).unwrap();
    }
    writeln!(out, "];").unwrap();
}

fn main() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("data");
    let mut out = String::new();
    bundle(&mut out, "ROUTES", &data.join("routes"));
    bundle(&mut out, "LAYOUTS", &data.join("layouts/ggg"));
    let target = Path::new(&std::env::var("OUT_DIR").unwrap()).join("bundle.rs");
    std::fs::write(target, out).unwrap();
    tauri_build::build()
}
