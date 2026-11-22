//! A union gives several occurrences at one range. The query commands must handle them.
//!
//! The project has `e: A | B` then `e.items`. The index holds one occurrence for each class at
//! the range of `items`, and the reference stays ambiguous with both candidates. The tests check
//! the three readers of that data: the `.scip` file, the metalayer store, and `metalayer explain`.

mod common;

use std::fs;
use std::path::Path;

use protobuf::Message;
use scip::types::{Index, Occurrence, SymbolRole, occurrence::Typed_range};
use serde_json::Value;

const SOURCE: &str = "class A:
    def __init__(self):
        self.items = []


class B:
    def __init__(self):
        self.items = ()


def use(e: A | B):
    return e.items
";

const CANDIDATES: [&str; 2] = [
    "scippy python union . m/A#items.",
    "scippy python union . m/B#items.",
];

fn indexed(root: &Path) -> (Index, std::path::PathBuf) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("m.py"), SOURCE).unwrap();
    let output = root.join("out.scip");
    let db = root.join("layer.sqlite");
    let run = common::scippy()
        .args(["index", root.to_str().unwrap(), "--output"])
        .arg(&output)
        .arg("--metalayer")
        .arg(&db)
        .args(["--package-name", "union"])
        .output()
        .unwrap();
    assert!(
        common::finished(&run),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let index = Index::parse_from_bytes(&fs::read(&output).unwrap()).unwrap();
    (index, db)
}

fn at_items(index: &Index) -> Vec<&Occurrence> {
    let document = index
        .documents
        .iter()
        .find(|d| d.relative_path == "m.py")
        .unwrap();
    // `e.items` is the last line with `items`: line 12 of the file, index 11.
    document
        .occurrences
        .iter()
        .filter(|o| matches!(&o.typed_range, Some(Typed_range::SingleLineRange(r)) if r.line == 11))
        .filter(|o| o.symbol_roles & SymbolRole::Definition as i32 == 0)
        .filter(|o| o.symbol.contains("items"))
        .collect()
}

fn explain(db: &Path, candidate: &str) -> Value {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_scippy"))
        .args([
            "metalayer",
            "explain",
            "--from",
            "m.py",
            "--to",
            candidate,
            "--json",
            "--db",
        ])
        .arg(db)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn go_to_definition_finds_every_candidate_of_a_union() {
    let dir = tempfile::tempdir().unwrap();
    let (index, _) = indexed(dir.path());
    let here = at_items(&index);
    // Two occurrences share one range. Each names a candidate, in the order of the symbols.
    assert_eq!(here.len(), 2, "{here:?}");
    assert_eq!(here[0].typed_range, here[1].typed_range);
    let symbols: Vec<&str> = here.iter().map(|o| o.symbol.as_str()).collect();
    assert_eq!(symbols, CANDIDATES);
    // The definition of each candidate is in the index, so a client can go to it.
    let document = index
        .documents
        .iter()
        .find(|d| d.relative_path == "m.py")
        .unwrap();
    for symbol in CANDIDATES {
        let definitions = document
            .occurrences
            .iter()
            .filter(|o| o.symbol == symbol && o.symbol_roles & SymbolRole::Definition as i32 != 0)
            .count();
        assert_eq!(definitions, 1, "{symbol}");
    }
}

#[test]
fn find_references_lists_the_site_for_each_candidate_of_a_union() {
    let dir = tempfile::tempdir().unwrap();
    let (index, db) = indexed(dir.path());
    let site = at_items(&index)[0].typed_range.clone();
    for symbol in CANDIDATES {
        // The index lists the site as a reference of the symbol.
        let document = index
            .documents
            .iter()
            .find(|d| d.relative_path == "m.py")
            .unwrap();
        assert!(
            document
                .occurrences
                .iter()
                .any(|o| o.symbol == symbol && o.typed_range == site),
            "{symbol}"
        );
        // The store lists the site as an ambiguous reference with both candidates.
        let envelope = explain(&db, symbol);
        let ambiguous = envelope["value"]["ambiguous"].as_array().unwrap();
        assert_eq!(ambiguous.len(), 1, "{symbol}: {envelope:#}");
        let reference = &ambiguous[0]["reference"];
        assert_eq!(reference["outcome"], "ambiguous");
        let candidates: Vec<&str> = reference["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["symbol"].as_str().unwrap())
            .collect();
        assert_eq!(candidates, CANDIDATES);
        assert_eq!(ambiguous[0]["line"], 12);
    }
}
