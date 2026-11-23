//! An override chain: a base declaration and its overrides. The index names the target that a
//! type checker names for an attribute, and it names every candidate for a method.

mod common;

use std::fs;

use protobuf::Message;
use scip::types::{Index, SymbolRole, occurrence::Typed_range};

/// Index one file `m.py` and return the symbols of the references at 0-based `line`, whose symbol
/// text contains `name`.
fn references_at(source: &str, line: i32, name: &str) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("m.py"), source).unwrap();
    let output = dir.path().join("out.scip");
    let run = common::scippy()
        .args(["index", dir.path().to_str().unwrap(), "--output"])
        .arg(&output)
        .args(["--package-name", "chain"])
        .output()
        .unwrap();
    assert!(
        common::finished(&run),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let index = Index::parse_from_bytes(&fs::read(&output).unwrap()).unwrap();
    let document = index
        .documents
        .iter()
        .find(|d| d.relative_path == "m.py")
        .unwrap();
    document
        .occurrences
        .iter()
        .filter(
            |o| matches!(&o.typed_range, Some(Typed_range::SingleLineRange(r)) if r.line == line),
        )
        .filter(|o| o.symbol_roles & SymbolRole::Definition as i32 == 0)
        .filter(|o| o.symbol.contains(name))
        .map(|o| o.symbol.clone())
        .collect()
}

#[test]
fn a_method_chain_names_the_base_and_the_override() {
    let source = "class Plugin:
    def hook(self):
        return 1


class Chained(Plugin):
    def hook(self):
        return 2


class Manager:
    def __init__(self, plugin: Plugin):
        self.plugin = plugin

    def wrap(self):
        self.plugin = Chained()

    def run(self):
        return self.plugin.hook()
";
    let symbols = references_at(source, 18, "hook");
    assert_eq!(
        symbols,
        [
            "scippy python chain . m/Chained#hook().",
            "scippy python chain . m/Plugin#hook()."
        ]
    );
}

#[test]
fn an_attribute_chain_names_the_most_derived_store() {
    let source = "class Base:
    def __init__(self):
        self.enc = None


class Child(Base):
    def __init__(self):
        super().__init__()
        if not self.enc:
            self.enc = 'utf-8'
        self.n = self.enc
";
    let symbols = references_at(source, 10, "enc");
    assert_eq!(symbols, ["scippy python chain . m/Child#enc."]);
}
