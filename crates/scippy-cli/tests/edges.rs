//! Check the edges and the reference kind that a full run publishes.

use std::fs;
use std::path::Path;

use scippy_core::{AnalysisSnapshot, IndexRequest, index_project_with_snapshot};
use scippy_loci::LociPack;
use scippy_metalayer::{
    EdgeKind, Metalayer, Outcome, PublishOptions, ReferenceKind, ReferenceQuery, RelationKind,
};
use scippy_python::PythonPack;
use scippy_typescript::TypeScriptPack;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn publish(root: &Path, database: &Path) -> (AnalysisSnapshot, Metalayer) {
    let mut request = IndexRequest::new(root, root.parent().unwrap().join("index.scip"));
    request.jobs = 2;
    let (_, snapshot) =
        index_project_with_snapshot(&request, &[&PythonPack, &TypeScriptPack, &LociPack]).unwrap();
    let mut layer = Metalayer::open(database).unwrap();
    layer
        .publish(&snapshot, &PublishOptions::default())
        .unwrap();
    (snapshot, layer)
}

const SOURCE: &str = "import os\n\nclass A:\n    def run(self):\n        return 1\n\nclass B:\n    pass\n\nclass C(A, B):\n    def run(self):\n        return os.getcwd()\n\ndef f(x):\n    return x\n\nf(x=1)\nC().run()\n";

#[test]
fn a_class_with_two_supertypes_gives_two_inheritance_edges() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "m.py", SOURCE);
    write(&root, "pyproject.toml", "[project]\nname = \"demo\"\n");
    let (_, mut layer) = publish(&root, &scratch.path().join("layer.sqlite"));
    let view = layer.view().unwrap().unwrap();
    let edges = view.edges(Some(EdgeKind::Inheritance)).unwrap();
    assert_eq!(edges.len(), 2, "{edges:?}");
    let sources: Vec<_> = edges
        .iter()
        .map(|e| e.source.clone().unwrap().symbol)
        .collect();
    assert!(sources.iter().all(|s| s.ends_with("C#")), "{sources:?}");
    let targets: Vec<_> = edges
        .iter()
        .map(|e| e.target.clone().unwrap().symbol)
        .collect();
    assert!(
        targets[0].ends_with("A#") && targets[1].ends_with("B#"),
        "{targets:?}"
    );
    // The span is the name in the class header.
    let start = SOURCE.find("C(A, B)").unwrap() as u32 + 2;
    assert_eq!(edges[0].span, Some((start, start + 1)));
    assert_eq!(edges[1].span, Some((start + 3, start + 4)));
    // The override edge from the declarations is still there.
    assert_eq!(view.edges(Some(EdgeKind::Override)).unwrap().len(), 1);
}

#[test]
fn an_import_reference_gives_an_import_edge() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "lib.py", "def make():\n    return 1\n");
    let user = "from lib import make\n\nmake()\n";
    write(&root, "use.py", user);
    write(&root, "pyproject.toml", "[project]\nname = \"demo\"\n");
    let (_, mut layer) = publish(&root, &scratch.path().join("layer.sqlite"));
    let view = layer.view().unwrap().unwrap();
    let imports = view.edges(Some(EdgeKind::Import)).unwrap();
    let start = user.find("make").unwrap() as u32;
    // The module name and the imported name are two import references.
    let found: Vec<_> = imports.iter().filter(|e| e.path == "use.py").collect();
    assert_eq!(found.len(), 2, "{imports:?}");
    assert!(found[0].target.clone().unwrap().symbol.ends_with("lib/"));
    assert_eq!(found[0].span, Some((5, 8)));
    assert_eq!(found[1].span, Some((start, start + 4)));
    assert!(
        found[1]
            .target
            .clone()
            .unwrap()
            .symbol
            .ends_with("lib/make().")
    );
    assert!(found.iter().all(|e| e.source.is_none()));
    // The use of the name is not an import.
    let supports = view
        .supports("use.py", &found[1].target.clone().unwrap())
        .unwrap();
    let kinds: Vec<_> = supports.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [RelationKind::Import, RelationKind::Use]);
}

#[test]
fn the_reference_kind_is_plain_member_or_keyword_and_none_for_a_native_pack() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "m.py", SOURCE);
    write(&root, "pyproject.toml", "[project]\nname = \"demo\"\n");
    write(
        &root,
        ".loci/vault.toml",
        "schema = 1\nid = \"01a08b81-648f-70e8-bb5f-cd8c53f0f305\"\n",
    );
    write(&root, "n.md", "# N\n\n[[n]]\n");
    let (_, mut layer) = publish(&root, &scratch.path().join("layer.sqlite"));
    let view = layer.view().unwrap().unwrap();
    let kind_at = |path: &str, text: &str, token: &str| {
        let source = fs::read_to_string(root.join(path)).unwrap();
        let start = (source.find(text).unwrap() + text.find(token).unwrap()) as u32;
        let found = view
            .references(ReferenceQuery {
                path: Some(path.to_owned()),
                ..Default::default()
            })
            .unwrap()
            .into_iter()
            .find(|r| r.span == (start, start + token.len() as u32))
            .unwrap_or_else(|| panic!("no reference for {token} in {text}"));
        found.kind
    };
    assert_eq!(kind_at("m.py", "C(A, B)", "A"), Some(ReferenceKind::Plain));
    assert_eq!(
        kind_at("m.py", "C().run()", "run"),
        Some(ReferenceKind::Member)
    );
    assert_eq!(kind_at("m.py", "f(x=1)", "x"), Some(ReferenceKind::Keyword));
    assert_eq!(kind_at("n.md", "[[n]]", "[[n]]"), None);
    // Every reference of a stack-graphs pack has a kind.
    let all = view.references(ReferenceQuery::default()).unwrap();
    assert!(
        all.iter()
            .filter(|r| r.path == "m.py")
            .all(|r| r.kind.is_some())
    );
    assert!(
        all.iter()
            .filter(|r| r.path == "n.md")
            .all(|r| r.kind.is_none())
    );
    assert!(
        all.iter()
            .any(|r| matches!(r.outcome, Outcome::Resolved(_)))
    );
}

#[test]
fn spans_are_utf8_bytes_in_a_file_that_is_not_ascii() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    let source = "# ééé 日本\nclass A:\n    pass\n\n\nclass B(A):\n    pass\n";
    write(&root, "m.py", source);
    write(&root, "pyproject.toml", "[project]\nname = \"demo\"\n");
    let (_, mut layer) = publish(&root, &scratch.path().join("layer.sqlite"));
    let view = layer.view().unwrap().unwrap();
    let edges = view.edges(Some(EdgeKind::Inheritance)).unwrap();
    assert_eq!(edges.len(), 1);
    // The offset counts bytes. The comment holds six non-ASCII characters of two or three bytes.
    let bytes = source.find("B(A)").unwrap() as u32 + 2;
    let chars = source[..bytes as usize].chars().count() as u32;
    assert_ne!(bytes, chars);
    assert_eq!(edges[0].span, Some((bytes, bytes + 1)));
}
