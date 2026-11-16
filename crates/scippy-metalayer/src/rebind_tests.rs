//! Tests of the rebinding engine. They publish snapshots and read the identity tables.

use std::path::PathBuf;

use rusqlite::Connection;
use scippy_core::{AnalysisDocument, AnalysisSnapshot, AnalysisSymbol, DeclRecord};

use crate::identity::{Identity, IdentityOptions};
use crate::publish::sha256_hex;
use crate::*;

/// One declaration of a test file.
#[derive(Clone)]
struct Sym {
    desc: String,
    name: String,
    kind: &'static str,
    container: Vec<String>,
    sig: Option<String>,
    body: Option<String>,
    tokens: Option<u32>,
    parent: Option<String>,
}

fn func(desc: &str, name: &str) -> Sym {
    Sym {
        desc: desc.into(),
        name: name.into(),
        kind: "Function",
        container: Vec::new(),
        sig: Some(format!("sig-{name}")),
        body: Some(format!("body-{name}")),
        tokens: Some(20),
        parent: None,
    }
}

fn variable(desc: &str, name: &str) -> Sym {
    Sym {
        kind: "Variable",
        sig: None,
        body: None,
        tokens: None,
        ..func(desc, name)
    }
}

fn param(parent: &str, desc: &str, name: &str) -> Sym {
    Sym {
        kind: "Parameter",
        parent: Some(parent.into()),
        ..variable(desc, name)
    }
}

fn module(desc: &str, name: &str) -> Sym {
    Sym {
        kind: "Module",
        ..variable(desc, name)
    }
}

impl Sym {
    fn sig(mut self, v: &str) -> Self {
        self.sig = Some(v.into());
        self
    }
    fn body(mut self, v: &str) -> Self {
        self.body = Some(v.into());
        self
    }
    fn tokens(mut self, n: u32) -> Self {
        self.tokens = Some(n);
        self
    }
    fn within(mut self, container: &[&str]) -> Self {
        self.container = container.iter().map(|s| (*s).into()).collect();
        self
    }
}

/// One file of a test run.
#[derive(Clone)]
struct File {
    path: String,
    source: String,
    syms: Vec<Sym>,
}

fn file(path: &str, syms: Vec<Sym>) -> File {
    File {
        path: path.into(),
        source: String::new(),
        syms,
    }
}

impl File {
    fn source(mut self, text: &str) -> Self {
        self.source = text.into();
        self
    }
}

/// A key as the engine builds it for a test symbol.
fn key(desc: &str) -> String {
    format!("scip-python python pkg . {desc}")
}

struct World {
    dir: tempfile::TempDir,
    layer: Metalayer,
    ids: FixedIds,
    ver: String,
    tag: String,
    /// The version is part of the options of the run. A test turns it off.
    ver_in_options: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Row {
    id: String,
    kind: String,
    path: String,
    name: String,
    key: String,
    outcome: Option<String>,
    tier: Option<String>,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let layer = Metalayer::open(dir.path().join("layer.sqlite")).unwrap();
        Self {
            dir,
            layer,
            ids: FixedIds::default(),
            ver: "1.0".into(),
            tag: String::new(),
            ver_in_options: true,
        }
    }

    fn identity_path(&self) -> PathBuf {
        self.dir.path().join("layer.identity.db")
    }

    fn snapshot(&self, files: &[File]) -> AnalysisSnapshot {
        let documents = files
            .iter()
            .map(|f| {
                // The text of a file follows its declarations, unless a test sets it.
                let source = if f.source.is_empty() {
                    f.syms
                        .iter()
                        .map(|s| {
                            format!(
                                "{} {} {:?} {:?} {:?} {:?} {:?}\n",
                                s.desc, s.name, s.container, s.sig, s.body, s.tokens, s.parent
                            )
                        })
                        .collect()
                } else {
                    f.source.clone()
                };
                (f, source)
            })
            .map(|(f, source)| AnalysisDocument {
                path: f.path.clone(),
                language: "Python".into(),
                pack: Some("python".into()),
                real_path: None,
                sha256: Some(sha256_hex(source.as_bytes())),
                source,
                stale: false,
                occurrences: Vec::new(),
                symbols: f
                    .syms
                    .iter()
                    .map(|s| {
                        let mut decl = DeclRecord::default();
                        decl.container = s.container.clone();
                        decl.parent = s
                            .parent
                            .as_ref()
                            .map(|p| format!("scip-python python pkg {} {p}", self.ver));
                        decl.sig_digest = s.sig.clone();
                        decl.body_digest = s.body.clone();
                        decl.body_tokens = s.tokens;
                        AnalysisSymbol {
                            symbol: format!("scip-python python pkg {} {}", self.ver, s.desc),
                            kind: s.kind.into(),
                            display_name: s.name.clone(),
                            implements: Vec::new(),
                            references: Vec::new(),
                            decl: Some(decl),
                        }
                    })
                    .collect(),
            })
            .collect();
        AnalysisSnapshot {
            documents,
            lost_files: Vec::new(),
            references: Vec::new(),
            edges: Vec::new(),
            external_symbols: Vec::new(),
            diagnostics: Vec::new(),
            input_complete: true,
            analysis_exact: true,
            bounded_references: 0,
            scip: b"scip".to_vec(),
            tool_version: "test".into(),
            project_root: "/project".into(),
            shards: Vec::new(),
            options: if self.ver_in_options {
                format!("{}/{}", self.ver, self.tag)
            } else {
                self.tag.clone()
            },
        }
    }

    fn publish(&mut self, files: &[File]) -> RebindReport {
        let snapshot = self.snapshot(files);
        let identity = self.identity_path();
        self.layer
            .publish_with_identity_ids(&snapshot, &PublishOptions::default(), &identity, &self.ids)
            .unwrap()
            .rebind
            .expect("a rebind report")
    }

    fn db(&self) -> Connection {
        Connection::open(self.identity_path()).unwrap()
    }

    fn rows(&self) -> Vec<Row> {
        let db = self.db();
        let mut st = db
            .prepare(
                "SELECT e.id, e.kind, l.path, l.name, l.key, o.outcome, o.tier
                 FROM entity e JOIN locator l ON l.entity_id = e.id AND l.seq = 0
                 LEFT JOIN outcome o ON o.entity_id = e.id ORDER BY e.id",
            )
            .unwrap();
        st.query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                kind: r.get(1)?,
                path: r.get(2)?,
                name: r.get(3)?,
                key: r.get(4)?,
                outcome: r.get(5)?,
                tier: r.get(6)?,
            })
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    /// The row of the declaration with this descriptor.
    fn row(&self, desc: &str) -> Row {
        let want = key(desc);
        let mut found: Vec<Row> = self.rows().into_iter().filter(|r| r.key == want).collect();
        assert_eq!(found.len(), 1, "one entity for {desc}");
        found.remove(0)
    }

    /// The row of a parameter by the key of its parent.
    fn param(&self, parent: &str, name: &str) -> Row {
        let parent = self.row(parent);
        let want = format!("{}/{name}", parent.id);
        let mut found: Vec<Row> = self.rows().into_iter().filter(|r| r.key == want).collect();
        assert_eq!(found.len(), 1, "one entity for parameter {name}");
        found.remove(0)
    }

    /// `matched`, `moved:T3`, `ambiguous:T2`, or `orphaned`.
    fn state(&self, desc: &str) -> String {
        state_of(&self.row(desc))
    }

    fn count(&self, table: &str) -> i64 {
        self.db()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// Every row of the entity, locator, and outcome tables.
    fn dump(&self) -> Vec<String> {
        let db = self.db();
        let mut out = Vec::new();
        for (table, sql) in [
            (
                "entity",
                "SELECT id, family, kind, ifnull(parent_id,''), created_at FROM entity ORDER BY id",
            ),
            (
                "locator",
                "SELECT entity_id, seq, key, ifnull(symbol,''), path, name, container, ifnull(sig_digest,''), ifnull(body_digest,''), input_digest FROM locator ORDER BY entity_id, seq",
            ),
            (
                "outcome",
                "SELECT entity_id, outcome, ifnull(tier,''), ifnull(candidates,''), since_digest FROM outcome ORDER BY entity_id",
            ),
        ] {
            let mut st = db.prepare(sql).unwrap();
            let n = st.column_count();
            let rows = st
                .query_map([], |r| {
                    (0..n)
                        .map(|i| r.get_ref(i).map(|v| format!("{v:?}")))
                        .collect::<Result<Vec<_>, _>>()
                        .map(|v| format!("{table} {}", v.join(" | ")))
                })
                .unwrap();
            out.extend(rows.map(Result::unwrap));
        }
        out
    }

    fn candidates(&self, desc: &str) -> Vec<String> {
        let id = self.row(desc).id;
        let text: Option<String> = self
            .db()
            .query_row(
                "SELECT candidates FROM outcome WHERE entity_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        text.map(|t| serde_json::from_str(&t).unwrap())
            .unwrap_or_default()
    }

    fn store(&self) -> Identity {
        Identity::open(self.identity_path(), &IdentityOptions::default()).unwrap()
    }
}

fn state_of(row: &Row) -> String {
    match (&row.outcome, &row.tier) {
        (None, _) => "matched".into(),
        (Some(o), Some(t)) => format!("{o}:{t}"),
        (Some(o), None) => o.clone(),
    }
}

fn ids_of(world: &World) -> Vec<String> {
    world.rows().into_iter().map(|r| r.id).collect()
}

// ---- Keys ----

#[test]
fn the_key_removes_the_package_version() {
    assert_eq!(
        symbol_key("scip-python python pkg 1.2.3 mod/f().").unwrap(),
        "scip-python python pkg . mod/f()."
    );
    assert_eq!(
        symbol_key("scip-python python pkg . mod/f().").unwrap(),
        "scip-python python pkg . mod/f()."
    );
    // A double space is a space inside a field.
    assert_eq!(
        symbol_key("scip-python python my  pkg 1  0 mod/`a b`#").unwrap(),
        "scip-python python my  pkg . mod/`a b`#"
    );
    assert_eq!(symbol_key("local 3"), None);
    assert_eq!(symbol_key("not a symbol"), None);
}

// ---- Tiers ----

#[test]
fn a_version_bump_binds_every_declaration_by_t1() {
    let mut w = World::new();
    let files = |_: ()| {
        vec![file(
            "a.py",
            vec![
                module("a/__init__:", "a"),
                func("a/f().", "f"),
                func("a/C#", "C"),
                variable("a/x.", "x"),
            ],
        )]
    };
    let first = w.publish(&files(()));
    assert_eq!(first.minted, 4);
    let before = ids_of(&w);
    w.ver = "2.0".into();
    let second = w.publish(&files(()));
    assert_eq!(second.bound.get("T1"), Some(&4));
    assert_eq!(second.minted, 0);
    assert_eq!(ids_of(&w), before);
    assert_eq!(w.count("outcome"), 0);
    // A version bump writes no locator, and it updates the symbol in place.
    assert_eq!(w.count("locator"), 4);
    let symbol: String = w
        .db()
        .query_row("SELECT symbol FROM locator WHERE name = 'f'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(symbol.contains(" 2.0 "));
}

#[test]
fn publishing_the_same_input_twice_changes_nothing() {
    let mut w = World::new();
    let files = vec![file(
        "a.py",
        vec![func("a/f().", "f"), variable("a/x.", "x")],
    )];
    w.publish(&files);
    let before = w.dump();
    let again = w.publish(&files);
    assert!(again.skipped);
    assert_eq!(w.dump(), before);
}

#[test]
fn a_new_digest_with_the_same_declarations_writes_no_entity_row() {
    // An edit of a comment changes the file hash, but no digest of a declaration.
    let mut w = World::new();
    let one = vec![file("a.py", vec![func("a/f().", "f"), variable("a/x.", "x")]).source("1")];
    let two = vec![file("a.py", vec![func("a/f().", "f"), variable("a/x.", "x")]).source("2")];
    w.publish(&one);
    let before = w.dump();
    let report = w.publish(&two);
    assert!(!report.skipped);
    assert_eq!(report.bound.get("T1"), Some(&2));
    assert_eq!(w.dump(), before);
}

#[test]
fn an_inserted_declaration_mints_and_the_old_ones_keep_their_ids() {
    let mut w = World::new();
    w.publish(&[file("a.py", vec![func("a/f().", "f"), func("a/g().", "g")])]);
    let before = w.row("a/f().").id;
    let report = w.publish(&[file(
        "a.py",
        vec![
            func("a/h().", "h"),
            func("a/g().", "g"),
            func("a/f().", "f"),
        ],
    )]);
    assert_eq!(report.minted, 1);
    assert_eq!(w.row("a/f().").id, before);
    assert_eq!(w.state("a/f()."), "matched");
    assert_eq!(w.state("a/h()."), "matched");
}

#[test]
fn t1h_resurrects_the_ids_after_a_branch_switch() {
    let mut w = World::new();
    let a = vec![file("a.py", vec![func("a/f().", "f"), func("a/g().", "g")])];
    let b = vec![file(
        "a.py",
        vec![func("a/h().", "h").sig("other").body("other")],
    )];
    w.publish(&a);
    let (f, g) = (w.row("a/f().").id, w.row("a/g().").id);
    w.publish(&b);
    assert_eq!(w.state("a/f()."), "orphaned");
    assert_eq!(w.state("a/g()."), "orphaned");
    let h = w.row("a/h().").id;
    let back = w.publish(&a);
    assert_eq!(back.bound.get("T1"), Some(&2));
    assert_eq!(w.row("a/f().").id, f);
    assert_eq!(w.row("a/g().").id, g);
    assert_eq!(w.state("a/f()."), "matched");
    assert_eq!(w.state("a/h()."), "orphaned");
    assert_eq!(w.row("a/h().").id, h);
}

#[test]
fn t1h_binds_a_key_that_an_older_locator_held() {
    // f moves to b.py by T3, then a later run puts the old key back under another path.
    let mut w = World::new();
    w.publish(&[
        file("a.py", vec![func("a/f().", "f"), func("a/pad().", "pad")]),
        file("b.py", vec![func("b/other().", "other")]),
    ]);
    let id = w.row("a/f().").id;
    w.publish(&[
        file("a.py", vec![func("a/pad().", "pad")]),
        file(
            "b.py",
            vec![func("b/f().", "f"), func("b/other().", "other")],
        ),
    ]);
    assert_eq!(w.state("b/f()."), "moved:T3");
    w.publish(&[
        file("a.py", vec![func("a/f().", "f"), func("a/pad().", "pad")]),
        file("b.py", vec![func("b/other().", "other")]),
    ]);
    let row = w.row("a/f().");
    assert_eq!(row.id, id);
    // The entity is still flagged: a later match does not clear `moved`.
    assert_eq!(state_of(&row), "moved:T3");
    assert_eq!(w.count("locator"), 5);
}

#[test]
fn t2_binds_the_same_path_name_and_kind_with_another_container() {
    let mut w = World::new();
    w.publish(&[file("a.py", vec![func("a/f().", "f")])]);
    let id = w.row("a/f().").id;
    let report = w.publish(&[file(
        "a.py",
        vec![func("a/C#f().", "f").within(&["C"]).sig("s2").body("b2")],
    )]);
    assert_eq!(report.bound.get("T2"), Some(&1));
    let row = w.row("a/C#f().");
    assert_eq!(row.id, id);
    assert_eq!(state_of(&row), "moved:T2");
    assert_eq!(w.count("locator"), 2);
}

#[test]
fn t2_with_two_claimants_leaves_both_ambiguous_and_mints() {
    let mut w = World::new();
    w.publish(&[file(
        "a.py",
        vec![
            func("a/C#f().", "f").within(&["C"]),
            func("a/D#f().", "f").within(&["D"]),
        ],
    )]);
    let report = w.publish(&[file("a.py", vec![func("a/E#f().", "f").within(&["E"])])]);
    assert_eq!(report.ambiguous, 2);
    assert_eq!(report.minted, 1);
    assert_eq!(w.state("a/C#f()."), "ambiguous:T2");
    assert_eq!(w.state("a/D#f()."), "ambiguous:T2");
    assert_eq!(w.candidates("a/C#f()."), vec![key("a/E#f().")]);
    assert_eq!(w.state("a/E#f()."), "matched");
}

#[test]
fn t2_narrows_by_the_signature_digest() {
    let mut w = World::new();
    w.publish(&[file(
        "a.py",
        vec![
            func("a/C#f().", "f").within(&["C"]).sig("one"),
            func("a/D#f().", "f").within(&["D"]).sig("two"),
        ],
    )]);
    let one = w.row("a/C#f().").id;
    w.publish(&[file(
        "a.py",
        vec![func("a/E#f().", "f").within(&["E"]).sig("one")],
    )]);
    assert_eq!(w.row("a/E#f().").id, one);
    assert_eq!(state_of(&w.row("a/E#f().")), "moved:T2");
    assert_eq!(w.state("a/D#f()."), "orphaned");
}

#[test]
fn a_vetoed_pair_does_not_bind() {
    let mut w = World::new();
    w.publish(&[file("a.py", vec![func("a/f().", "f")])]);
    let id = w.row("a/f().").id;
    w.store().add_veto(&id, &key("a/C#f().")).unwrap();
    let report = w.publish(&[file("a.py", vec![func("a/C#f().", "f").within(&["C"])])]);
    assert_eq!(report.bound_total(), 0);
    assert_eq!(report.minted, 1);
    assert_ne!(w.row("a/C#f().").id, id);
    assert_eq!(w.state("a/f()."), "orphaned");
}

#[test]
fn a_veto_blocks_t1_too() {
    let mut w = World::new();
    w.publish(&[file("a.py", vec![func("a/f().", "f")])]);
    let id = w.row("a/f().").id;
    w.store().add_veto(&id, &key("a/f().")).unwrap();
    w.tag = "x".into();
    let report = w.publish(&[file("a.py", vec![func("a/f().", "f")])]);
    assert_eq!(report.minted, 1);
    assert_eq!(w.rows().len(), 2);
}

#[test]
fn t3_binds_a_declaration_that_moves_to_another_file() {
    let mut w = World::new();
    w.publish(&[
        file("a.py", vec![func("a/f().", "f"), func("a/g().", "g")]),
        file("b.py", vec![func("b/h().", "h")]),
    ]);
    let id = w.row("a/f().").id;
    let report = w.publish(&[
        file("a.py", vec![func("a/g().", "g")]),
        file(
            "b.py",
            vec![func("b/f().", "f").body("edited"), func("b/h().", "h")],
        ),
    ]);
    assert_eq!(report.bound.get("T3"), Some(&1));
    let row = w.row("b/f().");
    assert_eq!(row.id, id);
    assert_eq!(state_of(&row), "moved:T3");
    assert_eq!(row.path, "b.py");
}

#[test]
fn t3_with_equal_content_in_two_files_is_ambiguous() {
    let mut w = World::new();
    let same = |d: &str| func(d, "f").sig("s").body("b");
    w.publish(&[
        file("a.py", vec![same("a/f().")]),
        file("b.py", vec![same("b/f().")]),
        file("c.py", vec![func("c/k().", "k")]),
    ]);
    let report = w.publish(&[
        file("a.py", vec![func("a/z().", "z")]),
        file("b.py", vec![func("b/y().", "y")]),
        file("c.py", vec![func("c/k().", "k"), same("c/f().")]),
    ]);
    assert_eq!(report.ambiguous, 2);
    assert_eq!(w.state("a/f()."), "ambiguous:T3");
    assert_eq!(w.state("b/f()."), "ambiguous:T3");
    assert_eq!(w.candidates("a/f()."), vec![key("c/f().")]);
    assert_eq!(w.state("c/f()."), "matched");
}

#[test]
fn t4_binds_a_renamed_declaration_in_another_file_by_its_body() {
    let mut w = World::new();
    w.publish(&[
        file(
            "a.py",
            vec![func("a/f().", "f").body("big"), func("a/p().", "p")],
        ),
        file("b.py", vec![func("b/q().", "q")]),
    ]);
    let id = w.row("a/f().").id;
    let report = w.publish(&[
        file("a.py", vec![func("a/p().", "p")]),
        file(
            "b.py",
            vec![func("b/q().", "q"), func("b/g().", "g").body("big")],
        ),
    ]);
    assert_eq!(report.bound.get("T4"), Some(&1));
    assert_eq!(w.row("b/g().").id, id);
    assert_eq!(state_of(&w.row("b/g().")), "moved:T4");
}

#[test]
fn t4_needs_the_minimum_body_size() {
    let mut w = World::new();
    w.publish(&[
        file(
            "a.py",
            vec![
                func("a/f().", "f")
                    .body("big")
                    .tokens(MIN_BODY_TOKENS as u32 - 1),
            ],
        ),
        file("b.py", vec![func("b/q().", "q")]),
    ]);
    let id = w.row("a/f().").id;
    w.publish(&[
        file("a.py", vec![]),
        file(
            "b.py",
            vec![
                func("b/q().", "q"),
                func("b/g().", "g")
                    .body("big")
                    .tokens(MIN_BODY_TOKENS as u32 - 1),
            ],
        ),
    ]);
    assert_ne!(w.row("b/g().").id, id);
    assert_eq!(w.state("a/f()."), "orphaned");
}

#[test]
fn t5_binds_a_rename_inside_a_file() {
    let mut w = World::new();
    w.publish(&[file("a.py", vec![func("a/f().", "f").body("big")])]);
    let id = w.row("a/f().").id;
    let report = w.publish(&[file(
        "a.py",
        vec![func("a/g().", "g").body("big").sig("sig2")],
    )]);
    assert_eq!(report.bound.get("T5"), Some(&1));
    assert_eq!(w.row("a/g().").id, id);
    assert_eq!(state_of(&w.row("a/g().")), "moved:T5");
}

#[test]
fn a_variable_skips_t3_and_t4() {
    let mut w = World::new();
    let var = |d: &str| Sym {
        sig: Some("s".into()),
        body: Some("b".into()),
        tokens: Some(50),
        ..variable(d, "x")
    };
    w.publish(&[
        file("a.py", vec![var("a/x.")]),
        file("b.py", vec![func("b/k().", "k")]),
    ]);
    let id = w.row("a/x.").id;
    w.publish(&[
        file("a.py", vec![]),
        file("b.py", vec![func("b/k().", "k"), var("b/x.")]),
    ]);
    assert_ne!(w.row("b/x.").id, id);
    assert_eq!(w.state("a/x."), "orphaned");
}

#[test]
fn a_renamed_file_binds_by_the_file_hash_including_variables_and_the_module() {
    let mut w = World::new();
    let make = |path: &str, stem: &str| {
        file(
            path,
            vec![
                module(&format!("{stem}/__init__:"), stem),
                func(&format!("{stem}/f()."), "f"),
                variable(&format!("{stem}/x."), "x"),
                variable(&format!("{stem}/y."), "y"),
            ],
        )
        .source("same text")
    };
    w.publish(&[make("a.py", "a")]);
    let ids = ids_of(&w);
    let report = w.publish(&[make("b.py", "b")]);
    assert_eq!(report.bound.get("T2f"), Some(&4));
    assert_eq!(report.minted, 0);
    assert_eq!(ids_of(&w), ids);
    for desc in ["b/__init__:", "b/f().", "b/x.", "b/y."] {
        assert_eq!(w.state(desc), "moved:T2f");
        assert_eq!(w.row(desc).path, "b.py");
    }
}

#[test]
fn a_renamed_and_edited_file_pairs_by_the_probe() {
    let mut w = World::new();
    let make = |path: &str, stem: &str, source: &str| {
        file(
            path,
            vec![
                func(&format!("{stem}/f()."), "f"),
                func(&format!("{stem}/g()."), "g"),
                variable(&format!("{stem}/x."), "x"),
            ],
        )
        .source(source)
    };
    w.publish(&[
        make("a.py", "a", "one"),
        file("c.py", vec![func("c/k().", "k")]),
    ]);
    let x = w.row("a/x.").id;
    let report = w.publish(&[
        make("b.py", "b", "two"),
        file("c.py", vec![func("c/k().", "k")]),
    ]);
    assert_eq!(report.bound.get("T2f"), Some(&3));
    assert_eq!(w.row("b/x.").id, x);
    assert_eq!(w.state("b/x."), "moved:T2f");
}

#[test]
fn one_hit_is_not_enough_for_the_probe() {
    let mut w = World::new();
    let make = |path: &str, stem: &str, source: &str| {
        file(
            path,
            vec![
                func(&format!("{stem}/f()."), "f"),
                variable(&format!("{stem}/x."), "x"),
            ],
        )
        .source(source)
    };
    w.publish(&[make("a.py", "a", "one")]);
    let x = w.row("a/x.").id;
    w.publish(&[make("b.py", "b", "two")]);
    // The function binds by T3 and the variable has no pair.
    assert_eq!(w.state("b/f()."), "moved:T3");
    assert_ne!(w.row("b/x.").id, x);
    assert_eq!(w.state("a/x."), "orphaned");
}

#[test]
fn a_copied_file_mints_new_ids_and_the_original_keeps_its_own() {
    let mut w = World::new();
    let make = |path: &str, stem: &str| {
        file(
            path,
            vec![
                func(&format!("{stem}/f()."), "f"),
                variable(&format!("{stem}/x."), "x"),
            ],
        )
        .source("same text")
    };
    w.publish(&[make("a.py", "a")]);
    let ids = ids_of(&w);
    let report = w.publish(&[make("a.py", "a"), make("b.py", "b")]);
    assert_eq!(report.minted, 2);
    assert_eq!(report.bound.get("T1"), Some(&2));
    for id in &ids {
        assert!(ids_of(&w).contains(id));
    }
    assert_eq!(w.state("a/f()."), "matched");
    assert_ne!(w.row("b/f().").id, w.row("a/f().").id);
}

// ---- Parameters ----

#[test]
fn a_parameter_binds_when_the_parent_binds_and_the_name_is_equal() {
    let mut w = World::new();
    let make = |params: &[&str], sig: &str| {
        let mut syms = vec![func("a/f().", "f").sig(sig).within(&[])];
        for p in params {
            syms.push(param("a/f().", &format!("a/f().({p})"), p).within(&["f"]));
        }
        vec![file("a.py", syms)]
    };
    w.publish(&make(&["a", "b"], "s1"));
    let a = w.param("a/f().", "a").id;
    let b = w.param("a/f().", "b").id;
    let report = w.publish(&make(&["a", "c"], "s2"));
    assert_eq!(report.bound.get("T1"), Some(&1));
    assert_eq!(report.bound.get("Param"), Some(&1));
    assert_eq!(w.param("a/f().", "a").id, a);
    assert_eq!(state_of(&w.param("a/f().", "a")), "matched");
    // A renamed parameter orphans, and the new name mints.
    let old = w.rows().into_iter().find(|r| r.id == b).unwrap();
    assert_eq!(state_of(&old), "orphaned");
    assert_ne!(w.param("a/f().", "c").id, b);
}

#[test]
fn a_parameter_follows_a_moved_parent_without_an_outcome_of_its_own() {
    let mut w = World::new();
    let make = |name: &str| {
        vec![file(
            "a.py",
            vec![
                func(&format!("a/{name}()."), name).body("big"),
                param(&format!("a/{name}()."), &format!("a/{name}().(p)"), "p").within(&[name]),
            ],
        )]
    };
    w.publish(&make("f"));
    let p = w.param("a/f().", "p").id;
    let report = w.publish(&make("g"));
    assert_eq!(report.bound.get("T5"), Some(&1));
    assert_eq!(w.state("a/g()."), "moved:T5");
    let after = w.param("a/g().", "p");
    assert_eq!(after.id, p);
    assert_eq!(state_of(&after), "matched");
}

#[test]
fn a_parameter_of_an_orphaned_parent_is_orphaned() {
    let mut w = World::new();
    let with = vec![file(
        "a.py",
        vec![
            func("a/f().", "f"),
            param("a/f().", "a/f().(p)", "p").within(&["f"]),
        ],
    )];
    w.publish(&with);
    let p = w.param("a/f().", "p").id;
    w.publish(&[file(
        "a.py",
        vec![func("a/other().", "other").sig("o").body("o")],
    )]);
    let row = w.rows().into_iter().find(|r| r.id == p).unwrap();
    assert_eq!(state_of(&row), "orphaned");
    // The parent comes back, and so does the parameter.
    w.publish(&with);
    assert_eq!(w.param("a/f().", "p").id, p);
    assert_eq!(state_of(&w.param("a/f().", "p")), "matched");
}

// ---- Rules of the update and the state ----

#[test]
fn a_moved_entity_stays_flagged_after_a_later_t1_match() {
    let mut w = World::new();
    w.publish(&[file("a.py", vec![func("a/f().", "f").body("big")])]);
    w.publish(&[file("a.py", vec![func("a/g().", "g").body("big")])]);
    assert_eq!(w.state("a/g()."), "moved:T5");
    w.tag = "next".into();
    w.publish(&[file("a.py", vec![func("a/g().", "g").body("big")])]);
    assert_eq!(w.state("a/g()."), "moved:T5");
}

#[test]
fn a_t1_match_with_a_new_path_pushes_a_locator_and_the_history_keeps_16() {
    let mut w = World::new();
    let at = |path: &str| file(path, vec![func("a/f().", "f")]);
    w.publish(&[at("a.py")]);
    let id = w.row("a/f().").id;
    for i in 0..20 {
        let path = if i % 2 == 0 { "b.py" } else { "a.py" };
        w.publish(&[at(path)]);
    }
    let n: i64 = w
        .db()
        .query_row(
            "SELECT count(*) FROM locator WHERE entity_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 16);
    assert_eq!(w.rows().len(), 1);
}

#[test]
fn an_unannotated_orphan_goes_after_100_publishes_and_an_annotated_one_stays() {
    let mut w = World::new();
    w.publish(&[file(
        "a.py",
        vec![
            func("a/f().", "f"),
            func("a/g().", "g").sig("sg").body("bg"),
        ],
    )]);
    let annotated = w.row("a/g().").id;
    {
        let mut store = w.store();
        let a = crate::identity::Annotation {
            id: "note-1".into(),
            entity_id: annotated.clone(),
            created_at: "2026-01-01T00:00:00Z".into(),
            author: None,
            tag: None,
            body: "keep".into(),
            snapshot: "{}".into(),
        };
        store.insert_annotation(&a).unwrap();
    }
    w.publish(&[file("a.py", vec![func("a/h().", "h").sig("sh").body("bh")])]);
    assert_eq!(w.rows().len(), 3);
    for i in 0..99 {
        w.tag = format!("t{i}");
        w.publish(&[file("a.py", vec![func("a/h().", "h").sig("sh").body("bh")])]);
    }
    assert_eq!(w.rows().len(), 3, "still there 99 publishes later");
    w.tag = "last".into();
    let report = w.publish(&[file("a.py", vec![func("a/h().", "h").sig("sh").body("bh")])]);
    assert_eq!(report.collected, 1);
    let left: Vec<String> = w.rows().into_iter().map(|r| r.name).collect();
    assert_eq!(left, vec!["g", "h"]);
}

#[test]
fn the_engine_restores_the_ids_after_a_failure_between_the_two_commits() {
    let mut w = World::new();
    let one = vec![file("a.py", vec![func("a/f().", "f")])];
    w.publish(&one);
    let id = w.row("a/f().").id;
    // The identity store commits for the second run, and the facts commit never happens.
    w.ver = "2.0".into();
    let two = vec![file("a.py", vec![func("a/f().", "f"), func("a/g().", "g")])];
    let snapshot = w.snapshot(&two);
    let digest = "lost-facts-digest";
    let input = RebindInput::from_snapshot(&snapshot, digest, None);
    rebind(&mut w.store(), &input, &w.ids).unwrap();
    // The next publish still holds the old facts. The first run publishes again with the
    // old input: every entity binds by T1, and no id changes.
    w.ver = "1.0".into();
    let again = w.publish(&one);
    assert_eq!(again.bound.get("T1"), Some(&1));
    assert_eq!(w.row("a/f().").id, id);
    assert_eq!(w.state("a/f()."), "matched");
}

#[test]
fn the_identity_store_has_its_own_root_set_guard() {
    let mut w = World::new();
    w.publish(&[file("a.py", vec![func("a/f().", "f")])]);
    let id = w.row("a/f().").id;
    // A second facts database of another project, and the same identity store.
    let mut other = Metalayer::open(w.dir.path().join("other.sqlite")).unwrap();
    let mut s = w.snapshot(&[file("a.py", vec![func("a/f().", "f")])]);
    s.project_root = "/elsewhere".into();
    let path = w.identity_path();
    let refused = other.publish_with_identity_ids(&s, &PublishOptions::default(), &path, &w.ids);
    assert!(matches!(
        refused,
        Err(scippy_core::ScippyError::RootSetMismatch { .. })
    ));
    let options = PublishOptions {
        replace_root_set: true,
        ..PublishOptions::default()
    };
    let report = other
        .publish_with_identity_ids(&s, &options, &path, &w.ids)
        .unwrap()
        .rebind
        .unwrap();
    assert_eq!(report.bound.get("T1"), Some(&1));
    assert_eq!(w.row("a/f().").id, id);
}

#[test]
fn a_publish_without_an_identity_store_makes_none() {
    let mut w = World::new();
    let s = w.snapshot(&[file("a.py", vec![func("a/f().", "f")])]);
    let published = w.layer.publish(&s, &PublishOptions::default()).unwrap();
    assert!(published.rebind.is_none());
    assert!(!w.identity_path().exists());
}

#[test]
fn an_orphan_row_is_not_rewritten_by_the_next_run() {
    let mut w = World::new();
    w.publish(&[file("a.py", vec![func("a/f().", "f")])]);
    w.publish(&[file("a.py", vec![func("a/h().", "h").sig("x").body("x")])]);
    let since = |w: &World| -> String {
        w.db()
            .query_row("SELECT since_digest FROM outcome", [], |r| r.get(0))
            .unwrap()
    };
    let first = since(&w);
    w.tag = "later".into();
    w.publish(&[file("a.py", vec![func("a/h().", "h").sig("x").body("x")])]);
    assert_eq!(since(&w), first);
}

#[test]
fn a_rejected_parent_orphans_its_parameters() {
    let mut w = World::new();
    let make = |name: &str| {
        vec![file(
            "a.py",
            vec![
                func(&format!("a/{name}()."), name).body("big"),
                param(&format!("a/{name}()."), &format!("a/{name}().(p)"), "p").within(&[name]),
            ],
        )]
    };
    w.publish(&make("f"));
    let parent = w.row("a/f().").id;
    let old_param = w.param("a/f().", "p").id;
    w.publish(&make("g"));
    assert_eq!(w.state("a/g()."), "moved:T5");
    assert_eq!(state_of(&w.param("a/g().", "p")), "matched");
    // Reject the move. The parameter has no state of its own, so the store still shows it matched.
    w.store().reject(&parent).unwrap();
    // The same input again: the rebinding is skipped, and the parameter is still orphaned.
    let again = w.publish(&make("g"));
    assert!(again.skipped);
    let after = |w: &World| w.rows().into_iter().find(|r| r.id == old_param).unwrap();
    assert_eq!(state_of(&after(&w)), "orphaned");
    // A later input: the declaration g mints a new entity, with a new parameter.
    w.tag = "later".into();
    w.publish(&make("g"));
    assert_eq!(state_of(&after(&w)), "orphaned");
    assert_ne!(w.row("a/g().").id, parent);
    assert_ne!(w.param("a/g().", "p").id, old_param);
    assert_eq!(state_of(&w.param("a/g().", "p")), "matched");
}

#[test]
fn a_version_only_bump_rebinds_and_updates_the_symbol_in_place() {
    let mut w = World::new();
    w.ver_in_options = false;
    let files = vec![file("a.py", vec![func("a/f().", "f"), variable("a/x.", "x")]).source("same")];
    w.publish(&files);
    let ids = ids_of(&w);
    w.ver = "2.0".into();
    let report = w.publish(&files);
    // The facts input digest did not change, and the rebinding still ran.
    assert!(!report.skipped);
    assert_eq!(report.bound.get("T1"), Some(&2));
    assert_eq!(ids_of(&w), ids);
    assert_eq!(w.count("locator"), 2);
    assert_eq!(w.count("outcome"), 0);
    let symbols: Vec<String> = {
        let db = w.db();
        let mut st = db.prepare("SELECT symbol FROM locator").unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(symbols.iter().all(|s| s.contains(" 2.0 ")), "{symbols:?}");
    // The same input again is skipped.
    assert!(w.publish(&files).skipped);
}

#[test]
fn the_minimum_body_size_reads_the_environment() {
    // The variable is read when a run starts. Only a whole number counts.
    assert_eq!(min_body_tokens(), MIN_BODY_TOKENS);
}
