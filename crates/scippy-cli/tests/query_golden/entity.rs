//! Golden cases for `scippy metalayer entity`. The seed is in `harness::identity_store`.

use std::fs;

use serde_json::Value;

use crate::harness::{
    golden_identity, golden_identity_raw, golden_identity_text, identity_path, identity_store,
    run_fixed, seed_id,
};

fn golden(case: &str, locator: &str, exit: i32) {
    let store = identity_store(&format!("entity-{case}"));
    golden_identity(case, &store, &["entity", locator], exit);
}

#[test]
fn by_symbol() {
    golden("symbol", "scippy python demo 0.1.0 pkg/base/Base#", 0);
}

#[test]
fn by_symbol_without_the_version() {
    golden("key", "pkg/base/Base#make().", 0);
}

#[test]
fn by_path_and_container() {
    golden("path-container", "pkg/base.py::Base.make", 0);
}

#[test]
fn by_id_with_a_moved_history() {
    golden("moved", &seed_id(0xe4), 0);
}

#[test]
fn an_ambiguous_entity_lists_its_candidates() {
    golden("ambiguous", "pkg/old.py::pick", 0);
}

#[test]
fn an_orphan_keeps_its_veto_and_annotation() {
    golden("orphaned", "pkg/gone.py::gone", 0);
}

#[test]
fn a_module() {
    golden("module", "pkg/base.py::", 0);
}

#[test]
fn a_note() {
    golden("note", "notes/index.md", 0);
}

#[test]
fn a_heading() {
    golden("heading", "notes/index.md#Intro/Setup", 0);
}

#[test]
fn a_block() {
    golden("block", "notes/index.md^b1", 0);
}

#[test]
fn several_matches_are_an_ambiguous_locator() {
    golden("ambiguous-locator", "pkg/dup.py::dup", 1);
}

#[test]
fn no_match_is_no_entity() {
    golden("no-entity", "pkg/base.py::Missing", 1);
}

#[test]
fn a_missing_store_is_no_identity_store() {
    let store = identity_path("entity-missing");
    let _ = fs::remove_file(&store);
    golden_identity("no-identity-store", &store, &["entity", "x"], 1);
    assert!(!store.exists(), "a read must not create the store");
}

#[test]
fn a_newer_store_is_a_schema_mismatch() {
    let store = identity_store("entity-newer");
    let db = rusqlite::Connection::open(&store).unwrap();
    db.execute_batch("PRAGMA user_version = 99;").unwrap();
    drop(db);
    golden_identity("schema-mismatch-newer", &store, &["entity", "x"], 1);
}

#[test]
fn a_foreign_file_is_a_schema_mismatch() {
    let file = identity_path("entity-foreign");
    fs::write(&file, "text, not SQLite\n").unwrap();
    golden_identity("schema-mismatch-foreign", &file, &["entity", "x"], 1);
}

#[test]
fn the_default_path_sits_next_to_the_database() {
    let store = identity_store("entity-default");
    let db = store.with_file_name("entity-default.sqlite");
    // No facts store exists at `db`. The identity store is enough.
    assert!(!db.exists());
    golden_identity_raw(
        "default-path",
        &["entity", "pkg/base.py::Base", "--db", db.to_str().unwrap()],
        0,
    );
}

#[test]
fn human_output() {
    let store = identity_store("entity-human");
    golden_identity_text("human", &store, &["entity", &seed_id(0xe4)]);
}

#[test]
fn a_command_needs_a_path() {
    let out = run_fixed(&["entity", "x", "--json"]);
    assert_eq!(out.status, 2, "{}", out.stderr);
    assert!(out.stdout.is_empty());
}

#[test]
fn the_envelope_has_no_revision() {
    let store = identity_store("entity-envelope");
    let out = run_fixed(&[
        "entity",
        "pkg/base.py::Base",
        "--identity",
        store.to_str().unwrap(),
        "--json",
    ]);
    let envelope: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(envelope["format_version"], 1);
    assert_eq!(envelope["identity"]["version"], 1);
    assert_eq!(envelope["identity"]["bound_input_digest"], "digest-2");
    assert!(envelope.get("revision").is_none());
}
