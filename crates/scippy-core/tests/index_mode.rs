//! `index` takes a mode. The mode decides the snapshot and the output file.

use std::path::Path;

use scippy_core::{IndexMode, IndexRequest, LanguagePack, ScippyError, index};

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

fn project() -> (tempfile::TempDir, IndexRequest) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.stub"), "x\n").unwrap();
    let request = IndexRequest::new(&root, dir.path().join("index.scip"));
    (dir, request)
}

#[test]
fn write_mode_writes_the_file_and_returns_no_snapshot() {
    let (_dir, request) = project();
    let out = index(&request, &[&StubPack], IndexMode::Write).unwrap();
    assert!(out.snapshot.is_none());
    assert!(request.output.exists());
}

#[test]
fn write_with_snapshot_mode_writes_the_file_and_returns_the_snapshot() {
    let (_dir, request) = project();
    let out = index(&request, &[&StubPack], IndexMode::WriteWithSnapshot).unwrap();
    assert!(out.snapshot.is_some());
    assert!(request.output.exists());
}

#[test]
fn snapshot_only_mode_returns_the_snapshot_and_writes_no_file() {
    let (_dir, request) = project();
    let out = index(&request, &[&StubPack], IndexMode::SnapshotOnly).unwrap();
    assert!(out.snapshot.is_some());
    assert!(!request.output.exists());
}

#[test]
fn a_strict_write_run_that_lost_input_fails_and_writes_no_file() {
    let (_dir, mut request) = project();
    request.strict = true;
    let result = index(&request, &[&StubPack], IndexMode::Write);
    assert!(matches!(result, Err(ScippyError::Incomplete)));
    assert!(!request.output.exists());
}
