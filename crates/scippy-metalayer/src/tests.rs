use std::path::Path;

use rusqlite::Connection;
use scippy_core::{
    AnalysisDiagnostic, AnalysisDocument, AnalysisOccurrence, AnalysisReference, AnalysisSnapshot,
    AnalysisSymbol, ReferenceOutcome, ScippyError,
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
        real_path: None,
        sha256: Some(sha256_hex(source.as_bytes())),
        source: source.into(),
        occurrences: Vec::new(),
        symbols: Vec::new(),
    }
}

fn reference(path: &str, start: u32, end: u32, outcome: ReferenceOutcome) -> AnalysisReference {
    AnalysisReference {
        path: path.into(),
        start,
        end,
        outcome,
        limited: None,
    }
}

fn resolved(target: &str) -> ReferenceOutcome {
    ReferenceOutcome::Resolved(target.into())
}

fn empty() -> AnalysisSnapshot {
    AnalysisSnapshot {
        documents: Vec::new(),
        references: Vec::new(),
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
    }
}

/// One document and one reference to `target`.
fn snapshot(target: &str) -> AnalysisSnapshot {
    let mut s = empty();
    s.documents = vec![document("a.py", "x = 1\n")];
    s.references = vec![reference("a.py", 0, 1, resolved(target))];
    s
}

fn layer(dir: &Path) -> Metalayer {
    Metalayer::open(dir.join("layer.sqlite")).unwrap()
}

fn publish(layer: &mut Metalayer, s: &AnalysisSnapshot) -> i64 {
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
fn rich() -> AnalysisSnapshot {
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
    limited.limited = Some("work".into());
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
    let rows = view.file_references(RelationQuery::default()).unwrap();
    let flat: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.path.as_str(),
                r.target.symbol.as_str(),
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
            .map(|s| (s.span, s.limited.as_deref()))
            .collect::<Vec<_>>(),
        vec![((14, 15), None), ((30, 31), Some("work"))]
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
        Outcome::Resolved(SymbolRef::local("local 0", "b.py"))
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
        Outcome::Ambiguous(vec![SymbolRef::global("g ext."), SymbolRef::global("g z.")])
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
    kept.limited = Some("work".into());
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
    assert_eq!(all[1].limited.as_deref(), Some("work"));
    assert_eq!(all[2].outcome, Outcome::Bounded("queue-length".into()));
    let rows = view.file_references(RelationQuery::default()).unwrap();
    assert_eq!((rows[0].supports, rows[0].limited), (2, 1));
}

#[test]
fn incomplete_input_is_refused_and_the_flags_must_agree_with_the_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut layer = layer(dir.path());
    let mut s = snapshot("s");
    s.input_complete = false;
    assert!(matches!(
        layer.publish(&s, &PublishOptions::default()),
        Err(ScippyError::Incomplete)
    ));
    assert!(layer.view().unwrap().is_none());
    // A snapshot that claims exactness over a bounded reference is a bug of the caller.
    let mut liar = snapshot("s");
    liar.references = vec![reference(
        "a.py",
        0,
        1,
        ReferenceOutcome::Bounded("work".into()),
    )];
    assert!(layer.publish(&liar, &PublishOptions::default()).is_err());
    assert!(layer.view().unwrap().is_none());
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
    let rows = view.file_references(RelationQuery::default()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].target, SymbolRef::global("first"));
    let references = view.references(ReferenceQuery::default()).unwrap();
    assert_eq!(
        references[0].outcome,
        Outcome::Resolved(SymbolRef::global("first"))
    );
}

#[test]
fn a_revision_follows_its_parent_and_older_revisions_go() {
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
    assert_eq!((info.id, info.parent), (2, Some(1)));
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
        assert_eq!(references[0].outcome, Outcome::Resolved(expected.clone()));
        let rows = view.file_references(RelationQuery::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target, expected);
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
