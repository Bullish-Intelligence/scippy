//! The declaration digests of the TypeScript pack, as the snapshot reports them.

use std::collections::BTreeMap;
use std::fs;

use scippy_core::{AnalysisSymbol, DeclRecord, IndexRequest, index_project_with_snapshot};
use scippy_typescript::TypeScriptPack;

const BASE: &str = "\
export class Box {
  put(item: number, count = 1): number {
    const total = item * count;
    return total;
  }
}

export function top(a: number, b: number): number {
  return a + b;
}

export const x = 1;
";

/// Index one file. Return its symbols by display name.
fn symbols(source: &str) -> BTreeMap<String, AnalysisSymbol> {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("m.ts"), source).unwrap();
    let request = IndexRequest::new(&root, scratch.path().join("index.scip"));
    let (_, snapshot) = index_project_with_snapshot(&request, &[&TypeScriptPack]).unwrap();
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
}

#[test]
fn whitespace_and_comments_keep_both_digests() {
    let edited = "\
// A comment before.
export class Box {
  /* A comment in the class. */
  put( item : number,count=1 ) : number {   // Trailing.

    const total=item*count; /* Multiply. */
    // A comment line.
    return   total ;
  }
}



export function top(a:number,
                    b: number): number {
  // A comment in the body.
  return a+b;
}

export const x = 1;
";
    for name in ["Box", "put", "top"] {
        assert_eq!(decl(BASE, name), decl(edited, name), "{name}");
    }
}

#[test]
fn a_body_edit_changes_only_the_body_digest() {
    let edited = BASE.replace("return a + b", "return a - b");
    let (a, b) = (decl(BASE, "top"), decl(&edited, "top"));
    assert_eq!(a.sig_digest, b.sig_digest);
    assert_ne!(a.body_digest, b.body_digest);
    assert_eq!(a.body_tokens, b.body_tokens);
    let edited = BASE.replace("return total;", "return total + 1;");
    let (a, b) = (decl(BASE, "put"), decl(&edited, "put"));
    assert_eq!(a.sig_digest, b.sig_digest);
    assert_ne!(a.body_digest, b.body_digest);
    let (a, b) = (decl(BASE, "Box"), decl(&edited, "Box"));
    assert_eq!(a.sig_digest, b.sig_digest);
    assert_ne!(a.body_digest, b.body_digest);
}

#[test]
fn a_parameter_change_changes_only_the_signature_digest() {
    let edited = BASE.replace("top(a: number, b: number)", "top(a: number, b: string)");
    let (a, b) = (decl(BASE, "top"), decl(&edited, "top"));
    assert_ne!(a.sig_digest, b.sig_digest);
    assert_eq!(a.body_digest, b.body_digest);
    assert_eq!(a.body_tokens, b.body_tokens);
    let edited = BASE.replace("count = 1", "count = 2");
    let (a, b) = (decl(BASE, "put"), decl(&edited, "put"));
    assert_ne!(a.sig_digest, b.sig_digest);
    assert_eq!(a.body_digest, b.body_digest);
}

#[test]
fn a_rename_keeps_both_digests() {
    let edited = BASE.replace("function top(", "function summed(");
    assert_eq!(decl(BASE, "top"), decl(&edited, "summed"));
    let edited = BASE.replace("  put(", "  add(");
    assert_eq!(decl(BASE, "put"), decl(&edited, "add"));
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
    assert_eq!(all["put"].decl.as_ref().unwrap().container, ["Box"]);
    let item = all["item"].decl.as_ref().unwrap();
    assert_eq!(item.container, ["Box", "put"]);
    assert_eq!(item.parent.as_deref(), Some(all["put"].symbol.as_str()));
    assert_eq!(item.sig_digest, None);
    assert!(all["total"].decl.is_none());
}

#[test]
fn a_parameter_of_a_nested_function_has_no_record() {
    let all = symbols(
        "export function outer() {\n  function inner(p: number) { return p; }\n  return inner;\n}\n",
    );
    assert!(all["outer"].decl.is_some());
    assert!(all["inner"].decl.is_none());
    assert!(all["p"].decl.is_none());
}
