use std::path::Path;

use rusqlite::Connection;
use scippy_core::ScippyError;

use crate::identity::{
    IDENTITY_APPLICATION_ID, IdSource, Identity, IdentityOptions, Migration, backup_path,
    format_utc,
};
use crate::*;

fn entity(ids: &FixedIds, kind: EntityKind) -> Entity {
    Entity {
        id: ids.new_id(),
        family: Family::Code,
        kind,
        parent_id: None,
        created_at: ids.now(),
    }
}

fn locator(entity_id: &str, key: &str, input: &str) -> Locator {
    Locator {
        entity_id: entity_id.into(),
        seq: 0,
        key: key.into(),
        symbol: Some(format!("scip-python python pkg . {key}")),
        path: "pkg/mod.py".into(),
        kind: "Function".into(),
        name: key.into(),
        container: join_container(&["mod", "Outer"]),
        sig_digest: Some("s".into()),
        body_digest: None,
        body_tokens: Some(12),
        input_digest: input.into(),
    }
}

fn open(path: &Path) -> Identity {
    Identity::open(path, &IdentityOptions::default()).unwrap()
}

fn error_text(result: Result<Identity, ScippyError>) -> String {
    result.expect_err("the open must fail").to_string()
}

#[test]
fn open_creates_the_tables_and_stamps_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.identity.db");
    let store = open(&path);
    assert_eq!(store.version(), IDENTITY_VERSION);
    drop(store);
    let db = Connection::open(&path).unwrap();
    let id: i32 = db
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap();
    let version: i32 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!((id, version), (IDENTITY_APPLICATION_ID, IDENTITY_VERSION));
    assert_ne!(IDENTITY_APPLICATION_ID, crate::schema::APPLICATION_ID);
    let mut tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables.sort();
    assert_eq!(
        tables,
        ["annotation", "entity", "locator", "meta", "outcome", "veto"]
    );
}

#[test]
fn facts_schema_version_stays_7() {
    assert_eq!(SCHEMA_VERSION, 7);
    assert_eq!(IDENTITY_VERSION, 1);
}

#[test]
fn reopen_keeps_the_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.identity.db");
    let ids = FixedIds::default();
    let e = entity(&ids, EntityKind::Function);
    {
        let mut store = open(&path);
        store.insert_entity(&e).unwrap();
    }
    let store = open(&path);
    assert_eq!(store.get_entity(&e.id).unwrap(), Some(e.clone()));
    drop(store);
    let reader = Identity::open_read_only(&path).unwrap();
    assert_eq!(reader.get_entity(&e.id).unwrap(), Some(e));
    assert_eq!(reader.version(), IDENTITY_VERSION);
}

#[test]
fn read_only_open_never_creates_and_never_writes() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("none.identity.db");
    assert!(Identity::open_read_only(&missing).is_err());
    assert!(!missing.exists());
    let path = dir.path().join("a.identity.db");
    drop(open(&path));
    let mut reader = Identity::open_read_only(&path).unwrap();
    let ids = FixedIds::default();
    assert!(
        reader
            .insert_entity(&entity(&ids, EntityKind::Class))
            .is_err()
    );
}

#[test]
fn open_refuses_a_foreign_file() {
    let dir = tempfile::tempdir().unwrap();
    // A facts store has another application id.
    let facts = dir.path().join("facts.db");
    drop(Metalayer::open(&facts).unwrap());
    assert!(
        error_text(Identity::open(&facts, &IdentityOptions::default()))
            .contains("not a Scippy identity store")
    );
    assert!(Identity::open_read_only(&facts).is_err());
    // A file with tables and no application id.
    let other = dir.path().join("other.db");
    Connection::open(&other)
        .unwrap()
        .execute_batch("CREATE TABLE t (x)")
        .unwrap();
    assert!(error_text(Identity::open(&other, &IdentityOptions::default())).contains("has tables"));
}

#[test]
fn open_refuses_a_newer_version_and_leaves_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.identity.db");
    let ids = FixedIds::default();
    let e = entity(&ids, EntityKind::Class);
    {
        let mut store = open(&path);
        store.insert_entity(&e).unwrap();
    }
    Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA user_version=2")
        .unwrap();
    let text = error_text(Identity::open(&path, &IdentityOptions::default()));
    assert!(
        text.contains("version 2") && text.contains("reads version 1"),
        "{text}"
    );
    assert!(Identity::open_read_only(&path).is_err());
    // No table is dropped and no backup exists.
    assert!(!backup_path(&path, 2).exists());
    let db = Connection::open(&path).unwrap();
    let n: i64 = db
        .query_row("SELECT count(*) FROM entity", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

/// The fake step of the migration tests: it adds a column.
fn fake_v0_to_v1(tx: &rusqlite::Transaction<'_>) -> Result<(), ScippyError> {
    tx.execute_batch("ALTER TABLE old ADD COLUMN extra TEXT")
        .map_err(crate::schema::storage)
}

#[test]
fn an_older_store_is_copied_then_migrated_forward() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.identity.db");
    // A version 0 store: the application id is set, the version is 0, one table holds a row.
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch(&format!(
            "CREATE TABLE old (x TEXT); INSERT INTO old VALUES ('kept');
             PRAGMA application_id={IDENTITY_APPLICATION_ID}; PRAGMA user_version=0;"
        ))
        .unwrap();
    }
    let steps: &[Migration] = &[fake_v0_to_v1];
    let store = Identity::open_with(&path, &IdentityOptions::default(), 1, steps).unwrap();
    assert_eq!(store.version(), 1);
    drop(store);

    let db = Connection::open(&path).unwrap();
    let version: i32 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1);
    let kept: String = db
        .query_row("SELECT x FROM old WHERE extra IS NULL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kept, "kept");

    // The backup holds the old version, without the new column.
    let backup = backup_path(&path, 0);
    assert!(backup.ends_with("a.identity.db.bak-v0"), "{backup:?}");
    let old = Connection::open(&backup).unwrap();
    let old_version: i32 = old
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(old_version, 0);
    assert!(old.prepare("SELECT extra FROM old").is_err());
    let kept: String = old
        .query_row("SELECT x FROM old", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kept, "kept");
}

#[test]
fn a_failed_migration_rolls_back_and_keeps_the_backup() {
    fn fail(tx: &rusqlite::Transaction<'_>) -> Result<(), ScippyError> {
        tx.execute_batch("INSERT INTO old VALUES ('half')")
            .map_err(crate::schema::storage)?;
        Err(ScippyError::Storage("fake failure".into()))
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.identity.db");
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch(&format!(
            "CREATE TABLE old (x TEXT);
             PRAGMA application_id={IDENTITY_APPLICATION_ID}; PRAGMA user_version=0;"
        ))
        .unwrap();
    }
    let steps: &[Migration] = &[fail];
    assert!(Identity::open_with(&path, &IdentityOptions::default(), 1, steps).is_err());
    let db = Connection::open(&path).unwrap();
    let rows: i64 = db
        .query_row("SELECT count(*) FROM old", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    assert!(backup_path(&path, 0).exists());
}

#[test]
fn a_missing_migration_step_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.identity.db");
    drop(open(&path));
    // Version 1 to version 2 has no step in the real list.
    let text = error_text(Identity::open_with(
        &path,
        &IdentityOptions::default(),
        2,
        &[],
    ));
    assert!(
        text.contains("no migration from identity store version 1"),
        "{text}"
    );
}

#[test]
fn the_root_set_guard_binds_refuses_and_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.identity.db");
    let ids = FixedIds::default();
    let e = entity(&ids, EntityKind::Module);
    {
        let mut store = Identity::open(&path, &IdentityOptions::with_root_key("key-a")).unwrap();
        assert_eq!(store.root_key().unwrap().as_deref(), Some("key-a"));
        store.insert_entity(&e).unwrap();
    }
    // The same key opens. No key skips the guard.
    drop(Identity::open(&path, &IdentityOptions::with_root_key("key-a")).unwrap());
    drop(open(&path));
    // Another key is refused, and the store stays bound to the first key.
    let result = Identity::open(&path, &IdentityOptions::with_root_key("key-b"));
    match result {
        Err(ScippyError::RootSetMismatch { stored, requested }) => {
            assert_eq!((stored.as_str(), requested.as_str()), ("key-a", "key-b"));
        }
        other => panic!("expected a root set mismatch, got {other:?}"),
    }
    assert_eq!(open(&path).root_key().unwrap().as_deref(), Some("key-a"));
    // A replace binds the new key and keeps the data.
    let mut options = IdentityOptions::with_root_key("key-b");
    options.replace_root_set = true;
    let store = Identity::open(&path, &options).unwrap();
    assert_eq!(store.root_key().unwrap().as_deref(), Some("key-b"));
    assert_eq!(store.get_entity(&e.id).unwrap(), Some(e));
}

#[test]
fn the_default_path_sits_next_to_the_database() {
    assert_eq!(
        default_identity_path(Path::new("/x/y/metalayer.db")),
        Path::new("/x/y/metalayer.identity.db")
    );
    assert_eq!(
        default_identity_path(Path::new("rel/scippy.sqlite")),
        Path::new("rel/scippy.identity.db")
    );
}

#[test]
fn entity_round_trip_and_unknown_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&dir.path().join("a.identity.db"));
    let ids = FixedIds::default();
    let parent = entity(&ids, EntityKind::Function);
    let mut param = entity(&ids, EntityKind::Parameter);
    param.parent_id = Some(parent.id.clone());
    let mut heading = entity(&ids, EntityKind::Heading);
    heading.family = Family::Note;
    for e in [&parent, &param, &heading] {
        store.insert_entity(e).unwrap();
    }
    assert_eq!(store.get_entity(&param.id).unwrap(), Some(param.clone()));
    assert_eq!(
        store.get_entity(&heading.id).unwrap().unwrap().family,
        Family::Note
    );
    assert_eq!(store.get_entity(&ids.new_id()).unwrap(), None);
    // An id is minted once.
    assert!(store.insert_entity(&parent).is_err());
}

#[test]
fn a_locator_needs_an_entity() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&dir.path().join("a.identity.db"));
    assert!(store.push_locator(&locator("missing", "f", "d1")).is_err());
}

#[test]
fn locator_history_keeps_the_newest_sixteen() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&dir.path().join("a.identity.db"));
    let ids = FixedIds::default();
    let e = entity(&ids, EntityKind::Function);
    let other = entity(&ids, EntityKind::Function);
    store.insert_entity(&e).unwrap();
    store.insert_entity(&other).unwrap();
    store
        .push_locator(&locator(&other.id, "other", "o1"))
        .unwrap();
    for n in 0..20 {
        store
            .push_locator(&locator(&e.id, &format!("k{n}"), &format!("d{n}")))
            .unwrap();
    }
    let all = store.locators(&e.id).unwrap();
    assert_eq!(all.len(), LOCATOR_HISTORY);
    // Seq 0 is the newest. Seq 15 is the sixteenth newest.
    let seqs: Vec<i64> = all.iter().map(|l| l.seq).collect();
    assert_eq!(seqs, (0..16).collect::<Vec<i64>>());
    assert_eq!(all[0].key, "k19");
    assert_eq!(all[15].key, "k4");
    assert_eq!(
        store.current_locator(&e.id).unwrap().unwrap().input_digest,
        "d19"
    );
    // Another entity keeps its own history.
    assert_eq!(store.locators(&other.id).unwrap().len(), 1);
    // The fields survive the round trip.
    let first = &all[0];
    assert_eq!(first.containers(), ["mod", "Outer"]);
    assert_eq!(first.sig_digest.as_deref(), Some("s"));
    assert_eq!(first.body_digest, None);
    assert_eq!(first.body_tokens, Some(12));
}

#[test]
fn container_join_and_split() {
    assert_eq!(join_container::<&str>(&[]), "");
    assert!(split_container("").is_empty());
    assert_eq!(
        split_container(&join_container(&["a", "b", "c"])),
        ["a", "b", "c"]
    );
}

#[test]
fn outcome_set_replace_and_clear() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&dir.path().join("a.identity.db"));
    let ids = FixedIds::default();
    let e = entity(&ids, EntityKind::Function);
    store.insert_entity(&e).unwrap();
    assert_eq!(store.get_outcome(&e.id).unwrap(), None);

    let moved = OutcomeRow {
        entity_id: e.id.clone(),
        outcome: BindOutcome::Moved,
        tier: Some(Tier::T2F),
        candidates: Vec::new(),
        since_digest: "d1".into(),
    };
    store.set_outcome(&moved).unwrap();
    assert_eq!(store.get_outcome(&e.id).unwrap(), Some(moved));

    let ambiguous = OutcomeRow {
        entity_id: e.id.clone(),
        outcome: BindOutcome::Ambiguous,
        tier: None,
        candidates: vec!["k1".into(), "k2".into()],
        since_digest: "d2".into(),
    };
    store.set_outcome(&ambiguous).unwrap();
    assert_eq!(store.get_outcome(&e.id).unwrap(), Some(ambiguous));

    let matched = OutcomeRow {
        outcome: BindOutcome::Matched,
        ..store.get_outcome(&e.id).unwrap().unwrap()
    };
    assert!(store.set_outcome(&matched).is_err());

    store.clear_outcome(&e.id).unwrap();
    assert_eq!(store.get_outcome(&e.id).unwrap(), None);
    store.clear_outcome(&e.id).unwrap();
}

#[test]
fn vetoes_are_a_sorted_set() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&dir.path().join("a.identity.db"));
    store.add_veto("e1", "b").unwrap();
    store.add_veto("e1", "a").unwrap();
    store.add_veto("e1", "b").unwrap();
    store.add_veto("e2", "z").unwrap();
    assert_eq!(store.vetoes("e1").unwrap(), ["a", "b"]);
    assert_eq!(store.vetoes("e3").unwrap(), Vec::<String>::new());
}

#[test]
fn annotation_crud() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&dir.path().join("a.identity.db"));
    let ids = FixedIds::default();
    let e = entity(&ids, EntityKind::Function);
    let f = entity(&ids, EntityKind::Class);
    store.insert_entity(&e).unwrap();
    store.insert_entity(&f).unwrap();
    let note = |entity: &Entity, body: &str, time: &str| Annotation {
        id: ids.new_id(),
        entity_id: entity.id.clone(),
        created_at: time.into(),
        author: Some("andrew".into()),
        tag: None,
        body: body.into(),
        snapshot: "{\"path\":\"pkg/mod.py\"}".into(),
    };
    let late = note(&e, "late", "2026-01-02T00:00:00Z");
    let early = note(&e, "early", "2026-01-01T00:00:00Z");
    let mut tagged = note(&f, "tagged", "2026-01-01T00:00:00Z");
    tagged.tag = Some("todo".into());
    tagged.author = None;
    for a in [&late, &early, &tagged] {
        store.insert_annotation(a).unwrap();
    }
    assert_eq!(
        store.get_annotation(&tagged.id).unwrap(),
        Some(tagged.clone())
    );
    assert_eq!(store.get_annotation("none").unwrap(), None);
    // The list is by entity, oldest first.
    assert_eq!(
        store.list_annotations(&e.id).unwrap(),
        [early.clone(), late.clone()]
    );
    assert_eq!(store.list_annotations(&f.id).unwrap(), [tagged.clone()]);
    // An annotation needs an entity.
    let mut orphan = note(&e, "x", "2026-01-03T00:00:00Z");
    orphan.entity_id = "missing".into();
    assert!(store.insert_annotation(&orphan).is_err());
    // Delete.
    assert!(store.delete_annotation(&early.id).unwrap());
    assert!(!store.delete_annotation(&early.id).unwrap());
    assert_eq!(store.list_annotations(&e.id).unwrap(), [late]);
}

#[test]
fn the_bound_input_digest_is_stored_in_meta() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&dir.path().join("a.identity.db"));
    assert_eq!(store.bound_input_digest().unwrap(), None);
    store.set_bound_input_digest("d1").unwrap();
    store.set_bound_input_digest("d2").unwrap();
    assert_eq!(store.bound_input_digest().unwrap().as_deref(), Some("d2"));
}

#[test]
fn enum_text_round_trips() {
    for tier in [
        Tier::T1,
        Tier::T1H,
        Tier::T2F,
        Tier::T2,
        Tier::T3,
        Tier::T4,
        Tier::T5,
        Tier::M2,
        Tier::M3,
        Tier::M4,
        Tier::M5,
        Tier::Param,
    ] {
        assert_eq!(Tier::parse(tier.as_str()), Some(tier));
    }
    for kind in [
        EntityKind::Module,
        EntityKind::Class,
        EntityKind::Function,
        EntityKind::Method,
        EntityKind::Variable,
        EntityKind::Parameter,
        EntityKind::Note,
        EntityKind::Heading,
        EntityKind::Block,
    ] {
        assert_eq!(EntityKind::parse(kind.as_str()), Some(kind));
    }
    for outcome in [
        BindOutcome::Matched,
        BindOutcome::Moved,
        BindOutcome::Ambiguous,
        BindOutcome::Orphaned,
    ] {
        assert_eq!(BindOutcome::parse(outcome.as_str()), Some(outcome));
    }
    assert_eq!(Tier::T1H.as_str(), "T1h");
    assert_eq!(Tier::T2F.as_str(), "T2f");
    assert_eq!(Family::parse("code"), Some(Family::Code));
    assert_eq!(Tier::parse("T9"), None);
}

#[test]
fn id_sources() {
    let fixed = FixedIds::new("2026-11-15T12:00:00Z");
    assert_eq!(fixed.now(), "2026-11-15T12:00:00Z");
    assert_ne!(fixed.new_id(), fixed.new_id());
    assert_eq!(
        FixedIds::default().new_id(),
        "00000000-0000-7000-8000-000000000001"
    );
    let id = SystemIds.new_id();
    let parsed = uuid::Uuid::parse_str(&id).unwrap();
    assert_eq!(parsed.get_version_num(), 7);
    assert_ne!(id, SystemIds.new_id());
    let now = SystemIds.now();
    assert_eq!(now.len(), 20);
    assert!(now.starts_with("20") && now.ends_with('Z'), "{now}");
    assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_utc(1_784_116_800), "2026-07-15T12:00:00Z");
    assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
}
