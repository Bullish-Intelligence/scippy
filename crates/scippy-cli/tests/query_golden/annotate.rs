//! Golden cases for `scippy metalayer annotate`. The seed is in `harness::identity_store`.
//!
//! Each case runs on a fresh store, because the writers change it.

use std::fs;

use serde_json::Value;

use crate::harness::{
    CANDIDATE_A, CANDIDATE_B, golden_identity, golden_identity_raw, golden_identity_text,
    identity_path, identity_store, run_fixed, seed_id,
};

/// Run a golden case on a fresh store.
fn golden(case: &str, args: &[&str], exit: i32) {
    let store = identity_store(&format!("annotate-{case}"));
    golden_identity(case, &store, args, exit);
}

/// Run a command with `--json` on a store. Return the exit code and the envelope.
fn query(store: &std::path::Path, args: &[&str]) -> (i32, Value) {
    let mut all: Vec<&str> = args.to_vec();
    all.extend(["--identity", store.to_str().unwrap(), "--json"]);
    let out = run_fixed(&all);
    let envelope = serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}\n{}", out.stdout, out.stderr));
    (out.status, envelope)
}

// ---- add ----

#[test]
fn add_by_symbol_with_a_tag_and_an_author() {
    golden(
        "add",
        &[
            "annotate",
            "add",
            "scippy python demo 0.1.0 pkg/base/Base#make().",
            "--body",
            "Cache the result.",
            "--tag",
            "perf",
            "--author",
            "ann",
        ],
        0,
    );
}

#[test]
fn add_to_a_heading() {
    golden(
        "add-heading",
        &[
            "annotate",
            "add",
            "notes/index.md#Intro/Setup",
            "--body",
            "Add a diagram.",
        ],
        0,
    );
}

#[test]
fn add_to_an_orphan() {
    golden(
        "add-orphan",
        &["annotate", "add", &seed_id(0xe6), "--body", "Still wrong."],
        0,
    );
}

#[test]
fn add_with_an_ambiguous_locator_lists_the_matches() {
    golden(
        "add-ambiguous-locator",
        &["annotate", "add", "pkg/dup.py::dup", "--body", "x"],
        1,
    );
}

#[test]
fn add_with_an_unknown_locator() {
    golden(
        "add-no-entity",
        &["annotate", "add", "pkg/nope.py::f", "--body", "x"],
        1,
    );
}

#[test]
fn add_with_an_empty_body() {
    golden(
        "add-empty-body",
        &["annotate", "add", &seed_id(0xe2), "--body", " "],
        1,
    );
}

#[test]
fn add_needs_a_store() {
    let store = identity_path("annotate-add-missing");
    let _ = fs::remove_file(&store);
    golden_identity(
        "add-no-identity-store",
        &store,
        &["annotate", "add", "x", "--body", "y"],
        1,
    );
    assert!(!store.exists(), "add must not create the store");
}

#[test]
fn annotate_needs_no_facts_store() {
    let store = identity_store("annotate-nofacts");
    let db = store.with_file_name("annotate-nofacts.sqlite");
    assert!(!db.exists());
    golden_identity_raw(
        "list-no-facts-store",
        &["annotate", "list", "--db", db.to_str().unwrap()],
        0,
    );
    assert!(!db.exists(), "annotate must not create the facts store");
}

// ---- list ----

#[test]
fn list_all() {
    golden("list", &["annotate", "list"], 0);
}

#[test]
fn list_by_path() {
    golden(
        "list-path",
        &["annotate", "list", "--path", "pkg/base.py"],
        0,
    );
}

#[test]
fn list_by_status() {
    golden(
        "list-status-orphaned",
        &["annotate", "list", "--status", "orphaned"],
        0,
    );
}

#[test]
fn list_by_entity() {
    let id = seed_id(0xe4);
    golden("list-entity", &["annotate", "list", "--entity", &id], 0);
}

#[test]
fn list_with_a_bad_status() {
    golden(
        "list-bad-status",
        &["annotate", "list", "--status", "lost"],
        1,
    );
}

#[test]
fn list_human() {
    let store = identity_store("annotate-list-human");
    golden_identity_text("list-human", &store, &["annotate", "list"]);
}

// ---- rm ----

#[test]
fn rm_one_annotation() {
    golden("rm", &["annotate", "rm", &seed_id(0xa1)], 0);
}

#[test]
fn rm_an_unknown_annotation() {
    golden("rm-unknown", &["annotate", "rm", "nope"], 1);
}

// ---- status ----

#[test]
fn status_lists_the_flagged_entities() {
    golden("status", &["annotate", "status"], 0);
}

#[test]
fn status_annotated_only() {
    golden(
        "status-annotated",
        &["annotate", "status", "--annotated"],
        0,
    );
}

#[test]
fn status_human() {
    let store = identity_store("annotate-status-human");
    golden_identity_text("status-human", &store, &["annotate", "status"]);
}

// ---- accept ----

#[test]
fn accept_a_moved_entity() {
    golden("accept-moved", &["annotate", "accept", &seed_id(0xe4)], 0);
}

#[test]
fn accept_all_of_one_tier() {
    golden(
        "accept-all-tier",
        &["annotate", "accept", "--all", "--tier", "T2"],
        0,
    );
}

#[test]
fn accept_all_with_a_bad_tier() {
    golden(
        "accept-all-bad-tier",
        &["annotate", "accept", "--all", "--tier", "T9"],
        1,
    );
}

#[test]
fn accept_a_candidate() {
    golden(
        "accept-candidate",
        &[
            "annotate",
            "accept",
            &seed_id(0xe5),
            "--candidate",
            CANDIDATE_A,
        ],
        0,
    );
}

#[test]
fn accept_a_candidate_that_has_an_annotation() {
    golden(
        "accept-candidate-annotated",
        &[
            "annotate",
            "accept",
            &seed_id(0xe5),
            "--candidate",
            CANDIDATE_B,
        ],
        1,
    );
}

#[test]
fn accept_an_ambiguous_entity_without_a_candidate() {
    golden(
        "accept-needs-candidate",
        &["annotate", "accept", &seed_id(0xe5)],
        1,
    );
}

#[test]
fn accept_a_matched_entity() {
    golden("accept-matched", &["annotate", "accept", &seed_id(0xe2)], 1);
}

#[test]
fn accept_needs_an_entity_or_all() {
    let out = run_fixed(&["annotate", "accept", "--identity", "x"]);
    assert_eq!(out.status, 2, "{}", out.stderr);
    let out = run_fixed(&["annotate", "accept", "--all", "--identity", "x"]);
    assert_eq!(out.status, 2, "{}", out.stderr);
}

// ---- reject ----

#[test]
fn reject_a_moved_entity() {
    golden("reject-moved", &["annotate", "reject", &seed_id(0xe4)], 0);
}

#[test]
fn reject_an_ambiguous_entity() {
    golden(
        "reject-ambiguous",
        &["annotate", "reject", &seed_id(0xe5)],
        0,
    );
}

#[test]
fn reject_a_matched_entity() {
    golden("reject-matched", &["annotate", "reject", &seed_id(0xe2)], 1);
}

// ---- flows ----

#[test]
fn add_list_and_remove_round_trip() {
    let store = identity_store("annotate-flow-add");
    let (exit, added) = query(
        &store,
        &["annotate", "add", "pkg/base.py::Base", "--body", "Hello"],
    );
    assert_eq!(exit, 0);
    let id = added["value"]["annotation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(id, seed_id(1));
    let (_, listed) = query(&store, &["annotate", "list", "--entity", &seed_id(0xe2)]);
    assert_eq!(listed["value"]["total"], 2);
    let (exit, _) = query(&store, &["annotate", "rm", &id]);
    assert_eq!(exit, 0);
    let (_, listed) = query(&store, &["annotate", "list", "--entity", &seed_id(0xe2)]);
    assert_eq!(listed["value"]["total"], 1);
}

#[test]
fn reject_returns_the_entity_to_its_old_locator() {
    let store = identity_store("annotate-flow-reject");
    let (exit, rejected) = query(&store, &["annotate", "reject", &seed_id(0xe4)]);
    assert_eq!(exit, 0);
    assert_eq!(rejected["value"]["entity"]["state"], "orphaned");
    let (_, entity) = query(&store, &["entity", &seed_id(0xe4)]);
    let value = &entity["value"];
    assert_eq!(value["locator"]["path"], "pkg/base.py");
    assert_eq!(value["locators"].as_array().unwrap().len(), 1);
    assert_eq!(value["vetoes"][0], "pkg/util/helper().");
    assert_eq!(value["annotations"].as_array().unwrap().len(), 1);
    // The orphan is in the status list, with its annotation.
    let (_, status) = query(&store, &["annotate", "status", "--annotated"]);
    let states: Vec<&str> = status["value"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["entity"]["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["orphaned", "orphaned", "ambiguous"]);
}

#[test]
fn accept_a_candidate_moves_the_locator_and_deletes_the_minted_entity() {
    let store = identity_store("annotate-flow-candidate");
    let (exit, accepted) = query(
        &store,
        &[
            "annotate",
            "accept",
            &seed_id(0xe5),
            "--candidate",
            CANDIDATE_A,
        ],
    );
    assert_eq!(exit, 0);
    assert_eq!(accepted["value"]["action"], "bound");
    assert_eq!(accepted["value"]["removed_entity"], seed_id(0xe10));
    let (_, entity) = query(&store, &["entity", &seed_id(0xe5)]);
    assert_eq!(entity["value"]["outcome"]["state"], "matched");
    assert_eq!(entity["value"]["locator"]["path"], "pkg/a.py");
    let (exit, gone) = query(&store, &["entity", &seed_id(0xe10)]);
    assert_eq!(exit, 1);
    assert_eq!(gone["error"]["kind"], "no-entity");
}

#[test]
fn a_failed_accept_changes_nothing() {
    let store = identity_store("annotate-flow-annotated");
    let (exit, error) = query(
        &store,
        &[
            "annotate",
            "accept",
            &seed_id(0xe5),
            "--candidate",
            CANDIDATE_B,
        ],
    );
    assert_eq!(exit, 1);
    assert_eq!(error["error"]["kind"], "candidate-annotated");
    let (_, entity) = query(&store, &["entity", &seed_id(0xe5)]);
    assert_eq!(entity["value"]["outcome"]["state"], "ambiguous");
    let (exit, _) = query(&store, &["entity", &seed_id(0xe11)]);
    assert_eq!(exit, 0);
}

#[test]
fn fixed_ids_are_for_tests_and_the_default_ids_are_uuids() {
    let store = identity_store("annotate-system-ids");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_scippy"))
        .args([
            "metalayer",
            "annotate",
            "add",
            &seed_id(0xe2),
            "--body",
            "x",
            "--json",
        ])
        .args(["--identity", store.to_str().unwrap()])
        .env_remove("SCIPPY_TEST_FIXED_IDS")
        .output()
        .unwrap();
    assert!(out.status.success());
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
    let id = envelope["value"]["annotation"]["id"].as_str().unwrap();
    assert_eq!(id.len(), 36);
    assert_eq!(id.as_bytes()[14], b'7', "UUIDv7: {id}");
    let created = envelope["value"]["annotation"]["created_at"]
        .as_str()
        .unwrap();
    assert!(created.ends_with('Z') && created.len() == 20, "{created}");
}
