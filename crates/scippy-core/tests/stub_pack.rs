//! A language pack without analysis only routes files. The core skips them and says so.

use std::path::Path;

use scippy_core::{IndexRequest, LanguagePack, Severity, index_project};

struct StubPack;

impl LanguagePack for StubPack {
    fn id(&self) -> &'static str {
        "stub"
    }

    fn scip_language(&self) -> &'static str {
        "Stub"
    }

    fn accepts(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "stub")
    }
}

#[test]
fn a_pack_without_analysis_skips_its_files_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.stub"), "x\n").unwrap();
    std::fs::write(root.join("b.stub"), "y\n").unwrap();
    let request = IndexRequest::new(&root, dir.path().join("index.scip"));
    let report = index_project(&request, &[&StubPack]).unwrap();
    assert_eq!(report.files_discovered, 2);
    assert_eq!(report.files_indexed, 0);
    assert_eq!(report.files_skipped, 2);
    let warning = report
        .diagnostics
        .iter()
        .find(|d| d.code == "unsupported-language")
        .expect("a diagnostic");
    assert_eq!(warning.severity, Severity::Warning);
}
