//! Golden cases for `scippy metalayer show`.

use crate::harness::{golden, golden_db, golden_text, run, scratch_db};

#[test]
fn default() {
    golden("default", &["show"]);
}

#[test]
fn human() {
    golden_text("human", &["show"]);
}

#[test]
fn database_is_missing() {
    let db = scratch_db("missing").with_file_name("absent.sqlite");
    golden_db("database-missing", &db, &["show"], 1);
}

#[test]
fn database_has_no_revision() {
    let db = scratch_db("no-revision");
    scippy_metalayer::Metalayer::open(&db).unwrap();
    golden_db("no-revision", &db, &["show"], 4);
}

#[test]
fn database_has_another_format_version() {
    let db = scratch_db("old-version");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("PRAGMA application_id = 1396920409; PRAGMA user_version = 3;")
        .unwrap();
    drop(conn);
    golden_db("schema-mismatch", &db, &["show"], 1);
}

#[test]
fn file_is_not_a_database() {
    let db = scratch_db("not-a-database");
    std::fs::write(&db, "just text\n").unwrap();
    golden_db("not-scippy", &db, &["show"], 1);
}

#[test]
fn usage_error_is_a_clap_error() {
    let out = run(&["show", "--no-such-option"]);
    assert_eq!(out.status, 2);
    assert!(out.stdout.is_empty(), "{}", out.stdout);
    assert!(out.stderr.contains("--no-such-option"), "{}", out.stderr);
}

#[test]
fn human_error_goes_to_stderr() {
    let db = scratch_db("human-error").with_file_name("absent.sqlite");
    let out = run(&["show", "--db", db.to_str().unwrap()]);
    assert_eq!(out.status, 1);
    assert!(out.stdout.is_empty(), "{}", out.stdout);
    assert!(
        out.stderr.starts_with("scippy: no database at"),
        "{}",
        out.stderr
    );
}
