//! Run the identity scenarios of METALAYER_IDENTITY.md section 9.A.
//!
//! Each scenario publishes tree A, then tree B, into one database. The test reads the identity
//! store with rusqlite and checks the outcome and the tier of each entity. The trees live in
//! `tests/fixtures/identity/`. `base` is tree A of every code scenario. `md_base` is tree A of
//! every Markdown scenario. Every other directory is a complete tree B.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::{Connection, OpenFlags};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/identity")
        .join(name)
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// An entity with its current locator.
#[derive(Clone, Debug)]
struct Ent {
    id: String,
    kind: String,
    parent_id: Option<String>,
    key: String,
    symbol: Option<String>,
    path: String,
    name: String,
    body_digest: Option<String>,
    locators: i64,
}

#[derive(Clone, Debug)]
struct Out {
    outcome: String,
    tier: Option<String>,
    candidates: Option<String>,
}

/// The identity store after one publish.
struct Snap {
    ents: BTreeMap<String, Ent>,
    outcomes: BTreeMap<String, Out>,
}

impl Snap {
    fn ids(&self) -> BTreeSet<String> {
        self.ents.keys().cloned().collect()
    }

    /// The entities of one kind at one path, with one name.
    fn find(&self, path: &str, kind: &str, name: &str) -> &Ent {
        let found: Vec<&Ent> = self
            .ents
            .values()
            .filter(|e| e.path == path && e.kind == kind && e.name == name)
            .collect();
        assert_eq!(
            found.len(),
            1,
            "want one {kind} `{name}` in {path}, found {found:?}"
        );
        found[0]
    }

    fn param(&self, parent: &str, name: &str) -> Option<&Ent> {
        self.ents.values().find(|e| {
            e.kind == "parameter" && e.parent_id.as_deref() == Some(parent) && e.name == name
        })
    }

    /// The entities with a locator path, parameters included.
    fn at_path(&self, path: &str) -> Vec<&Ent> {
        self.ents.values().filter(|e| e.path == path).collect()
    }

    fn outcome(&self, id: &str) -> Option<&Out> {
        self.outcomes.get(id)
    }

    fn assert_moved(&self, id: &str, tiers: &[&str]) {
        let out = self
            .outcome(id)
            .unwrap_or_else(|| panic!("no outcome for {:?}", self.ents.get(id)));
        assert_eq!(out.outcome, "moved", "{:?}", self.ents.get(id));
        let tier = out.tier.as_deref().unwrap_or("");
        assert!(
            tiers.contains(&tier),
            "tier {tier} not in {tiers:?} for {:?}",
            self.ents.get(id)
        );
    }

    fn assert_matched(&self, id: &str) {
        assert!(self.ents.contains_key(id), "entity {id} is gone");
        assert!(
            self.outcome(id).is_none(),
            "entity {:?} has outcome {:?}",
            self.ents.get(id),
            self.outcome(id)
        );
    }

    fn assert_orphaned(&self, id: &str) {
        let out = self
            .outcome(id)
            .unwrap_or_else(|| panic!("no outcome for {:?}", self.ents.get(id)));
        assert_eq!(out.outcome, "orphaned", "{:?}", self.ents.get(id));
    }
}

/// A project root, a facts database, and an identity store in one scratch folder.
struct Lab {
    _scratch: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
    identity: PathBuf,
    output: PathBuf,
}

impl Lab {
    fn new() -> Self {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("project");
        fs::create_dir(&root).unwrap();
        Self {
            db: scratch.path().join("layer.sqlite"),
            identity: scratch.path().join("layer.identity.db"),
            output: scratch.path().join("index.scip"),
            root,
            _scratch: scratch,
        }
    }

    /// Replace the project files with a fixture tree. Keep the root path, so the root set stays.
    fn put(&self, tree: &str) {
        for entry in fs::read_dir(&self.root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                fs::remove_dir_all(&path).unwrap();
            } else {
                fs::remove_file(&path).unwrap();
            }
        }
        copy_tree(&fixture(tree), &self.root);
    }

    /// Put a tree, publish it, and read the identity store.
    fn publish(&self, tree: &str) -> Snap {
        self.put(tree);
        let run = Command::new(env!("CARGO_BIN_EXE_scippy"))
            .args([
                "index",
                self.root.to_str().unwrap(),
                "--output",
                self.output.to_str().unwrap(),
                "--metalayer",
                self.db.to_str().unwrap(),
                "--identity",
                self.identity.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "publish of `{tree}` failed: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        self.snap()
    }

    fn snap(&self) -> Snap {
        let conn =
            Connection::open_with_flags(&self.identity, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut ents = BTreeMap::new();
        let mut stmt = conn
            .prepare(
                "SELECT e.id, e.kind, e.parent_id, l.key, l.symbol, l.path, l.name, \
                 l.body_digest, (SELECT count(*) FROM locator WHERE entity_id = e.id) \
                 FROM entity e JOIN locator l ON l.entity_id = e.id AND l.seq = 0",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok(Ent {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    parent_id: r.get(2)?,
                    key: r.get(3)?,
                    symbol: r.get(4)?,
                    path: r.get(5)?,
                    name: r.get(6)?,
                    body_digest: r.get(7)?,
                    locators: r.get(8)?,
                })
            })
            .unwrap();
        for row in rows {
            let ent = row.unwrap();
            ents.insert(ent.id.clone(), ent);
        }
        let mut outcomes = BTreeMap::new();
        let mut stmt = conn
            .prepare("SELECT entity_id, outcome, tier, candidates FROM outcome")
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    Out {
                        outcome: r.get(1)?,
                        tier: r.get(2)?,
                        candidates: r.get(3)?,
                    },
                ))
            })
            .unwrap();
        for row in rows {
            let (id, out) = row.unwrap();
            outcomes.insert(id, out);
        }
        Snap { ents, outcomes }
    }
}

/// Publish `base`, then `tree`.
fn pair(tree: &str) -> (Snap, Snap) {
    let lab = Lab::new();
    let before = lab.publish("base");
    let after = lab.publish(tree);
    (before, after)
}

/// Every old entity keeps its id, its key, and its matched state.
fn assert_all_matched(before: &Snap, after: &Snap) {
    for ent in before.ents.values() {
        after.assert_matched(&ent.id);
        assert_eq!(after.ents[&ent.id].key, ent.key);
    }
}

#[test]
fn base_holds_the_declarations_of_the_scenarios() {
    let lab = Lab::new();
    let snap = lab.publish("base");
    for (path, kind, name) in [
        ("demo/a.py", "function", "alpha"),
        ("demo/a.py", "function", "beta"),
        ("demo/a.py", "class", "Widget"),
        ("demo/a.py", "method", "run"),
        ("demo/a.py", "method", "reset"),
        ("demo/a.py", "variable", "REGISTRY"),
        ("demo/b.py", "function", "use"),
        ("demo/p.py", "function", "shared"),
        ("demo/q.py", "function", "shared"),
    ] {
        snap.find(path, kind, name);
    }
    let alpha = snap.find("demo/a.py", "function", "alpha");
    for name in ["x", "y"] {
        assert!(snap.param(&alpha.id, name).is_some(), "parameter {name}");
    }
    assert!(snap.outcomes.is_empty(), "a first publish has no outcome");
}

#[test]
fn no_op_publish_writes_nothing() {
    let (before, after) = pair("base");
    assert_eq!(before.ids(), after.ids());
    assert!(after.outcomes.is_empty());
    for ent in before.ents.values() {
        assert_eq!(after.ents[&ent.id].locators, ent.locators);
        assert_eq!(after.ents[&ent.id].key, ent.key);
    }
}

#[test]
fn comment_and_whitespace_edits_match_everything() {
    let (before, after) = pair("comments");
    assert_eq!(before.ids(), after.ids());
    assert!(after.outcomes.is_empty());
    assert_all_matched(&before, &after);
}

#[test]
fn insert_and_reorder_keep_old_ids_and_mint_new_ones() {
    let (before, after) = pair("insert_reorder");
    assert_all_matched(&before, &after);
    assert!(after.outcomes.is_empty());
    let new: Vec<&Ent> = after
        .ents
        .values()
        .filter(|e| !before.ents.contains_key(&e.id))
        .collect();
    let gamma = after.find("demo/a.py", "function", "gamma");
    assert!(new.iter().any(|e| e.id == gamma.id));
    // gamma and its three parameters.
    assert_eq!(new.len(), 4, "{new:?}");
    for name in ["first", "second", "third"] {
        assert!(after.param(&gamma.id, name).is_some(), "parameter {name}");
    }
}

#[test]
fn rename_in_a_file_is_moved_by_t5() {
    let (before, after) = pair("rename_decl");
    let alpha = before.find("demo/a.py", "function", "alpha");
    after.assert_moved(&alpha.id, &["T5"]);
    assert_eq!(after.ents[&alpha.id].name, "alpha_total");
    assert_eq!(after.ents[&alpha.id].path, "demo/a.py");
    // The parameters follow the parent. They have no outcome row.
    for name in ["x", "y"] {
        let param = before.param(&alpha.id, name).unwrap();
        after.assert_matched(&param.id);
    }
    assert_eq!(before.ids(), after.ids());
    assert_eq!(after.outcomes.len(), 1, "{:?}", after.outcomes);
}

#[test]
fn move_of_a_declaration_is_moved_by_t3() {
    let (before, after) = pair("move_decl");
    let alpha = before.find("demo/a.py", "function", "alpha");
    after.assert_moved(&alpha.id, &["T3"]);
    assert_eq!(after.ents[&alpha.id].path, "demo/c.py");
    for name in ["x", "y"] {
        let param = before.param(&alpha.id, name).unwrap();
        after.assert_matched(&param.id);
    }
    assert_eq!(after.outcomes.len(), 1, "{:?}", after.outcomes);
    for name in ["beta", "Widget"] {
        let ent = before.find(
            "demo/a.py",
            if name == "beta" { "function" } else { "class" },
            name,
        );
        after.assert_matched(&ent.id);
    }
}

#[test]
fn rename_of_a_file_moves_its_declarations_and_variables() {
    let (before, after) = pair("move_file");
    let moved: Vec<&Ent> = before
        .at_path("demo/a.py")
        .into_iter()
        .filter(|e| e.kind != "parameter")
        .collect();
    // The module, the variable, alpha, Widget, run, reset, and beta.
    assert!(moved.len() >= 7, "{moved:?}");
    for ent in &moved {
        let now = &after.ents[&ent.id];
        assert_eq!(now.path, "demo/aa.py", "{ent:?}");
        // The file pairing (T2f) runs before T3. Both are exact.
        after.assert_moved(&ent.id, &["T2f", "T3"]);
        if ent.kind == "variable" || ent.kind == "module" {
            after.assert_moved(&ent.id, &["T2f"]);
        }
    }
    for ent in before.at_path("demo/a.py") {
        if ent.kind == "parameter" {
            after.assert_matched(&ent.id);
        }
    }
    // The other files stay matched.
    for ent in before.at_path("demo/b.py") {
        after.assert_matched(&ent.id);
    }
}

#[test]
fn copy_of_a_file_mints_new_ids_and_keeps_the_original() {
    let (before, after) = pair("copy_file");
    assert_all_matched(&before, &after);
    assert!(after.outcomes.is_empty(), "{:?}", after.outcomes);
    let copies = after.at_path("demo/a_copy.py");
    assert_eq!(
        copies.len(),
        before.at_path("demo/a.py").len(),
        "{copies:?}"
    );
    for copy in copies {
        assert!(!before.ents.contains_key(&copy.id));
    }
}

#[test]
fn version_bump_matches_everything_by_t1() {
    let (before, after) = pair("version_bump");
    assert_eq!(before.ids(), after.ids());
    assert!(after.outcomes.is_empty(), "{:?}", after.outcomes);
    for ent in before.ents.values() {
        let now = &after.ents[&ent.id];
        assert_eq!(now.key, ent.key);
        // A version bump writes no locator. It updates the symbol in place.
        assert_eq!(now.locators, ent.locators);
        if let (Some(old), Some(new)) = (&ent.symbol, &now.symbol) {
            assert_ne!(old, new, "the symbol keeps the old version: {ent:?}");
        }
    }
}

#[test]
fn signature_change_keeps_parameters_with_kept_names() {
    let (before, after) = pair("sig_change");
    let alpha = before.find("demo/a.py", "function", "alpha");
    after.assert_matched(&alpha.id);
    for name in ["x", "y"] {
        let old = before.param(&alpha.id, name).unwrap();
        let now = after.param(&alpha.id, name).unwrap();
        assert_eq!(old.id, now.id);
        after.assert_matched(&old.id);
    }
    let added = after.param(&alpha.id, "z").unwrap();
    assert!(!before.ents.contains_key(&added.id));
    assert!(after.outcomes.is_empty(), "{:?}", after.outcomes);
}

#[test]
fn renamed_parameter_orphans_the_old_one_and_mints_a_new_one() {
    let (before, after) = pair("param_rename");
    let alpha = before.find("demo/a.py", "function", "alpha");
    after.assert_matched(&alpha.id);
    let kept = before.param(&alpha.id, "x").unwrap();
    after.assert_matched(&kept.id);
    let old = before.param(&alpha.id, "y").unwrap();
    after.assert_orphaned(&old.id);
    let new = after.param(&alpha.id, "w").unwrap();
    assert!(!before.ents.contains_key(&new.id));
    assert!(after.outcome(&new.id).is_none());
}

#[test]
fn branch_switch_orphans_and_switching_back_matches_the_same_ids() {
    let lab = Lab::new();
    let first = lab.publish("base");
    let other = lab.publish("branch_other");
    let gone = ["demo/a.py", "demo/p.py", "demo/q.py"];
    for path in gone {
        for ent in first.at_path(path) {
            other.assert_orphaned(&ent.id);
        }
    }
    let feature = other.find("demo/d.py", "function", "feature_only");
    let feature_id = feature.id.clone();
    let back = lab.publish("base");
    for ent in first.ents.values() {
        back.assert_matched(&ent.id);
        assert_eq!(back.ents[&ent.id].key, ent.key);
    }
    // The entity of the other branch stays in the pool as an orphan.
    back.assert_orphaned(&feature_id);
    // Switching back mints no entity for the old declarations.
    let minted: Vec<&Ent> = back
        .ents
        .values()
        .filter(|e| !first.ents.contains_key(&e.id) && e.id != feature_id)
        .collect();
    let others = other.ids();
    assert!(
        minted.iter().all(|e| others.contains(&e.id)),
        "minted on the way back: {minted:?}"
    );
}

#[test]
fn same_content_in_two_files_one_moves_binds_the_mover_alone() {
    let (before, after) = pair("dup_one_moves");
    let stay = before.find("demo/p.py", "function", "shared");
    let mover = before.find("demo/q.py", "function", "shared");
    after.assert_matched(&stay.id);
    after.assert_moved(&mover.id, &["T2f", "T3"]);
    assert_eq!(after.ents[&mover.id].path, "demo/r.py");
    for name in ["a", "b"] {
        let param = before.param(&mover.id, name).unwrap();
        after.assert_matched(&param.id);
    }
}

#[test]
fn same_content_claimed_by_two_old_entities_is_ambiguous() {
    let (before, after) = pair("dup_claimed_twice");
    let one = before.find("demo/p.py", "function", "shared");
    let two = before.find("demo/q.py", "function", "shared");
    for id in [&one.id, &two.id] {
        let out = after.outcome(id).expect("an outcome row");
        assert_eq!(out.outcome, "ambiguous", "{out:?}");
        assert!(
            out.candidates.as_deref().unwrap_or("").contains("shared"),
            "{out:?}"
        );
        // The old entity keeps its last locator.
        assert_eq!(after.ents[id].path, before.ents[id].path);
    }
    // The new declaration mints its own id.
    let new = after.find("demo/r.py", "function", "shared");
    assert!(!before.ents.contains_key(&new.id));
    assert!(after.outcome(&new.id).is_none());
}

// ---- Markdown (METALAYER_IDENTITY.md section 7). The `id-markdown` lane builds the engine. ----

fn md_pair(tree: &str) -> (Snap, Snap) {
    let lab = Lab::new();
    let before = lab.publish("md_base");
    let after = lab.publish(tree);
    (before, after)
}

#[test]
#[ignore = "needs id-markdown"]
fn markdown_duplicate_heading_inserted_above_never_binds_silently_wrong() {
    let (before, after) = md_pair("md_dup_heading");
    let old: Vec<&Ent> = before
        .ents
        .values()
        .filter(|e| e.path == "plain.md" && e.kind == "heading" && e.name == "Other")
        .collect();
    assert_eq!(old.len(), 2, "{old:?}");
    for ent in old {
        // A bind that is not ambiguous must keep the section digest.
        let ambiguous = after
            .outcome(&ent.id)
            .is_some_and(|o| o.outcome == "ambiguous" || o.outcome == "orphaned");
        if !ambiguous {
            assert_eq!(after.ents[&ent.id].body_digest, ent.body_digest, "{ent:?}");
        }
    }
    let count = after
        .ents
        .values()
        .filter(|e| e.path == "plain.md" && e.kind == "heading" && e.name == "Other")
        .count();
    assert_eq!(count, 3);
}

#[test]
#[ignore = "needs id-markdown"]
fn markdown_rename_with_loci_id_is_matched() {
    let (before, after) = md_pair("md_rename_with_id");
    for ent in before.at_path("with_id.md") {
        after.assert_matched(&ent.id);
        assert_eq!(after.ents[&ent.id].path, "renamed.md");
    }
}

#[test]
#[ignore = "needs id-markdown"]
fn markdown_rename_without_loci_id_is_moved_by_m2() {
    let (before, after) = md_pair("md_rename_without_id");
    let note = before
        .at_path("plain.md")
        .into_iter()
        .find(|e| e.kind == "note")
        .expect("the note entity");
    after.assert_moved(&note.id, &["M2"]);
    assert_eq!(after.ents[&note.id].path, "plain_moved.md");
    // The keys of headings and blocks hold the EntityId of the note. They stay matched.
    for ent in before.at_path("plain.md") {
        if ent.kind != "note" {
            after.assert_matched(&ent.id);
        }
    }
}
