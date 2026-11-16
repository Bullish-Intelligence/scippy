//! The three recipes of the skill `.agents/skills/scippy-metalayer/SKILL.md`.
//!
//! Each test runs the command of the skill and reads the fields that the skill names. If a
//! field or a command changes, change the skill with it.

use serde_json::Value;

use crate::check::indexed_copy;
use crate::harness::{fixture_db, run, scratch_db};

const BASE_MODULE: &str = "scippy python demo . pkg/base/";

/// Run `scippy metalayer <args> --db <db> --json`. Return the exit code and the envelope.
fn query(db: &std::path::Path, args: &[&str]) -> (i32, Value) {
    let mut all: Vec<&str> = args.to_vec();
    all.extend(["--db", db.to_str().unwrap(), "--json"]);
    let out = run(&all);
    let envelope = serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", out.stdout));
    (out.status, envelope)
}

/// The facts that every recipe reads from the envelope.
fn assert_limits_present(envelope: &Value) {
    assert_eq!(envelope["format_version"], 1);
    assert_eq!(envelope["ok"], true);
    // The fixture holds a note that is not UTF-8, so the input is not complete.
    assert_eq!(envelope["input_complete"], false);
    assert_eq!(envelope["analysis_exact"], true);
}

#[test]
fn who_imports_x() {
    let (exit, envelope) = query(
        fixture_db(),
        &["relations", "--target-name", "pick", "--kind", "import"],
    );
    assert_eq!(exit, 0);
    assert_limits_present(&envelope);
    let value = &envelope["value"];
    assert_eq!(value["total"], 2);
    assert_eq!(value["truncated"], false);
    let items = value["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    // Two symbols share the display name, so the skill says to compare `target.symbol`.
    let mut symbols = Vec::new();
    for item in items {
        assert_eq!(item["path"], "pkg/child.py");
        assert_eq!(item["kind"], "import");
        assert_eq!(item["supports"], 1);
        assert_eq!(item["target"]["display_name"], "pick");
        assert_eq!(item["target"]["external"], false);
        assert!(item["target"]["defined_in"].is_string());
        symbols.push(item["target"]["symbol"].as_str().unwrap().to_owned());
    }
    symbols.sort();
    symbols.dedup();
    assert_eq!(symbols.len(), 2);
}

#[test]
fn why_does_f_depend_on_s() {
    // The skill takes `S` from `target.symbol` of the first recipe.
    let (_, who) = query(
        fixture_db(),
        &["relations", "--target-name", "base", "--kind", "import"],
    );
    let symbol = who["value"]["items"][0]["target"]["symbol"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(symbol, BASE_MODULE);

    let (exit, envelope) = query(
        fixture_db(),
        &["explain", "--from", "pkg/child.py", "--to", &symbol],
    );
    assert_eq!(exit, 0);
    assert_limits_present(&envelope);
    let value = &envelope["value"];
    assert_eq!(value["source"], "ok");
    assert_eq!(value["relations"][0]["kind"], "import");
    assert_eq!(value["relations"][0]["supports"], 1);
    let support = &value["supports"][0];
    assert_eq!(support["kind"], "import");
    assert_eq!(support["line"], 3);
    assert_eq!(support["column"], 10);
    assert_eq!(support["reference"]["outcome"], "resolved");
    assert!(value["ambiguous"].as_array().unwrap().is_empty());
    assert!(value["unresolved_in_file"].is_null());
}

#[test]
fn a_pair_without_relation_reports_unresolved_in_file() {
    let (exit, envelope) = query(
        fixture_db(),
        &[
            "explain",
            "--from",
            "notes/index.md",
            "--to",
            "scippy python demo . pkg/base/Base#",
        ],
    );
    assert_eq!(exit, 0);
    let value = &envelope["value"];
    assert!(value["relations"].as_array().unwrap().is_empty());
    assert!(value["unresolved_in_file"].is_number());
}

#[test]
fn is_the_index_fresh() {
    // Exit 0: fresh.
    let (exit, envelope) = query(fixture_db(), &["check"]);
    assert_eq!(exit, 0);
    assert_limits_present(&envelope);
    let value = &envelope["value"];
    assert_eq!(value["fresh"], true);
    for list in ["changed", "missing", "new"] {
        assert!(value[list].as_array().unwrap().is_empty(), "{list}");
    }

    // Exit 3: stale. The envelope still has `ok: true`.
    let copy = indexed_copy("skill-stale");
    let base = copy.root.join("pkg/base.py");
    let mut text = std::fs::read_to_string(&base).unwrap();
    text.push_str("# one more line\n");
    std::fs::write(&base, text).unwrap();
    let (exit, envelope) = query(&copy.db, &["check"]);
    assert_eq!(exit, 3);
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["value"]["fresh"], false);
    assert_eq!(envelope["value"]["changed"][0], "pkg/base.py");

    // Exit 4: no revision. The error kind is `no-revision`.
    let db = scratch_db("skill-no-revision");
    scippy_metalayer::Metalayer::open(&db).unwrap();
    let (exit, envelope) = query(&db, &["check"]);
    assert_eq!(exit, 4);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["kind"], "no-revision");
}
