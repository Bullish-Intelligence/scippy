//! Golden cases for `scippy index --json`.

use std::fs;

use serde_json::Value;

use crate::harness::{golden_index, project_root, run_index, scratch_db, scratch_dir};

#[test]
fn incomplete_run() {
    // The fixture holds a note that is not UTF-8, so the run lost input.
    golden_index("incomplete", project_root(), &[], 3);
}

#[test]
fn complete_run() {
    let root = scratch_dir("index-complete-project");
    fs::write(root.join("good.py"), "def f():\n    return 1\n\nf()\n").unwrap();
    golden_index("complete", &root, &[], 0);
}

#[test]
fn published_run() {
    let db = scratch_db("index-published");
    golden_index(
        "published",
        project_root(),
        &["--metalayer", db.to_str().unwrap()],
        3,
    );
}

#[test]
fn strict_run_with_lost_input() {
    golden_index("strict", project_root(), &["--strict"], 1);
}

#[test]
fn json_run_reports_the_same_flags_as_the_database() {
    let db = scratch_db("index-flags");
    let out = run_index(
        project_root(),
        "index-flags",
        &["--json", "--metalayer", db.to_str().unwrap()],
    );
    assert_eq!(out.status, 3, "{}", out.stderr);
    let envelope: Value = serde_json::from_str(&out.stdout).unwrap();
    let shown = crate::harness::run(&["show", "--json", "--db", db.to_str().unwrap()]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(envelope["input_complete"], shown["input_complete"]);
    assert_eq!(envelope["analysis_exact"], shown["analysis_exact"]);
    assert_eq!(
        envelope["value"]["metalayer"]["revision"],
        shown["revision"]["id"]
    );
}
