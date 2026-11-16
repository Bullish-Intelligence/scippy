//! Tests of the annotation API. They seed the store through the helpers of `identity.rs`.

use std::path::Path;

use crate::identity::{IdentityOptions, IdentityStore, join_container};
use crate::*;

fn open(path: &Path) -> IdentityStore {
    IdentityStore::open(path, &IdentityOptions::default()).unwrap()
}

/// Add an entity with one locator. `path`, `container` (display names joined by `.`), and `name`
/// make the locator. The id is `id`.
fn declare(
    store: &mut IdentityStore,
    id: &str,
    kind: EntityKind,
    path: &str,
    container: &[&str],
    name: &str,
) -> Entity {
    let family = match kind {
        EntityKind::Note | EntityKind::Heading | EntityKind::Block => Family::Note,
        _ => Family::Code,
    };
    let entity = Entity {
        id: id.into(),
        family,
        kind,
        parent_id: None,
        created_at: "2026-01-01T00:00:00Z".into(),
    };
    store.insert_entity(&entity).unwrap();
    locate(store, id, path, container, name, "d1");
    entity
}

fn locate(
    store: &mut IdentityStore,
    id: &str,
    path: &str,
    container: &[&str],
    name: &str,
    input: &str,
) {
    let key = format!("{path} {}{name}", container.join("."));
    store
        .push_locator(&Locator {
            entity_id: id.into(),
            seq: 0,
            key: key.clone(),
            symbol: Some(format!("scip py pkg 1.0 {key}")),
            path: path.into(),
            kind: "Function".into(),
            name: name.into(),
            container: join_container(container),
            sig_digest: None,
            body_digest: None,
            body_tokens: None,
            input_digest: input.into(),
        })
        .unwrap();
}

fn flag(
    store: &mut IdentityStore,
    id: &str,
    outcome: BindOutcome,
    tier: Option<Tier>,
    keys: &[&str],
) {
    store
        .set_outcome(&OutcomeRow {
            entity_id: id.into(),
            outcome,
            tier,
            candidates: keys.iter().map(|k| (*k).to_owned()).collect(),
            since_digest: "d2".into(),
        })
        .unwrap();
}

fn note(store: &mut IdentityStore, ids: &FixedIds, id: &str, body: &str) -> Annotation {
    store.add(ids, id, body, None, None).unwrap()
}

fn kind_of<T: std::fmt::Debug>(result: Result<T, IdentityError>) -> &'static str {
    result.expect_err("the call must fail").kind()
}

fn store() -> (tempfile::TempDir, IdentityStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("a.identity.db"));
    (dir, store)
}

#[test]
fn resolve_reads_each_locator_form() {
    let (_dir, mut s) = store();
    declare(&mut s, "m1", EntityKind::Module, "pkg/a.py", &[], "a");
    declare(&mut s, "c1", EntityKind::Class, "pkg/a.py", &[], "Outer");
    declare(
        &mut s,
        "f1",
        EntityKind::Method,
        "pkg/a.py",
        &["Outer"],
        "Inner",
    );
    declare(&mut s, "n1", EntityKind::Note, "notes/x.md", &[], "x.md");
    declare(
        &mut s,
        "h1",
        EntityKind::Heading,
        "notes/x.md",
        &["H1"],
        "H2",
    );
    declare(&mut s, "b1", EntityKind::Block, "notes/x.md", &[], "^blk");

    assert_eq!(s.resolve("f1").unwrap().id, "f1");
    assert_eq!(s.resolve("pkg/a.py::Outer.Inner").unwrap().id, "f1");
    assert_eq!(s.resolve("pkg/a.py::Outer").unwrap().id, "c1");
    assert_eq!(s.resolve("pkg/a.py::").unwrap().id, "m1");
    assert_eq!(s.resolve("pkg/a.py").unwrap().id, "m1");
    assert_eq!(s.resolve("notes/x.md").unwrap().id, "n1");
    assert_eq!(s.resolve("notes/x.md#H1/H2").unwrap().id, "h1");
    assert_eq!(s.resolve("notes/x.md^blk").unwrap().id, "b1");
    // A symbol string, with or without the version.
    let symbol = s.current_locator("f1").unwrap().unwrap().symbol.unwrap();
    assert_eq!(s.resolve(&symbol).unwrap().id, "f1");
    let key = s.current_locator("f1").unwrap().unwrap().key;
    assert_eq!(s.resolve(&key).unwrap().id, "f1");

    assert_eq!(kind_of(s.resolve("pkg/a.py::Missing")), "no-entity");
    assert_eq!(kind_of(s.resolve("notes/x.md#H9")), "no-entity");
    assert_eq!(kind_of(s.resolve("nothing")), "no-entity");
}

#[test]
fn resolve_lists_the_matches_of_an_ambiguous_locator() {
    let (_dir, mut s) = store();
    declare(&mut s, "f1", EntityKind::Function, "pkg/a.py", &[], "pick");
    declare(&mut s, "v1", EntityKind::Variable, "pkg/a.py", &[], "pick");
    let error = s.resolve("pkg/a.py::pick").unwrap_err();
    assert_eq!(error.kind(), "ambiguous-locator");
    let IdentityError::AmbiguousLocator { matches, .. } = &error else {
        panic!("wrong error {error:?}");
    };
    let ids: Vec<&str> = matches.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["f1", "v1"]);
    assert_eq!(matches[0].display, "pkg/a.py::pick");
    let text = error.to_string();
    assert!(text.contains("f1") && text.contains("v1"), "{text}");
}

#[test]
fn resolve_prefers_a_live_entity_to_an_orphan() {
    let (_dir, mut s) = store();
    declare(&mut s, "old", EntityKind::Function, "pkg/a.py", &[], "pick");
    declare(&mut s, "new", EntityKind::Function, "pkg/a.py", &[], "pick");
    flag(&mut s, "old", BindOutcome::Orphaned, None, &[]);
    assert_eq!(s.resolve("pkg/a.py::pick").unwrap().id, "new");
    // An orphan alone still resolves, and its id always does.
    flag(&mut s, "new", BindOutcome::Orphaned, None, &[]);
    assert_eq!(kind_of(s.resolve("pkg/a.py::pick")), "ambiguous-locator");
    assert_eq!(s.resolve("old").unwrap().id, "old");
}

#[test]
fn add_stores_the_snapshot_and_checks_its_input() {
    let (_dir, mut s) = store();
    let ids = FixedIds::default();
    declare(
        &mut s,
        "f1",
        EntityKind::Function,
        "pkg/a.py",
        &["Outer"],
        "go",
    );
    let a = s
        .add(
            &ids,
            "pkg/a.py::Outer.go",
            "keep",
            Some("todo"),
            Some("ann"),
        )
        .unwrap();
    assert_eq!(a.entity_id, "f1");
    assert_eq!(a.tag.as_deref(), Some("todo"));
    assert_eq!(a.author.as_deref(), Some("ann"));
    let snapshot: serde_json::Value = serde_json::from_str(&a.snapshot).unwrap();
    assert_eq!(snapshot["path"], "pkg/a.py");
    assert_eq!(snapshot["container"][0], "Outer");
    assert_eq!(s.get_annotation(&a.id).unwrap(), Some(a));

    assert_eq!(kind_of(s.add(&ids, "f1", "  ", None, None)), "bad-argument");
    assert_eq!(kind_of(s.add(&ids, "zz", "x", None, None)), "no-entity");
}

#[test]
fn list_sorts_and_filters() {
    let (_dir, mut s) = store();
    let ids = FixedIds::default();
    declare(&mut s, "f1", EntityKind::Function, "pkg/b.py", &[], "go");
    declare(&mut s, "f2", EntityKind::Function, "pkg/a.py", &[], "go");
    declare(&mut s, "f3", EntityKind::Function, "pkg/a.py", &[], "also");
    flag(&mut s, "f3", BindOutcome::Orphaned, None, &[]);
    let a1 = note(&mut s, &ids, "f1", "one");
    let a2 = note(&mut s, &ids, "f2", "two");
    let a3 = note(&mut s, &ids, "f3", "three");
    let a4 = note(&mut s, &ids, "f2", "four");

    let all = s.list(&AnnotationFilter::default()).unwrap();
    let order: Vec<&str> = all.iter().map(|e| e.annotation.id.as_str()).collect();
    // pkg/a.py also, pkg/a.py go (two, four), pkg/b.py go.
    assert_eq!(order, [&a3.id, &a2.id, &a4.id, &a1.id]);

    let mut filter = AnnotationFilter {
        path: Some("pkg/a.py".into()),
        ..AnnotationFilter::default()
    };
    assert_eq!(s.list(&filter).unwrap().len(), 3);
    filter.status = Some(BindOutcome::Orphaned);
    let found = s.list(&filter).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].state, BindOutcome::Orphaned);
    assert_eq!(found[0].locator.as_ref().unwrap().name, "also");

    let mut filter = AnnotationFilter {
        entity: Some("pkg/b.py::go".into()),
        ..AnnotationFilter::default()
    };
    assert_eq!(s.list(&filter).unwrap().len(), 1);
    filter.entity = Some("nope".into());
    assert_eq!(kind_of(s.list(&filter)), "no-entity");
}

#[test]
fn remove_deletes_one_annotation() {
    let (_dir, mut s) = store();
    let ids = FixedIds::default();
    declare(&mut s, "f1", EntityKind::Function, "pkg/a.py", &[], "go");
    let a = note(&mut s, &ids, "f1", "one");
    assert_eq!(s.remove(&a.id).unwrap(), a);
    assert_eq!(s.get_annotation(&a.id).unwrap(), None);
    assert_eq!(kind_of(s.remove(&a.id)), "not-found");
}

#[test]
fn status_lists_flagged_entities_with_their_annotation_count() {
    let (_dir, mut s) = store();
    let ids = FixedIds::default();
    declare(&mut s, "f1", EntityKind::Function, "pkg/c.py", &[], "a");
    declare(&mut s, "f2", EntityKind::Function, "pkg/b.py", &[], "a");
    declare(&mut s, "f3", EntityKind::Function, "pkg/a.py", &[], "a");
    declare(&mut s, "f4", EntityKind::Function, "pkg/d.py", &[], "a");
    flag(&mut s, "f1", BindOutcome::Moved, Some(Tier::T3), &[]);
    flag(&mut s, "f2", BindOutcome::Ambiguous, None, &["k1", "k2"]);
    flag(&mut s, "f3", BindOutcome::Orphaned, None, &[]);
    note(&mut s, &ids, "f3", "one");
    note(&mut s, &ids, "f3", "two");
    let rows = s.status().unwrap();
    let seen: Vec<(&str, BindOutcome, usize)> = rows
        .iter()
        .map(|r| (r.entity.id.as_str(), r.outcome.outcome, r.annotations))
        .collect();
    assert_eq!(
        seen,
        [
            ("f3", BindOutcome::Orphaned, 2),
            ("f2", BindOutcome::Ambiguous, 0),
            ("f1", BindOutcome::Moved, 0)
        ]
    );
    assert_eq!(rows[1].outcome.candidates, ["k1", "k2"]);
}

#[test]
fn accept_clears_a_moved_entity_and_refuses_other_states() {
    let (_dir, mut s) = store();
    declare(&mut s, "f1", EntityKind::Function, "pkg/a.py", &[], "a");
    declare(&mut s, "f2", EntityKind::Function, "pkg/a.py", &[], "b");
    declare(&mut s, "f3", EntityKind::Function, "pkg/a.py", &[], "c");
    flag(&mut s, "f1", BindOutcome::Moved, Some(Tier::T3), &[]);
    flag(&mut s, "f3", BindOutcome::Orphaned, None, &[]);

    assert_eq!(kind_of(s.accept("f1", Some("k"))), "bad-argument");
    let done = s.accept("f1", None).unwrap();
    assert_eq!(done.action, AcceptAction::Cleared);
    assert_eq!(s.get_outcome("f1").unwrap(), None);
    // Matched and orphaned entities have nothing to accept.
    assert_eq!(kind_of(s.accept("f1", None)), "bad-argument");
    assert_eq!(kind_of(s.accept("f2", None)), "bad-argument");
    assert_eq!(kind_of(s.accept("f3", None)), "bad-argument");
}

/// An ambiguous entity `old`, and the declarations `n1` and `n2` that it may mean.
fn ambiguity(s: &mut IdentityStore) {
    declare(s, "old", EntityKind::Function, "pkg/a.py", &[], "pick");
    declare(s, "n1", EntityKind::Function, "pkg/b.py", &[], "pick");
    declare(s, "n2", EntityKind::Function, "pkg/c.py", &[], "pick");
    let k1 = s.current_locator("n1").unwrap().unwrap().key;
    let k2 = s.current_locator("n2").unwrap().unwrap().key;
    flag(s, "old", BindOutcome::Ambiguous, None, &[&k1, &k2]);
}

#[test]
fn accept_candidate_binds_and_deletes_the_minted_id() {
    let (_dir, mut s) = store();
    ambiguity(&mut s);
    let key = s.current_locator("n1").unwrap().unwrap().key;
    // A child of the minted entity goes with it.
    s.insert_entity(&Entity {
        id: "p1".into(),
        family: Family::Code,
        kind: EntityKind::Parameter,
        parent_id: Some("n1".into()),
        created_at: "t".into(),
    })
    .unwrap();

    assert_eq!(kind_of(s.accept("old", None)), "bad-argument");
    assert_eq!(kind_of(s.accept("old", Some("other"))), "bad-argument");
    let done = s.accept("old", Some(&key)).unwrap();
    assert_eq!(
        done.action,
        AcceptAction::Bound {
            candidate: key.clone(),
            removed_entity: "n1".into()
        }
    );
    assert_eq!(s.get_outcome("old").unwrap(), None);
    let locators = s.locators("old").unwrap();
    assert_eq!(locators.len(), 2);
    assert_eq!(locators[0].key, key);
    assert_eq!(locators[0].path, "pkg/b.py");
    assert_eq!(locators[1].path, "pkg/a.py");
    assert_eq!(s.get_entity("n1").unwrap(), None);
    assert_eq!(s.get_entity("p1").unwrap(), None);
    assert!(s.locators("n1").unwrap().is_empty());
    assert!(s.get_entity("n2").unwrap().is_some());
}

#[test]
fn accept_candidate_fails_when_the_minted_entity_has_an_annotation() {
    let (_dir, mut s) = store();
    let ids = FixedIds::default();
    ambiguity(&mut s);
    let key = s.current_locator("n1").unwrap().unwrap().key;
    let a = note(&mut s, &ids, "n1", "keep me");
    let error = s.accept("old", Some(&key)).unwrap_err();
    assert_eq!(error.kind(), "candidate-annotated");
    let IdentityError::CandidateAnnotated {
        entity_id,
        annotated_id,
        ..
    } = &error
    else {
        panic!("wrong error {error:?}");
    };
    assert_eq!((entity_id.as_str(), annotated_id.as_str()), ("old", "n1"));
    // Nothing changed.
    assert_eq!(
        s.get_outcome("old").unwrap().unwrap().outcome,
        BindOutcome::Ambiguous
    );
    assert_eq!(s.locators("old").unwrap().len(), 1);
    assert!(s.get_entity("n1").unwrap().is_some());
    assert_eq!(s.get_annotation(&a.id).unwrap(), Some(a));
}

#[test]
fn accept_all_takes_the_moved_entities_of_one_tier() {
    let (_dir, mut s) = store();
    for (id, name) in [("f1", "a"), ("f2", "b"), ("f3", "c"), ("f4", "d")] {
        declare(&mut s, id, EntityKind::Function, "pkg/a.py", &[], name);
    }
    flag(&mut s, "f2", BindOutcome::Moved, Some(Tier::T3), &[]);
    flag(&mut s, "f1", BindOutcome::Moved, Some(Tier::T3), &[]);
    flag(&mut s, "f3", BindOutcome::Moved, Some(Tier::T2), &[]);
    flag(&mut s, "f4", BindOutcome::Ambiguous, None, &["k"]);
    assert_eq!(s.accept_all(Tier::T3).unwrap(), ["f1", "f2"]);
    assert_eq!(s.get_outcome("f1").unwrap(), None);
    assert_eq!(s.get_outcome("f2").unwrap(), None);
    assert!(s.get_outcome("f3").unwrap().is_some());
    assert!(s.get_outcome("f4").unwrap().is_some());
    assert!(s.accept_all(Tier::T3).unwrap().is_empty());
}

#[test]
fn reject_a_moved_entity_vetoes_the_new_key_and_returns_to_the_old_locator() {
    let (_dir, mut s) = store();
    let ids = FixedIds::default();
    declare(&mut s, "f1", EntityKind::Function, "pkg/a.py", &[], "old");
    let old_key = s.current_locator("f1").unwrap().unwrap().key;
    locate(&mut s, "f1", "pkg/b.py", &[], "new", "d2");
    let new_key = s.current_locator("f1").unwrap().unwrap().key;
    flag(&mut s, "f1", BindOutcome::Moved, Some(Tier::T3), &[]);
    let a = note(&mut s, &ids, "f1", "stays");

    assert_eq!(s.reject("f1").unwrap(), std::slice::from_ref(&new_key));
    let row = s.get_outcome("f1").unwrap().unwrap();
    assert_eq!(row.outcome, BindOutcome::Orphaned);
    assert_eq!(row.tier, None);
    assert_eq!(row.since_digest, "d2");
    assert_eq!(s.vetoes("f1").unwrap(), [new_key]);
    let locators = s.locators("f1").unwrap();
    assert_eq!(locators.len(), 1);
    assert_eq!(locators[0].seq, 0);
    assert_eq!(locators[0].key, old_key);
    assert_eq!(s.get_annotation(&a.id).unwrap(), Some(a));
    // An orphan has nothing more to reject.
    assert_eq!(kind_of(s.reject("f1")), "bad-argument");
}

#[test]
fn reject_keeps_the_older_locators_in_order() {
    let (_dir, mut s) = store();
    declare(&mut s, "f1", EntityKind::Function, "pkg/a.py", &[], "one");
    locate(&mut s, "f1", "pkg/b.py", &[], "two", "d2");
    locate(&mut s, "f1", "pkg/c.py", &[], "three", "d3");
    flag(&mut s, "f1", BindOutcome::Moved, Some(Tier::T4), &[]);
    s.reject("f1").unwrap();
    let names: Vec<(i64, String)> = s
        .locators("f1")
        .unwrap()
        .into_iter()
        .map(|l| (l.seq, l.name))
        .collect();
    assert_eq!(names, [(0, "two".into()), (1, "one".into())]);
}

#[test]
fn reject_an_ambiguous_entity_vetoes_each_candidate() {
    let (_dir, mut s) = store();
    ambiguity(&mut s);
    let keys = s.get_outcome("old").unwrap().unwrap().candidates;
    let mut vetoed = s.reject("old").unwrap();
    vetoed.sort();
    let mut expected = keys.clone();
    expected.sort();
    assert_eq!(vetoed, expected);
    assert_eq!(s.vetoes("old").unwrap(), expected);
    assert_eq!(
        s.get_outcome("old").unwrap().unwrap().outcome,
        BindOutcome::Orphaned
    );
    assert_eq!(s.locators("old").unwrap().len(), 1);
}

#[test]
fn reject_refuses_a_matched_entity() {
    let (_dir, mut s) = store();
    declare(&mut s, "f1", EntityKind::Function, "pkg/a.py", &[], "a");
    assert_eq!(kind_of(s.reject("f1")), "bad-argument");
}

#[test]
fn probe_and_read_only_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.identity.db");
    assert_eq!(IdentityStore::probe(&path).unwrap(), IdentityProbe::Missing);
    std::fs::write(&path, "text, not SQLite\n").unwrap();
    assert_eq!(
        IdentityStore::probe(&path).unwrap(),
        IdentityProbe::NotIdentity
    );
    std::fs::remove_file(&path).unwrap();
    let mut s = open(&path);
    declare(&mut s, "f1", EntityKind::Function, "pkg/a.py", &[], "a");
    drop(s);
    assert_eq!(
        IdentityStore::probe(&path).unwrap(),
        IdentityProbe::Version(IDENTITY_VERSION)
    );
    // A facts store is not an identity store.
    let facts = dir.path().join("facts.sqlite");
    drop(Metalayer::open(&facts).unwrap());
    assert_eq!(
        IdentityStore::probe(&facts).unwrap(),
        IdentityProbe::NotIdentity
    );

    let mut ro = IdentityStore::open_read_only(&path).unwrap();
    assert_eq!(ro.resolve("pkg/a.py::a").unwrap().id, "f1");
    assert_eq!(ro.bound_input_digest().unwrap(), None);
    let ids = FixedIds::default();
    assert_eq!(kind_of(ro.add(&ids, "f1", "x", None, None)), "storage");
}
