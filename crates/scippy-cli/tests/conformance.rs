//! Gate 6 conformance checks through the `scippy` binary (concept, sections 10 and 11).

mod common;

use std::fs;
use std::path::Path;
use std::process::Output;

use protobuf::Message;
use scip::types::{Index, SymbolRole};

fn scippy(args: &[&str]) -> Output {
    common::scippy().args(args).output().unwrap()
}

fn write(root: &Path, file: &str, text: &str) {
    let path = root.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn index_root(root: &Path, out: &Path) -> Index {
    let output = scippy(&["index", root.to_str().unwrap(), "-o", out.to_str().unwrap()]);
    assert!(output.status.success(), "{output:?}");
    Index::parse_from_bytes(&fs::read(out).unwrap()).unwrap()
}

/// The symbol that the document defines with the given display name.
fn defined(index: &Index, path: &str, name: &str) -> String {
    let document = index
        .documents
        .iter()
        .find(|d| d.relative_path == path)
        .unwrap_or_else(|| panic!("no document {path}"));
    document
        .symbols
        .iter()
        .find(|s| s.display_name == name)
        .unwrap_or_else(|| panic!("no symbol {name} in {path}"))
        .symbol
        .clone()
}

/// How many occurrences of `symbol` the document holds that are not definitions.
fn references(index: &Index, path: &str, symbol: &str) -> usize {
    index
        .documents
        .iter()
        .find(|d| d.relative_path == path)
        .unwrap()
        .occurrences
        .iter()
        .filter(|o| o.symbol == symbol && o.symbol_roles & SymbolRole::Definition as i32 == 0)
        .count()
}

fn aliased_import_tree(root: &Path, order: &[&str]) {
    for file in order {
        match *file {
            "py_lib" => write(
                root,
                "src/services.py",
                "def process(value):\n    return value\n",
            ),
            "py_app" => write(
                root,
                "src/app.py",
                "from services import process as run\nresult = run(1)\n",
            ),
            "ts_lib" => write(
                root,
                "web/services.ts",
                "export function process(value: number): number {\n  return value;\n}\n",
            ),
            "ts_app" => write(
                root,
                "web/app.ts",
                "import { process as run } from \"./services\";\nexport const result = run(1);\n",
            ),
            other => panic!("{other}"),
        }
    }
}

#[test]
fn python_and_typescript_resolve_an_aliased_import_through_one_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("p");
    aliased_import_tree(&root, &["py_lib", "py_app", "ts_lib", "ts_app"]);
    let index = index_root(&root, &dir.path().join("index.scip"));
    let languages: Vec<_> = index
        .documents
        .iter()
        .map(|d| d.language.as_str())
        .collect();
    assert!(languages.contains(&"Python") && languages.contains(&"TypeScript"));
    // Each import name and each call names the definition in the other document. TypeScript
    // gives the alias name an occurrence too.
    for (lib, app, expected) in [
        ("src/services.py", "src/app.py", 2),
        ("web/services.ts", "web/app.ts", 3),
    ] {
        let symbol = defined(&index, lib, "process");
        assert_eq!(references(&index, app, &symbol), expected, "{app}");
    }
}

#[test]
fn cross_file_output_does_not_depend_on_the_creation_order_or_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let forward = ["py_lib", "py_app", "ts_lib", "ts_app"];
    let backward = ["ts_app", "ts_lib", "py_app", "py_lib"];
    let (one, two) = (dir.path().join("one"), dir.path().join("two"));
    aliased_import_tree(&one, &forward);
    aliased_import_tree(&two, &backward);
    let mut first = index_root(&one, &dir.path().join("one.scip"));
    let mut second = index_root(&two, &dir.path().join("two.scip"));
    // The symbol that a cross-file reference names exists in both runs.
    let symbol = defined(&first, "src/services.py", "process");
    assert_eq!(references(&first, "src/app.py", &symbol), 2);
    first.metadata.as_mut().unwrap().project_root.clear();
    second.metadata.as_mut().unwrap().project_root.clear();
    assert_eq!(
        first.write_to_bytes().unwrap(),
        second.write_to_bytes().unwrap()
    );
}

#[test]
fn an_invalid_vault_manifest_gives_a_non_zero_exit_and_a_named_error() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("p");
    write(&root, "n.md", "# N\n");
    write(&root, ".loci/vault.toml", "schema = 7\nid = \"x\"\n");
    let out = dir.path().join("index.scip");
    let output = scippy(&["index", root.to_str().unwrap(), "-o", out.to_str().unwrap()]);
    assert!(!output.status.success(), "{output:?}");
    let text = String::from_utf8_lossy(&output.stderr).to_string()
        + &String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("vault-manifest-invalid"), "{text}");
}
