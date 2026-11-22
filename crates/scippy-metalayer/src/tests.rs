use std::path::Path;

use rusqlite::Connection;
use scippy_core::{
    AnalysisDiagnostic, AnalysisDocument, AnalysisEdge, AnalysisFile, AnalysisOccurrence,
    AnalysisReference, AnalysisSnapshot, AnalysisSymbol, ReferenceOutcome, ScippyError,
};

use crate::publish::sha256_hex;
use crate::schema::{APPLICATION_ID, SCHEMA_VERSION};
use crate::*;

fn symbol(name: &str) -> AnalysisSymbol {
    AnalysisSymbol {
        symbol: name.into(),
        kind: "Function".into(),
        display_name: name.into(),
        implements: Vec::new(),
        references: Vec::new(),
        decl: None,
    }
}

fn occurrence(start: u32, end: u32, symbol: &str, role: &str) -> AnalysisOccurrence {
    AnalysisOccurrence {
        start,
        end,
        symbol: symbol.into(),
        role: role.into(),
    }
}

fn document(path: &str, source: &str) -> AnalysisDocument {
    AnalysisDocument {
        path: path.into(),
        language: "Python".into(),
        pack: Some("python".into()),
        identity: scippy_core::IdentityFamily::Code,
        real_path: None,
        sha256: Some(sha256_hex(source.as_bytes())),
        source: source.into(),
        stale: false,
        occurrences: Vec::new(),
        symbols: Vec::new(),
    }
}

fn lost(path: &str, reason: &str) -> AnalysisFile {
    AnalysisFile {
        path: path.into(),
        pack: Some("python".into()),
        real_path: None,
        status: scippy_core::file_status(reason).into(),
        reason: reason.into(),
        sha256: None,
    }
}

fn reference(path: &str, start: u32, end: u32, outcome: ReferenceOutcome) -> AnalysisReference {
    AnalysisReference {
        path: path.into(),
        start,
        end,
        outcome,
        limited: None,
        kind: None,
    }
}

fn resolved(target: &str) -> ReferenceOutcome {
    ReferenceOutcome::Resolved(target.into())
}

fn empty() -> AnalysisSnapshot {
    AnalysisSnapshot {
        documents: Vec::new(),
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
        options: "options".into(),
        support_inputs: Vec::new(),
    }
}

/// One document and one reference to `target`.
fn snapshot(target: &str) -> AnalysisSnapshot {
    let mut s = empty();
    s.documents = vec![document("a.py", "x = 1\n")];
    s.references = vec![reference("a.py", 0, 1, resolved(target))];
    s
}

pub(crate) fn layer(dir: &Path) -> Metalayer {
    Metalayer::open(dir.join("layer.sqlite")).unwrap()
}

pub(crate) fn publish(layer: &mut Metalayer, s: &AnalysisSnapshot) -> i64 {
    layer
        .publish(s, &PublishOptions::default())
        .unwrap()
        .revision
}

fn count(layer: &Metalayer, table: &str) -> i64 {
    layer
        .db
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// A fixture with a local symbol, a definition site, an import, and a limited reference.
pub(crate) fn rich() -> AnalysisSnapshot {
    let mut s = empty();
    let mut a = document("a.py", "def f(): pass\nf()\nlocal\nimp\n");
    a.symbols = vec![symbol("g f()."), symbol("local 0")];
    a.occurrences = vec![
        occurrence(4, 5, "g f().", "definition"),
        occurrence(14, 15, "g f().", "reference"),
        occurrence(16, 21, "local 0", "reference"),
        occurrence(22, 25, "g ext.", "import"),
    ];
    let mut b = document("b.py", "x = 1\n");
    b.symbols = vec![symbol("local 0")];
    b.occurrences = vec![occurrence(0, 1, "local 0", "reference")];
    s.documents = vec![a, b];
    s.external_symbols = vec![symbol("g ext.")];
    let mut limited = reference("a.py", 30, 31, resolved("g f()."));
    limited.limited = Some(Bound::Work);
    s.references = vec![
        // A write at the definition site resolves to its own declaration.
        reference("a.py", 4, 5, resolved("g f().")),
        reference("a.py", 14, 15, resolved("g f().")),
        reference("a.py", 16, 21, resolved("local 0")),
        reference("a.py", 22, 25, resolved("g ext.")),
        limited,
        reference(
            "a.py",
            40,
            41,
            ReferenceOutcome::Unresolved("no-path".into()),
        ),
        reference("a.py", 42, 43, ReferenceOutcome::Bounded("work".into())),
        reference(
            "a.py",
            44,
            45,
            ReferenceOutcome::Ambiguous(vec!["g z.".into(), "g ext.".into(), "g z.".into()]),
        ),
        reference("b.py", 0, 1, resolved("local 0")),
    ];
    s.analysis_exact = false;
    s.bounded_references = 2;
    s
}

#[test]
fn the_relation_drops_local_targets_and_definition_sites() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &rich());
    let view = layer.view().unwrap().unwrap();
    let rows = view.relations(RelationQuery::default()).unwrap();
    let flat: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.path.as_str(),
                r.target.symbol.symbol.as_str(),
                r.kind,
                r.supports,
                r.limited,
            )
        })
        .collect();
    // The same-file global target stays. The use at 14 and the limited use at 30 count.
    // The definition at 4 and the local targets do not.
    assert_eq!(
        flat,
        vec![
            ("a.py", "g ext.", RelationKind::Import, 1, 0),
            ("a.py", "g f().", RelationKind::Use, 2, 1),
        ]
    );
}

/// A union: the index has one occurrence for each candidate at one range, and the reference
/// stays ambiguous with both candidates. SCIP allows several occurrences at one range.
fn union() -> AnalysisSnapshot {
    let mut s = empty();
    let mut a = document("a.py", "e.items\n");
    a.symbols = vec![symbol("g A#items."), symbol("g B#items.")];
    a.occurrences = vec![
        // The order of the occurrences does not matter. The store sorts them by symbol.
        occurrence(2, 7, "g B#items.", "reference"),
        occurrence(2, 7, "g A#items.", "reference"),
    ];
    s.documents = vec![a];
    s.references = vec![reference(
        "a.py",
        2,
        7,
        ReferenceOutcome::Ambiguous(vec!["g A#items.".into(), "g B#items.".into()]),
    )];
    s
}

#[test]
fn a_union_stores_one_occurrence_for_each_candidate_and_one_ambiguous_reference() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &union());
    let view = layer.view().unwrap().unwrap();
    // Go to definition: every occurrence at the range names one candidate.
    let at_range: Vec<_> = view
        .occurrences("a.py")
        .unwrap()
        .into_iter()
        .filter(|o| o.span == (2, 7))
        .map(|o| (o.symbol.symbol, o.role))
        .collect();
    assert_eq!(
        at_range,
        vec![
            ("g A#items.".to_owned(), OccurrenceRole::Reference),
            ("g B#items.".to_owned(), OccurrenceRole::Reference)
        ]
    );
    // The reference at the range is one row with both candidates.
    let reference = view.references_at("a.py", (2, 7)).unwrap().unwrap();
    assert_eq!(
        reference.outcome,
        StoredOutcome::Ambiguous(vec![
            SymbolRef::global("g A#items."),
            SymbolRef::global("g B#items.")
        ])
    );
    // Find references: the site is listed for each candidate, and for no other symbol.
    for name in ["g A#items.", "g B#items."] {
        let sites = view
            .ambiguous_with("a.py", &SymbolRef::global(name))
            .unwrap();
        assert_eq!(sites.iter().map(|r| r.span).collect::<Vec<_>>(), [(2, 7)]);
    }
    assert!(
        view.ambiguous_with("a.py", &SymbolRef::global("g C#items."))
            .unwrap()
            .is_empty()
    );
    // No file depends on a candidate yet: the site is only a maybe.
    assert!(view.relations(RelationQuery::default()).unwrap().is_empty());
}

#[test]
fn a_limited_reference_counts_and_shows_in_the_relation_and_its_supports() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &rich());
    let view = layer.view().unwrap().unwrap();
    let supports = view.supports("a.py", &SymbolRef::global("g f().")).unwrap();
    assert_eq!(
        supports
            .iter()
            .map(|s| (s.span, s.limited))
            .collect::<Vec<_>>(),
        vec![((14, 15), None), ((30, 31), Some(Bound::Work))]
    );
    assert!(!view.revision().analysis_exact);
    assert!(view.revision().input_complete);
}

#[test]
fn a_local_symbol_in_two_files_stays_two_symbols() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &rich());
    assert_eq!(
        layer
            .db
            .query_row(
                "SELECT count(*) FROM symbol WHERE symbol = 'local 0'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    let view = layer.view().unwrap().unwrap();
    let a = view.occurrences("a.py").unwrap();
    let b = view.occurrences("b.py").unwrap();
    assert!(
        a.iter()
            .any(|o| o.symbol == SymbolRef::local("local 0", "a.py"))
    );
    assert_eq!(b[0].symbol, SymbolRef::local("local 0", "b.py"));
    let references = view
        .references(ReferenceQuery {
            path: Some("b.py".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        references[0].outcome,
        StoredOutcome::Resolved(SymbolRef::local("local 0", "b.py"))
    );
}

#[test]
fn the_rows_equal_the_snapshot_and_hold_no_dangling_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let s = rich();
    publish(&mut layer, &s);
    assert_eq!(count(&layer, "file"), s.documents.len() as i64);
    assert_eq!(count(&layer, "reference"), s.references.len() as i64);
    assert_eq!(
        count(&layer, "occurrence"),
        s.documents
            .iter()
            .map(|d| d.occurrences.len())
            .sum::<usize>() as i64
    );
    // Two candidates: the duplicate candidate collapses.
    assert_eq!(count(&layer, "reference_candidate"), 2);
    assert_eq!(count(&layer, "diagnostic"), 0);
    let violations: i64 = layer
        .db
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
    let view = layer.view().unwrap().unwrap();
    let candidates: Vec<_> = view
        .references(ReferenceQuery {
            outcome: Some(OutcomeKind::Ambiguous),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        candidates[0].outcome,
        StoredOutcome::Ambiguous(vec![SymbolRef::global("g ext."), SymbolRef::global("g z.")])
    );
}

#[test]
fn a_declaration_edge_keeps_its_source_and_target() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = empty();
    let mut a = document("a.py", "class B: pass\nclass C(B): pass\n");
    let mut child = symbol("g C#m().");
    child.implements = vec!["g B#m().".into()];
    child.references = vec!["g alias.".into()];
    a.symbols = vec![symbol("g B#m()."), child, symbol("g alias.")];
    s.documents = vec![a];
    publish(&mut layer, &s);
    let view = layer.view().unwrap().unwrap();
    let edges = view.edges(Some(EdgeKind::Override)).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].source, Some(SymbolRef::global("g C#m().")));
    assert_eq!(edges[0].target, Some(SymbolRef::global("g B#m().")));
    assert_eq!(view.edges(Some(EdgeKind::Alias)).unwrap().len(), 1);
    assert_eq!(view.edges(None).unwrap().len(), 2);
}

#[test]
fn diagnostics_keep_their_codes_and_spans() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("t");
    s.diagnostics = vec![
        AnalysisDiagnostic {
            severity: "Warning".into(),
            stage: "Parse".into(),
            file: Some("a.py".into()),
            range: Some((1, 2)),
            code: "repaired-syntax".into(),
            message: "m".into(),
        },
        AnalysisDiagnostic {
            severity: "Info".into(),
            stage: "Discover".into(),
            file: None,
            range: None,
            code: "other".into(),
            message: "n".into(),
        },
    ];
    publish(&mut layer, &s);
    let diagnostics = layer.view().unwrap().unwrap().diagnostics().unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].severity, "warning");
    assert_eq!(diagnostics[0].span, Some((1, 2)));
    assert_eq!(diagnostics[1].path, None);
}

#[test]
fn a_bounded_search_publishes_and_keeps_its_marks() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("s");
    let mut kept = reference("a.py", 2, 3, resolved("s"));
    kept.limited = Some(Bound::Work);
    s.references.push(kept);
    s.references.push(reference(
        "a.py",
        4,
        5,
        ReferenceOutcome::Bounded("queue-length".into()),
    ));
    s.analysis_exact = false;
    s.bounded_references = 2;
    publish(&mut layer, &s);
    let view = layer.view().unwrap().unwrap();
    let all = view.references(ReferenceQuery::default()).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[1].limited, Some(Bound::Work));
    assert_eq!(
        all[2].outcome,
        StoredOutcome::Bounded("queue-length".into())
    );
    let rows = view.relations(RelationQuery::default()).unwrap();
    assert_eq!((rows[0].supports, rows[0].limited), (2, 1));
}

#[test]
fn a_failed_file_publishes_and_the_input_is_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("s");
    s.lost_files = vec![lost("bad.py", "syntax-error")];
    s.input_complete = false;
    publish(&mut layer, &s);
    let view = layer.view().unwrap().unwrap();
    assert!(!view.revision().input_complete);
    assert!(!view.revision().strict);
    let failed = view.files(Some(FileStatus::Failed)).unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].path, "bad.py");
    assert_eq!(failed[0].reason.as_deref(), Some("syntax-error"));
    assert_eq!(failed[0].sha256, None);
    let all: Vec<_> = view
        .files(None)
        .unwrap()
        .into_iter()
        .map(|f| (f.path, f.status))
        .collect();
    assert_eq!(
        all,
        vec![
            ("a.py".to_string(), FileStatus::Indexed),
            ("bad.py".to_string(), FileStatus::Failed)
        ]
    );
}

#[test]
fn a_skipped_file_makes_the_input_incomplete_and_a_shadowed_file_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut shadowed = snapshot("s");
    shadowed.lost_files = vec![lost("a/__init__.py", "shadowed-file")];
    publish(&mut layer, &shadowed);
    let view = layer.view().unwrap().unwrap();
    assert!(view.revision().input_complete);
    let rows = view.files(Some(FileStatus::Shadowed)).unwrap();
    assert_eq!(rows[0].reason.as_deref(), Some("shadowed-file"));
    drop(view);
    let mut skipped = snapshot("s");
    skipped.lost_files = vec![lost("link.py", "symlink-skipped")];
    skipped.input_complete = false;
    publish(&mut layer, &skipped);
    let view = layer.view().unwrap().unwrap();
    assert!(!view.revision().input_complete);
    assert_eq!(view.files(Some(FileStatus::Skipped)).unwrap().len(), 1);
}

#[test]
fn a_strict_publish_rejects_incomplete_input_and_keeps_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &snapshot("first"));
    let mut bad = snapshot("second");
    bad.lost_files = vec![lost("bad.py", "syntax-error")];
    bad.input_complete = false;
    let strict = PublishOptions {
        strict: true,
        replace_root_set: false,
    };
    assert!(matches!(
        layer.publish(&bad, &strict),
        Err(ScippyError::Incomplete)
    ));
    let view = layer.view().unwrap().unwrap();
    assert_eq!(view.revision().id, 1);
    assert_eq!(view.files(None).unwrap().len(), 1);
    drop(view);
    // Without strict, the same snapshot publishes.
    assert_eq!(publish(&mut layer, &bad), 2);
}

#[test]
fn the_flags_must_agree_with_the_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    // The snapshot claims complete input, but a file failed.
    let mut liar = snapshot("s");
    liar.lost_files = vec![lost("bad.py", "bind-error")];
    assert!(layer.publish(&liar, &PublishOptions::default()).is_err());
    // The snapshot claims exactness over a bounded reference.
    let mut liar = snapshot("s");
    liar.references = vec![reference(
        "a.py",
        0,
        1,
        ReferenceOutcome::Bounded("work".into()),
    )];
    assert!(layer.publish(&liar, &PublishOptions::default()).is_err());
    // A diagnostic with a code of lost input makes the input incomplete.
    let mut liar = snapshot("s");
    liar.diagnostics = vec![AnalysisDiagnostic {
        severity: "Warning".into(),
        stage: "Discover".into(),
        file: None,
        range: None,
        code: "manifest-error".into(),
        message: "m".into(),
    }];
    assert!(layer.publish(&liar, &PublishOptions::default()).is_err());
    liar.input_complete = false;
    publish(&mut layer, &liar);
}

#[test]
fn a_lost_file_keeps_the_facts_of_its_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = empty();
    let mut note = document("note.md", "");
    note.language = "Markdown".into();
    note.pack = Some("loci".into());
    note.symbols = vec![symbol("g note.")];
    note.occurrences = vec![occurrence(0, 0, "g note.", "definition")];
    s.documents = vec![note];
    let mut row = lost("note.md", "invalid-utf8");
    row.pack = Some("loci".into());
    s.lost_files = vec![row];
    s.input_complete = false;
    publish(&mut layer, &s);
    let view = layer.view().unwrap().unwrap();
    let files = view.files(None).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, FileStatus::Skipped);
    assert_eq!(files[0].sha256, None);
    assert_eq!(view.occurrences("note.md").unwrap().len(), 1);
}

#[test]
fn the_digest_covers_the_status_and_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let digest = |layer: &mut Metalayer, reason: &str| {
        let mut s = snapshot("a");
        s.lost_files = vec![lost("x.py", reason)];
        s.input_complete = reason == "shadowed-file";
        publish(layer, &s);
        layer
            .view()
            .unwrap()
            .unwrap()
            .revision()
            .input_digest
            .clone()
    };
    let shadowed = digest(&mut layer, "shadowed-file");
    let syntax = digest(&mut layer, "syntax-error");
    assert_ne!(shadowed, syntax);
    assert_ne!(syntax, digest(&mut layer, "bind-error"));
    assert_eq!(syntax, digest(&mut layer, "syntax-error"));
}

#[test]
fn a_failed_publication_keeps_the_previous_revision_and_relations() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    assert_eq!(publish(&mut layer, &snapshot("first")), 1);
    layer
        .db
        .execute_batch("CREATE TRIGGER reject_publish BEFORE INSERT ON file_reference BEGIN SELECT RAISE(ABORT, 'injected failure'); END;")
        .unwrap();
    assert!(
        layer
            .publish(&snapshot("second"), &PublishOptions::default())
            .is_err()
    );
    let view = layer.view().unwrap().unwrap();
    assert_eq!(view.revision().id, 1);
    let rows = view.relations(RelationQuery::default()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].target.symbol, SymbolRef::global("first"));
    let references = view.references(ReferenceQuery::default()).unwrap();
    assert_eq!(
        references[0].outcome,
        StoredOutcome::Resolved(SymbolRef::global("first"))
    );
}

#[test]
fn a_revision_follows_the_highest_id_and_older_revisions_go() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &snapshot("a"));
    let published = layer
        .publish(
            &snapshot("b"),
            &PublishOptions {
                strict: true,
                replace_root_set: false,
            },
        )
        .unwrap();
    assert_eq!(published.revision, 2);
    assert_eq!(count(&layer, "revision"), 1);
    let view = layer.view().unwrap().unwrap();
    let info = view.revision();
    assert_eq!(info.id, 2);
    assert!(info.strict && info.input_complete && info.analysis_exact);
    assert_eq!(info.scip_sha256, sha256_hex(b"scip"));
    assert_eq!(info.scip_sha256, published.scip_sha256);
    assert_eq!(info.tool_version, "test");
    assert_eq!(info.input_digest.len(), 64);
}

#[test]
fn the_input_digest_follows_the_input_and_ignores_the_facts() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let digest = |layer: &mut Metalayer, s: &AnalysisSnapshot| {
        publish(layer, s);
        layer
            .view()
            .unwrap()
            .unwrap()
            .revision()
            .input_digest
            .clone()
    };
    let first = digest(&mut layer, &snapshot("a"));
    assert_eq!(digest(&mut layer, &snapshot("b")), first);
    let mut other_options = snapshot("a");
    other_options.options = "other".into();
    assert_ne!(digest(&mut layer, &other_options), first);
    let mut other_file = snapshot("a");
    other_file.documents[0] = document("a.py", "x = 2\n");
    assert_ne!(digest(&mut layer, &other_file), first);
}

#[test]
fn a_second_root_set_is_refused_and_the_flag_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &snapshot("a"));
    let mut other = snapshot("b");
    other.project_root = "/other".into();
    match layer.publish(&other, &PublishOptions::default()) {
        Err(ScippyError::RootSetMismatch { stored, requested }) => {
            assert_eq!(
                (stored.as_str(), requested.as_str()),
                ("/project", "/other")
            );
        }
        other => panic!("expected a mismatch, got {other:?}"),
    }
    assert_eq!(layer.view().unwrap().unwrap().revision().id, 1);
    let replaced = layer
        .publish(
            &other,
            &PublishOptions {
                strict: false,
                replace_root_set: true,
            },
        )
        .unwrap();
    assert_eq!(replaced.revision, 2);
    let root = layer.view().unwrap().unwrap().root_set().unwrap();
    assert_eq!(root.project_root, "/other");
    assert_eq!(root.key, sha256_hex(b"/other"));
}

#[test]
fn a_shard_change_publishes_without_a_flag_and_rewrites_the_members() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("a");
    s.shards = vec!["app".into(), "lib".into()];
    publish(&mut layer, &s);
    assert_eq!(
        layer.view().unwrap().unwrap().root_set().unwrap().shards,
        vec!["app", "lib"]
    );
    s.shards = vec!["lib".into()];
    publish(&mut layer, &s);
    let root = layer.view().unwrap().unwrap().root_set().unwrap();
    assert_eq!(root.shards, vec!["lib"]);
    assert_eq!(root.key, sha256_hex(b"/project"));
    assert_eq!(count(&layer, "root_member"), 2);
}

#[test]
fn eight_concurrent_publishers_all_succeed_with_distinct_revisions() {
    use std::sync::{Arc, Barrier};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layer.sqlite");
    let mut layers: Vec<_> = (0..8).map(|_| Metalayer::open(&path).unwrap()).collect();
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = layers
        .drain(..)
        .enumerate()
        .map(|(i, mut layer)| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let s = snapshot(&format!("t{i}"));
                barrier.wait();
                layer
                    .publish(&s, &PublishOptions::default())
                    .map(|p| p.revision)
            })
        })
        .collect();
    let mut revisions: Vec<i64> = handles
        .into_iter()
        .map(|h| h.join().unwrap().unwrap())
        .collect();
    revisions.sort_unstable();
    assert_eq!(revisions, (1..=8).collect::<Vec<_>>());
}

#[test]
fn a_reader_sees_one_revision_while_a_writer_publishes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layer.sqlite");
    let mut reader = Metalayer::open(&path).unwrap();
    let mut writer = Metalayer::open(&path).unwrap();
    let handle = std::thread::spawn(move || {
        for i in 0..100 {
            writer
                .publish(&snapshot(&format!("t{i}")), &PublishOptions::default())
                .unwrap();
        }
    });
    let mut last = 0;
    while last < 100 {
        let Some(view) = reader.view().unwrap() else {
            continue;
        };
        let expected = SymbolRef::global(format!("t{}", view.revision().id - 1));
        let references = view.references(ReferenceQuery::default()).unwrap();
        assert_eq!(
            references[0].outcome,
            StoredOutcome::Resolved(expected.clone())
        );
        let rows = view.relations(RelationQuery::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target.symbol, expected);
        last = view.revision().id;
    }
    handle.join().unwrap();
}

#[test]
fn a_new_database_gets_the_application_id_and_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layer.sqlite");
    drop(Metalayer::open(&path).unwrap());
    let db = Connection::open(&path).unwrap();
    let id: i32 = db
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap();
    let version: i32 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!((id, version), (APPLICATION_ID, SCHEMA_VERSION));
}

#[test]
fn reopening_a_current_database_keeps_its_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layer.sqlite");
    publish(&mut Metalayer::open(&path).unwrap(), &snapshot("a"));
    let mut layer = Metalayer::open(&path).unwrap();
    assert_eq!(layer.view().unwrap().unwrap().revision().id, 1);
}

fn open_error(path: &Path) -> String {
    match Metalayer::open(path) {
        Err(ScippyError::Storage(message)) => message,
        Err(other) => panic!("expected a storage error, got {other}"),
        Ok(_) => panic!("the open must fail"),
    }
}

#[test]
fn an_older_database_is_dropped_and_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layer.sqlite");
    publish(&mut Metalayer::open(&path).unwrap(), &snapshot("a"));
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP TABLE file_reference; CREATE TABLE file_reference (path TEXT, target TEXT);
             CREATE TABLE snapshot (id INTEGER PRIMARY KEY, revision INTEGER, value BLOB);
             PRAGMA user_version=2;",
        )
        .unwrap();
    let mut layer = Metalayer::open(&path).unwrap();
    assert!(layer.view().unwrap().is_none());
    let old: i64 = layer
        .db
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'snapshot'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(old, 0);
    let version: i32 = layer
        .db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    // The rebuilt database works.
    assert_eq!(publish(&mut layer, &snapshot("b")), 1);
}

#[test]
fn a_newer_database_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layer.sqlite");
    publish(&mut Metalayer::open(&path).unwrap(), &snapshot("a"));
    Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA user_version=99;")
        .unwrap();
    assert!(open_error(&path).contains("version 99"));
    let db = Connection::open(&path).unwrap();
    let version: i32 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    let revisions: i64 = db
        .query_row("SELECT count(*) FROM revision", [], |r| r.get(0))
        .unwrap();
    assert_eq!((version, revisions), (99, 1));
}

#[test]
fn a_database_with_another_application_id_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("other.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA application_id=1234; PRAGMA user_version=1;")
        .unwrap();
    assert!(open_error(&path).contains("not a Scippy"));
}

#[test]
fn an_unversioned_database_with_tables_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE snapshot (id INTEGER PRIMARY KEY);")
        .unwrap();
    assert!(open_error(&path).contains("not a Scippy"));
}

#[test]
fn a_read_only_open_never_writes_or_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layer.sqlite");
    assert!(Metalayer::open_read_only(&path).is_err());
    assert!(!path.exists());
    publish(&mut Metalayer::open(&path).unwrap(), &snapshot("a"));
    let mut reader = Metalayer::open_read_only(&path).unwrap();
    assert_eq!(reader.view().unwrap().unwrap().revision().id, 1);
    assert!(
        reader
            .publish(&snapshot("b"), &PublishOptions::default())
            .is_err()
    );
    drop(reader);
    Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA user_version=2;")
        .unwrap();
    assert!(Metalayer::open_read_only(&path).is_err());
    let db = Connection::open(&path).unwrap();
    let version: i32 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
}

#[test]
fn the_source_is_verified_by_its_hash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.py"), "x = 1\n").unwrap();
    let outside = dir.path().join("outside.py");
    std::fs::write(&outside, "y = 2\n").unwrap();
    let mut s = snapshot("t");
    let mut far = document("virtual/b.py", "y = 2\n");
    far.real_path = Some(outside.to_string_lossy().into_owned());
    s.documents.push(far);
    let mut layer = layer(dir.path());
    publish(&mut layer, &s);
    let view = layer.view().unwrap().unwrap();
    assert_eq!(
        view.source(&root, "a.py").unwrap(),
        Source::Verified("x = 1\n".into())
    );
    assert_eq!(
        view.source(&root, "virtual/b.py").unwrap(),
        Source::Verified("y = 2\n".into())
    );
    std::fs::write(root.join("a.py"), "x = 3\n").unwrap();
    assert_eq!(view.source(&root, "a.py").unwrap(), Source::Changed);
    std::fs::remove_file(root.join("a.py")).unwrap();
    assert_eq!(view.source(&root, "a.py").unwrap(), Source::Missing);
    assert!(view.source(&root, "unknown.py").is_err());
}

#[test]
fn a_reference_in_an_unknown_file_fails_the_publish() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("t");
    s.references
        .push(reference("missing.py", 0, 1, resolved("t")));
    assert!(layer.publish(&s, &PublishOptions::default()).is_err());
    assert!(layer.view().unwrap().is_none());
}

fn edge(kind: &str, path: &str, span: (u32, u32)) -> AnalysisEdge {
    AnalysisEdge {
        kind: kind.into(),
        path: path.into(),
        source: None,
        span,
        reference_span: Some(span),
        anchor_span: None,
        link_kind: None,
        label: None,
        state: None,
        anchor_state: None,
    }
}

/// A document of a native pack, with the module symbol and one heading.
fn note(path: &str) -> AnalysisDocument {
    let mut d = document(path, "# Heading\n");
    d.language = "Markdown".into();
    d.pack = Some("loci".into());
    d.symbols = vec![
        symbol(&format!("g {path}/")),
        symbol(&format!("g {path}/Heading.")),
    ];
    d.occurrences = vec![
        occurrence(0, 0, &format!("g {path}/"), "definition"),
        occurrence(0, 9, &format!("g {path}/Heading."), "definition"),
    ];
    d
}

fn link(span: (u32, u32), state: &str, anchor_state: &str) -> AnalysisEdge {
    AnalysisEdge {
        link_kind: Some("wikilink".into()),
        label: Some("label".into()),
        state: Some(state.into()),
        anchor_state: Some(anchor_state.into()),
        ..edge("link", "src.md", span)
    }
}

/// A source note with links of every state, and the target notes.
pub(crate) fn vault() -> AnalysisSnapshot {
    let mut s = empty();
    let mut src = note("src.md");
    src.occurrences = vec![occurrence(0, 0, "g src.md/", "definition")];
    src.symbols = vec![symbol("g src.md/")];
    s.documents = vec![src, note("t.md"), note("u.md")];
    let reference = |start, end, outcome| reference("src.md", start, end, outcome);
    s.references = vec![
        // [[t#Heading]] at 10..22: the target part and the anchor part have a reference each.
        reference(12, 13, resolved("g t.md/")),
        reference(14, 21, resolved("g t.md/Heading.")),
        // [[t#Nope]] at 30..40: the anchor is missing. One reference over the whole link.
        reference(30, 40, resolved("g t.md/")),
        // ![[u]] at 50..56: an embed with the read role.
        reference(50, 56, resolved("g u.md/")),
        // [[both]] at 60..68: two candidates.
        reference(
            60,
            68,
            ReferenceOutcome::Ambiguous(vec!["g u.md/".into(), "g t.md/".into()]),
        ),
        // [[nothing]] at 70..80: no target.
        reference(
            70,
            80,
            ReferenceOutcome::Unresolved("no-declaration".into()),
        ),
        // [[#Own]] at 90..96: the anchor reference only.
        reference(91, 95, resolved("g src.md/Own.")),
    ];
    let mut anchored = link((10, 22), "resolved", "resolved");
    anchored.reference_span = Some((12, 13));
    anchored.anchor_span = Some((14, 21));
    let mut own = link((90, 96), "resolved", "resolved");
    own.reference_span = None;
    own.anchor_span = Some((91, 95));
    let mut embed = edge("embed", "src.md", (50, 56));
    embed.link_kind = Some("embed".into());
    embed.state = Some("resolved".into());
    embed.anchor_state = Some("none".into());
    s.edges = vec![
        anchored,
        link((30, 40), "resolved", "missing"),
        embed,
        link((60, 68), "ambiguous", "none"),
        link((70, 80), "missing", "none"),
        own,
    ];
    s
}

#[test]
fn a_link_edge_keeps_its_state_and_anchor_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &vault());
    let view = layer.view().unwrap().unwrap();
    let edges = view.edges(Some(EdgeKind::Link)).unwrap();
    let flat: Vec<_> = edges
        .iter()
        .map(|e| {
            (
                e.span,
                e.state.as_deref(),
                e.anchor_state.as_deref(),
                e.target.is_some(),
            )
        })
        .collect();
    assert_eq!(
        flat,
        vec![
            (Some((10, 22)), Some("resolved"), Some("resolved"), true),
            (Some((30, 40)), Some("resolved"), Some("missing"), true),
            (Some((60, 68)), Some("ambiguous"), Some("none"), false),
            (Some((70, 80)), Some("missing"), Some("none"), false),
            (Some((90, 96)), Some("resolved"), Some("resolved"), false),
        ]
    );
    assert_eq!(edges[0].link_kind.as_deref(), Some("wikilink"));
    assert_eq!(edges[0].label.as_deref(), Some("label"));
    assert_eq!(edges[0].target, Some(SymbolRef::global("g t.md/")));
    let embeds = view.edges(Some(EdgeKind::Embed)).unwrap();
    assert_eq!(embeds.len(), 1);
    assert_eq!(embeds[0].target, Some(SymbolRef::global("g u.md/")));
}

#[test]
fn an_ambiguous_link_has_its_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &vault());
    let view = layer.view().unwrap().unwrap();
    let edges = view.edges(Some(EdgeKind::Link)).unwrap();
    let ambiguous = edges.iter().find(|e| e.span == Some((60, 68))).unwrap();
    assert_eq!(
        ambiguous.candidates,
        vec![SymbolRef::global("g t.md/"), SymbolRef::global("g u.md/")]
    );
    assert!(edges[0].candidates.is_empty());
}

#[test]
fn a_link_with_an_anchor_gives_one_edge_and_two_supports() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &vault());
    let view = layer.view().unwrap().unwrap();
    let anchored: Vec<_> = view
        .edges(Some(EdgeKind::Link))
        .unwrap()
        .into_iter()
        .filter(|e| e.span == Some((10, 22)))
        .collect();
    assert_eq!(anchored.len(), 1);
    let note = view
        .supports("src.md", &SymbolRef::global("g t.md/"))
        .unwrap();
    let heading = view
        .supports("src.md", &SymbolRef::global("g t.md/Heading."))
        .unwrap();
    assert_eq!(note.len(), 2);
    assert_eq!(note[0].span, (12, 13));
    assert_eq!(heading.len(), 1);
    assert_eq!(heading[0].span, (14, 21));
    assert!(
        note.iter()
            .chain(&heading)
            .all(|s| s.kind == RelationKind::Link)
    );
    // The embed is an embed support.
    let embed = view
        .supports("src.md", &SymbolRef::global("g u.md/"))
        .unwrap();
    assert_eq!(embed[0].kind, RelationKind::Embed);
    // The relation counts by kind.
    let rows = view
        .relations(RelationQuery {
            path: Some("src.md".into()),
            kinds: vec![RelationKind::Link],
            ..Default::default()
        })
        .unwrap();
    let t = rows
        .iter()
        .find(|r| r.target.symbol.symbol == "g t.md/")
        .unwrap();
    assert_eq!(t.supports, 2);
}

#[test]
fn the_import_edge_follows_the_import_reference() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    publish(&mut layer, &rich());
    let view = layer.view().unwrap().unwrap();
    let imports = view.edges(Some(EdgeKind::Import)).unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].path, "a.py");
    assert_eq!(imports[0].span, Some((22, 25)));
    assert_eq!(imports[0].target, Some(SymbolRef::global("g ext.")));
    assert_eq!(imports[0].source, None);
}

#[test]
fn an_inheritance_edge_keeps_the_class_and_the_supertype() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = empty();
    let mut a = document(
        "a.py",
        "class A: pass\nclass B: pass\nclass C(A, B): pass\n",
    );
    a.symbols = vec![symbol("g A#"), symbol("g B#"), symbol("g C#")];
    s.documents = vec![a];
    s.references = vec![
        reference("a.py", 36, 37, resolved("g A#")),
        reference("a.py", 39, 40, resolved("g B#")),
    ];
    for (span, _) in [((36, 37), "A"), ((39, 40), "B")] {
        let mut e = edge("inheritance", "a.py", span);
        e.source = Some("g C#".into());
        s.edges.push(e);
    }
    publish(&mut layer, &s);
    let view = layer.view().unwrap().unwrap();
    let edges = view.edges(Some(EdgeKind::Inheritance)).unwrap();
    let flat: Vec<_> = edges
        .iter()
        .map(|e| {
            (
                e.source.clone().unwrap().symbol,
                e.target.clone().unwrap().symbol,
                e.span,
            )
        })
        .collect();
    assert_eq!(
        flat,
        vec![
            ("g C#".to_string(), "g A#".to_string(), Some((36, 37))),
            ("g C#".to_string(), "g B#".to_string(), Some((39, 40))),
        ]
    );
}

#[test]
fn an_edge_that_names_no_reference_fails_the_publish() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("t");
    s.edges = vec![edge("inheritance", "a.py", (7, 8))];
    assert!(layer.publish(&s, &PublishOptions::default()).is_err());
    assert!(layer.view().unwrap().is_none());
}

#[test]
fn the_reference_kind_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("t");
    let all = [
        ReferenceKind::Plain,
        ReferenceKind::Member,
        ReferenceKind::Keyword,
    ];
    for (n, kind) in all.into_iter().enumerate() {
        let mut r = reference("a.py", 10 + n as u32, 11 + n as u32, resolved("t"));
        r.kind = Some(kind);
        s.references.push(r);
    }
    publish(&mut layer, &s);
    let view = layer.view().unwrap().unwrap();
    let kinds: Vec<_> = view
        .references(ReferenceQuery::default())
        .unwrap()
        .into_iter()
        .map(|r| r.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            None,
            Some(ReferenceKind::Plain),
            Some(ReferenceKind::Member),
            Some(ReferenceKind::Keyword)
        ]
    );
}

#[test]
fn every_bound_round_trips_through_publish_and_view() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("t");
    let all = [
        Bound::Work,
        Bound::QueueLength,
        Bound::ArenaCells,
        Bound::Failed,
    ];
    for (n, bound) in all.into_iter().enumerate() {
        let mut r = reference("a.py", 10 + n as u32, 11 + n as u32, resolved("t"));
        r.limited = Some(bound);
        s.references.push(r);
    }
    s.analysis_exact = false;
    s.bounded_references = 4;
    publish(&mut layer, &s);
    let view = layer.view().unwrap().unwrap();
    let limited: Vec<_> = view
        .references(ReferenceQuery::default())
        .unwrap()
        .into_iter()
        .map(|r| r.limited)
        .collect();
    assert_eq!(
        limited,
        vec![
            None,
            Some(Bound::Work),
            Some(Bound::QueueLength),
            Some(Bound::ArenaCells),
            Some(Bound::Failed)
        ]
    );
    let supports = view.supports("a.py", &SymbolRef::global("t")).unwrap();
    assert_eq!(
        supports
            .iter()
            .filter_map(|s| s.limited)
            .collect::<Vec<_>>(),
        all.to_vec()
    );
}

#[test]
fn an_unknown_stored_bound_or_kind_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("t");
    let mut r = reference("a.py", 10, 11, resolved("t"));
    r.limited = Some(Bound::Work);
    r.kind = Some(ReferenceKind::Plain);
    s.references.push(r);
    s.analysis_exact = false;
    s.bounded_references = 1;
    publish(&mut layer, &s);

    // The CHECK constraints refuse an unknown value.
    for column in ["limited", "kind"] {
        let sql = format!("UPDATE reference SET {column} = 'bogus' WHERE start = 10");
        assert!(layer.db.execute(&sql, []).is_err(), "{column}");
    }

    // A reader still refuses a value that got past the constraints.
    layer
        .db
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;
             UPDATE reference SET limited = 'bogus' WHERE start = 10;
             PRAGMA ignore_check_constraints = OFF;",
        )
        .unwrap();
    let view = layer.view().unwrap().unwrap();
    let error = view.references(ReferenceQuery::default()).unwrap_err();
    assert!(error.to_string().contains("Bound"), "{error}");
    drop(view);
    layer
        .db
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;
             UPDATE reference SET limited = NULL, kind = 'bogus' WHERE start = 10;
             PRAGMA ignore_check_constraints = OFF;",
        )
        .unwrap();
    let view = layer.view().unwrap().unwrap();
    let error = view.references(ReferenceQuery::default()).unwrap_err();
    assert!(error.to_string().contains("ReferenceKind"), "{error}");
}

#[test]
fn the_support_view_uses_the_edge_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let layer = layer(dir.path());
    let plan: Vec<String> = layer
        .db
        .prepare("EXPLAIN QUERY PLAN SELECT kind FROM support")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let plan = plan.join("\n");
    assert!(plan.contains("edge_reference"), "{plan}");
    assert!(plan.contains("edge_anchor"), "{plan}");
}

// ---- Integrity: the DDL hash, the foreign file, the support files ----

fn sha256_of(text: &str) -> String {
    sha256_hex(text.as_bytes())
}

#[test]
fn the_ddl_hash_follows_the_version() {
    use crate::schema::{DDL, DDL_HISTORY};
    let versions: Vec<i32> = DDL_HISTORY.iter().map(|(v, _)| *v).collect();
    assert!(
        versions.windows(2).all(|w| w[0] < w[1]),
        "the versions in DDL_HISTORY must rise: {versions:?}"
    );
    let (last_version, last_hash) = DDL_HISTORY.last().copied().unwrap();
    assert_eq!(
        last_version, SCHEMA_VERSION,
        "the last entry of DDL_HISTORY must be SCHEMA_VERSION"
    );
    assert_eq!(
        sha256_of(DDL),
        last_hash,
        "DDL changed but SCHEMA_VERSION did not: raise SCHEMA_VERSION and add an entry \
         (new version, new hash) to DDL_HISTORY"
    );
}

/// The bytes of a file and the names of the files beside it.
fn snapshot_of_dir(path: &Path) -> (Vec<u8>, Vec<String>) {
    let mut names: Vec<String> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    (std::fs::read(path).unwrap(), names)
}

#[test]
fn a_foreign_file_is_refused_before_the_wal_mode_is_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("foreign.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);")
        .unwrap();
    let before = snapshot_of_dir(&path);
    // Bytes 18 and 19 of the header are 1 in rollback mode and 2 in WAL mode.
    assert_eq!(before.0[18], 1);
    assert!(open_error(&path).contains("not a Scippy"));
    assert_eq!(snapshot_of_dir(&path), before);
    // A file with another application id gets the same care.
    let other = dir.path().join("other.sqlite");
    Connection::open(&other)
        .unwrap()
        .execute_batch("PRAGMA application_id=1234; PRAGMA user_version=1; CREATE TABLE t (x);")
        .unwrap();
    let before = snapshot_of_dir(&other);
    assert!(open_error(&other).contains("not a Scippy"));
    assert_eq!(snapshot_of_dir(&other), before);
}

fn input(path: &str, sha256: Option<&str>) -> scippy_core::AnalysisInput {
    scippy_core::AnalysisInput {
        path: path.into(),
        real_path: None,
        sha256: sha256.map(str::to_owned),
    }
}

fn digest_of(layer: &mut Metalayer) -> String {
    layer
        .view()
        .unwrap()
        .unwrap()
        .revision()
        .input_digest
        .clone()
}

#[test]
fn a_support_file_is_part_of_the_input_digest() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("a");
    publish(&mut layer, &s);
    let plain = digest_of(&mut layer);
    s.support_inputs = vec![input("pyproject.toml", Some("aa"))];
    publish(&mut layer, &s);
    let with = digest_of(&mut layer);
    assert_ne!(plain, with);
    s.support_inputs = vec![input("pyproject.toml", Some("bb"))];
    publish(&mut layer, &s);
    assert_ne!(digest_of(&mut layer), with);
    // The same file that did not exist is another input than a file with a hash.
    s.support_inputs = vec![input("pyproject.toml", None)];
    publish(&mut layer, &s);
    let absent = digest_of(&mut layer);
    assert_ne!(absent, plain);
    // The same input gives the same digest.
    publish(&mut layer, &s);
    assert_eq!(digest_of(&mut layer), absent);
    let view = layer.view().unwrap().unwrap();
    assert_eq!(
        view.support_inputs().unwrap(),
        vec![crate::InputRecord {
            path: "pyproject.toml".into(),
            real_path: None,
            sha256: None,
        }]
    );
}

#[test]
fn a_support_file_is_checked_against_the_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let root = dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let file = root.join("pyproject.toml");
    std::fs::write(&file, "[project]\n").unwrap();
    let mut s = snapshot("a");
    s.support_inputs = vec![
        input("pyproject.toml", Some(&sha256_of("[project]\n"))),
        input("tsconfig.json", None),
    ];
    publish(&mut layer, &s);
    let view = layer.view().unwrap().unwrap();
    let inputs = view.support_inputs().unwrap();
    let state = |name: &str| {
        let record = inputs.iter().find(|i| i.path == name).unwrap();
        view.input_state(&root, record).unwrap()
    };
    assert_eq!(state("pyproject.toml"), FileState::Ok);
    assert_eq!(state("tsconfig.json"), FileState::Ok);
    std::fs::write(&file, "[project]\nname = 'x'\n").unwrap();
    assert_eq!(state("pyproject.toml"), FileState::Changed);
    std::fs::remove_file(&file).unwrap();
    assert_eq!(state("pyproject.toml"), FileState::Missing);
    // A file that appears after the run changes the input.
    std::fs::write(root.join("tsconfig.json"), "{}").unwrap();
    assert_eq!(state("tsconfig.json"), FileState::Changed);
}
