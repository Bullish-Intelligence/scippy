//! Build the graphs of the builtins and the stubs once, at build time.
//!
//! Each run of the indexer needs them. Building them takes about a second. The pack loads
//! the result instead. The key inside the result names the rules and the sources, so a
//! mismatch never loads.

#[path = "src/rules.rs"]
mod rules;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/rules.rs");
    let blob =
        scippy_core::prebuild_assets(tree_sitter_python::LANGUAGE.into(), &rules::binding_rules())
            .unwrap_or_else(|e| panic!("cannot prebuild the Python asset graphs: {e}"));
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set"))
        .join("assets.bin");
    std::fs::write(&out, blob).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
}
