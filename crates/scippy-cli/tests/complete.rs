//! Check that `AnalysisSnapshot::complete` is false when the run loses input.

use std::fs;
use std::path::Path;

use scippy_core::{AnalysisSnapshot, IndexRequest, index_project_with_snapshot};
use scippy_loci::LociPack;
use scippy_python::PythonPack;
use scippy_typescript::TypeScriptPack;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn snapshot(root: &Path, configure: impl FnOnce(&mut IndexRequest)) -> AnalysisSnapshot {
    let out = tempfile::tempdir().unwrap();
    let mut request = IndexRequest::new(root, out.path().join("index.scip"));
    configure(&mut request);
    index_project_with_snapshot(&request, &[&PythonPack, &TypeScriptPack, &LociPack])
        .unwrap()
        .1
}

fn has_code(snapshot: &AnalysisSnapshot, code: &str) -> bool {
    snapshot.diagnostics.iter().any(|d| d.code == code)
}

#[test]
fn a_clean_project_is_complete() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.py", "def f():\n    return 1\n\nf()\n");
    write(dir.path(), "pyproject.toml", "[project]\nname = \"demo\"\n");
    let snapshot = snapshot(dir.path(), |_| {});
    assert!(snapshot.complete, "{:?}", snapshot.diagnostics);
    assert_eq!(snapshot.bounded_references, 0);
}

#[test]
fn a_file_over_the_size_limit_is_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.py", "def f():\n    return 1\n");
    write(dir.path(), "big.py", &format!("x = 1\n{}", "#".repeat(200)));
    let snapshot = snapshot(dir.path(), |r| r.limits.max_file_bytes = 100);
    assert!(!snapshot.complete);
    assert!(has_code(&snapshot, "file-too-large"));
}

#[cfg(unix)]
#[test]
fn a_symlinked_source_file_is_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "real.py", "x = 1\n");
    std::os::unix::fs::symlink("real.py", dir.path().join("link.py")).unwrap();
    let snapshot = snapshot(dir.path(), |_| {});
    assert!(!snapshot.complete);
    assert!(has_code(&snapshot, "symlink-skipped"));
}

#[test]
fn a_broken_pyproject_is_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.py", "x = 1\n");
    write(dir.path(), "pyproject.toml", "[project\nname = ");
    let snapshot = snapshot(dir.path(), |_| {});
    assert!(!snapshot.complete);
    assert!(has_code(&snapshot, "manifest-error"));
}

#[test]
fn a_broken_tsconfig_is_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.ts", "export const x = 1;\n");
    write(dir.path(), "tsconfig.json", "{ not json");
    let snapshot = snapshot(dir.path(), |_| {});
    assert!(!snapshot.complete);
    assert!(has_code(&snapshot, "support-file-error"));
}
