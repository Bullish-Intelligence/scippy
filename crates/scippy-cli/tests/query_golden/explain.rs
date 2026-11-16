//! Golden cases for `scippy metalayer explain`.

use std::fs;
use std::path::Path;

use crate::common::copy_tree;
use crate::harness::{golden, golden_db, golden_exit, golden_text, scratch_db};

const BASE_MODULE: &str = "scippy python demo . pkg/base/";
const BASE_CLASS: &str = "scippy python demo . pkg/base/Base#";
const CAFE_FUNCTION: &str = "scippy python demo . pkg/`café`/bonjour().";
const LEFT_PICK: &str = "scippy python demo . pkg/left/pick().";
const TARGET_NOTE: &str = "scippy loci project . notes/`target.md`/";

#[test]
fn import_pair() {
    golden(
        "import",
        &["explain", "--from", "pkg/child.py", "--to", BASE_MODULE],
    );
}

#[test]
fn inheritance_pair_has_an_import_and_a_use() {
    golden(
        "inheritance",
        &["explain", "--from", "pkg/child.py", "--to", BASE_CLASS],
    );
}

#[test]
fn ambiguous_candidate_pair() {
    golden(
        "ambiguous",
        &["explain", "--from", "pkg/child.py", "--to", LEFT_PICK],
    );
}

#[test]
fn pair_without_relation_counts_the_open_references() {
    golden(
        "no-relation",
        &["explain", "--from", "notes/index.md", "--to", BASE_CLASS],
    );
}

#[test]
fn loci_link_pair() {
    golden(
        "loci-link",
        &["explain", "--from", "notes/index.md", "--to", TARGET_NOTE],
    );
}

#[test]
fn non_ascii_column_counts_characters() {
    golden(
        "non-ascii",
        &["explain", "--from", "pkg/child.py", "--to", CAFE_FUNCTION],
    );
}

#[test]
fn human_text() {
    golden_text(
        "human",
        &["explain", "--from", "pkg/child.py", "--to", BASE_CLASS],
    );
}

#[test]
fn unknown_file_is_not_found() {
    golden_exit(
        "unknown-file",
        &["explain", "--from", "pkg/none.py", "--to", BASE_CLASS],
        1,
    );
}

#[test]
fn unknown_symbol_is_not_found() {
    golden_exit(
        "unknown-symbol",
        &[
            "explain",
            "--from",
            "pkg/child.py",
            "--to",
            "scippy nothing.",
        ],
        1,
    );
}

#[test]
fn local_symbol_without_a_file_is_not_found() {
    golden_exit(
        "local-without-file",
        &["explain", "--from", "pkg/child.py", "--to", "local 0"],
        1,
    );
}

/// Index a private copy of the fixture, then change one file and delete another.
fn edited_copy(name: &str) -> std::path::PathBuf {
    let db = scratch_db(name);
    // The project name is the name of the root folder, and it is part of a Loci symbol.
    let root = db.with_extension("dir").join("project");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/query"),
        &root,
    );
    let indexed = crate::common::scippy()
        .args(["index"])
        .arg(&root)
        .arg("--output")
        .arg(db.with_extension("scip"))
        .arg("--metalayer")
        .arg(&db)
        .output()
        .unwrap();
    assert!(
        crate::common::finished(&indexed),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    let child = root.join("pkg/child.py");
    let mut text = fs::read_to_string(&child).unwrap();
    text.push_str("# edited after the index\n");
    fs::write(child, text).unwrap();
    fs::remove_file(root.join("notes/index.md")).unwrap();
    db
}

#[test]
fn changed_and_missing_source_give_null_positions() {
    let db = edited_copy("explain-edited");
    golden_db(
        "changed",
        &db,
        &["explain", "--from", "pkg/child.py", "--to", BASE_CLASS],
        0,
    );
    golden_db(
        "missing",
        &db,
        &["explain", "--from", "notes/index.md", "--to", TARGET_NOTE],
        0,
    );
}
