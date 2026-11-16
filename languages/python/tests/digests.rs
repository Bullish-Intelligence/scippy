//! The declaration digests of the Python pack, as the snapshot reports them.

use std::collections::BTreeMap;
use std::fs;

use scippy_core::{AnalysisSymbol, DeclRecord, IndexRequest, index_project_with_snapshot};
use scippy_python::PythonPack;

const BASE: &str = "\
class Box:
    def put(self, item, count=1):
        total = item * count
        return total


def top(a, b):
    \"\"\"Add.\"\"\"
    return a + b


x = 1
";

/// Index one file. Return its symbols by display name.
fn symbols(source: &str) -> BTreeMap<String, AnalysisSymbol> {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("m.py"), source).unwrap();
    let request = IndexRequest::new(&root, scratch.path().join("index.scip"));
    let (_, snapshot) = index_project_with_snapshot(&request, &[&PythonPack]).unwrap();
    snapshot.documents[0]
        .symbols
        .iter()
        .map(|s| (s.display_name.clone(), s.clone()))
        .collect()
}

fn decl(source: &str, name: &str) -> DeclRecord {
    symbols(source)
        .remove(name)
        .and_then(|s| s.decl)
        .unwrap_or_else(|| panic!("`{name}` has no decl record"))
}

#[test]
fn a_function_has_both_digests_and_a_count() {
    let top = decl(BASE, "top");
    assert_eq!(top.sig_digest.as_ref().unwrap().len(), 32);
    assert_eq!(top.body_digest.as_ref().unwrap().len(), 32);
    assert_ne!(top.sig_digest, top.body_digest);
    assert!(top.body_tokens.unwrap() >= 5);
    assert!(top.container.is_empty());
    assert_eq!(top.parent, None);
}

#[test]
fn whitespace_and_comments_keep_both_digests() {
    let edited = "\
# A comment before.
class Box:
    # A comment in the class.
    def put( self,item,  count = 1 ):  # A trailing comment.

        total = item*count   # Multiply.
        # A comment line.
        return   total



def top( a,b ):
    \"\"\"Add.\"\"\"
    # A comment in the body.
    return a+b


x = 1
";
    for name in ["Box", "put", "top"] {
        let (a, b) = (decl(BASE, name), decl(edited, name));
        assert_eq!(a, b, "{name}");
    }
}

#[test]
fn a_body_edit_changes_only_the_body_digest() {
    let edited = BASE.replace("return a + b", "return a - b");
    let (a, b) = (decl(BASE, "top"), decl(&edited, "top"));
    assert_eq!(a.sig_digest, b.sig_digest);
    assert_ne!(a.body_digest, b.body_digest);
    assert_eq!(a.body_tokens, b.body_tokens);
    // The class holds the edit in its body only when it holds the function.
    let edited = BASE.replace("return total", "return total + 1");
    let (a, b) = (decl(BASE, "put"), decl(&edited, "put"));
    assert_eq!(a.sig_digest, b.sig_digest);
    assert_ne!(a.body_digest, b.body_digest);
    let (a, b) = (decl(BASE, "Box"), decl(&edited, "Box"));
    assert_eq!(a.sig_digest, b.sig_digest);
    assert_ne!(a.body_digest, b.body_digest);
}

#[test]
fn a_parameter_change_changes_only_the_signature_digest() {
    let edited = BASE.replace("def top(a, b)", "def top(a, b, c)");
    let (a, b) = (decl(BASE, "top"), decl(&edited, "top"));
    assert_ne!(a.sig_digest, b.sig_digest);
    assert_eq!(a.body_digest, b.body_digest);
    assert_eq!(a.body_tokens, b.body_tokens);
    let edited = BASE.replace("count=1", "count=2");
    let (a, b) = (decl(BASE, "put"), decl(&edited, "put"));
    assert_ne!(a.sig_digest, b.sig_digest);
    assert_eq!(a.body_digest, b.body_digest);
}

#[test]
fn a_rename_keeps_both_digests() {
    let edited = BASE.replace("def top(", "def summed(");
    assert_eq!(decl(BASE, "top"), decl(&edited, "summed"));
    let edited = BASE.replace("def put(", "def add(");
    assert_eq!(decl(BASE, "put"), decl(&edited, "add"));
}

#[test]
fn a_decorator_is_outside_the_extent() {
    let plain = "def top(a):\n    return a\n";
    let decorated = "@cache\ndef top(a):\n    return a\n";
    assert_eq!(decl(plain, "top"), decl(decorated, "top"));
}

#[test]
fn a_variable_has_a_record_without_digests() {
    let x = decl(BASE, "x");
    assert!(x.container.is_empty());
    assert_eq!(
        (x.sig_digest, x.body_digest, x.body_tokens),
        (None, None, None)
    );
}

#[test]
fn a_method_and_its_parameter_carry_the_container_and_the_parent() {
    let all = symbols(BASE);
    let put = all["put"].decl.as_ref().unwrap();
    assert_eq!(put.container, ["Box"]);
    let item = all["item"].decl.as_ref().unwrap();
    assert_eq!(item.container, ["Box", "put"]);
    assert_eq!(item.parent.as_deref(), Some(all["put"].symbol.as_str()));
    assert_eq!(item.sig_digest, None);
    assert_eq!(item.body_digest, None);
    // A local variable has no record.
    assert!(all["total"].decl.is_none());
}

#[test]
fn a_parameter_of_a_nested_function_has_no_record() {
    let all = symbols("def outer():\n    def inner(p):\n        return p\n    return inner\n");
    assert!(all["outer"].decl.is_some());
    assert!(all["inner"].decl.is_none());
    assert!(all["p"].decl.is_none());
}

#[test]
fn a_warm_graph_cache_run_reports_the_same_records() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("m.py"), BASE).unwrap();
    let run = |output: &str| {
        let mut request = IndexRequest::new(&root, scratch.path().join(output));
        request.graph_cache = Some(scratch.path().join("graphs.sqlite"));
        let (report, snapshot) = index_project_with_snapshot(&request, &[&PythonPack]).unwrap();
        let decls: Vec<_> = snapshot.documents[0]
            .symbols
            .iter()
            .map(|s| (s.symbol.clone(), s.decl.clone()))
            .collect();
        (report, decls)
    };
    let (cold, first) = run("cold.scip");
    let (warm, second) = run("warm.scip");
    assert_eq!(cold.graph_cache_hits, 0);
    assert!(warm.graph_cache_hits > 0 && warm.graph_cache_misses == 0);
    assert!(first.iter().any(|(_, d)| d.is_some()));
    assert_eq!(first, second);
}
