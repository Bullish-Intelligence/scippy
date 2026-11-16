//! The Loci pack with the identity store: the records of notes, headings, and blocks, and the
//! binds of a rename, a move, and a duplicate heading across two runs.

use std::fs;

use scippy_core::{AnalysisSnapshot, IndexRequest, index_project_with_snapshot};
use scippy_loci::LociPack;
use scippy_metalayer::{
    FixedIds, Identity, Metalayer, PublishOptions, RebindReport, default_identity_path,
};

const MANIFEST: &str = "schema = 1\nid = \"01a08b81-648f-70e8-bb5f-cd8c53f0f303\"\n";
const ID: &str = "927a21d1-fe2d-770f-be66-9b0484ff2871";

struct Vault {
    dir: tempfile::TempDir,
    state: tempfile::TempDir,
    layer: Metalayer,
    ids: FixedIds,
}

fn front(id: &str) -> String {
    format!("---\nloci:\n  schema: 1\n  id: {id}\n---\n")
}

impl Vault {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".loci")).unwrap();
        fs::write(dir.path().join(".loci/vault.toml"), MANIFEST).unwrap();
        let state = tempfile::tempdir().unwrap();
        let layer = Metalayer::open(state.path().join("layer.sqlite")).unwrap();
        Self {
            dir,
            state,
            layer,
            ids: FixedIds::default(),
        }
    }

    fn write(&self, path: &str, text: &str) {
        let full = self.dir.path().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }

    fn remove(&self, path: &str) {
        fs::remove_file(self.dir.path().join(path)).unwrap();
    }

    fn snapshot(&self) -> AnalysisSnapshot {
        let request = IndexRequest::new(self.dir.path(), self.state.path().join("index.scip"));
        index_project_with_snapshot(&request, &[&LociPack])
            .unwrap()
            .1
    }

    fn identity_path(&self) -> std::path::PathBuf {
        default_identity_path(&self.state.path().join("layer.sqlite"))
    }

    fn publish(&mut self) -> RebindReport {
        let snapshot = self.snapshot();
        let path = self.identity_path();
        self.layer
            .publish_with_identity_ids(&snapshot, &PublishOptions::default(), &path, &self.ids)
            .unwrap()
            .rebind
            .unwrap()
    }

    fn store(&self) -> Identity {
        Identity::open_read_only(self.identity_path()).unwrap()
    }

    /// The id and the state of the entity that a locator names.
    fn state(&self, locator: &str) -> (String, String) {
        let detail = self.store().detail(locator).unwrap();
        let state = match detail.outcome {
            None => "matched".to_string(),
            Some(o) => format!(
                "{}:{}",
                o.outcome.as_str(),
                o.tier.map_or("", |t| t.as_str())
            ),
        };
        (detail.entity.id, state)
    }
}

const BODY: &str =
    "# Top\n\none two three four five six seven eight nine ten eleven\n\n## Sub\n\ntext ^blk1\n";

#[test]
fn the_pack_gives_a_record_to_each_note_heading_and_block() {
    let v = Vault::new();
    v.write("a.md", &format!("{}{BODY}## Sub\n\nagain\n", front(ID)));
    let snapshot = v.snapshot();
    let doc = snapshot
        .documents
        .iter()
        .find(|d| d.path == "a.md")
        .unwrap();
    let records: Vec<_> = doc
        .symbols
        .iter()
        .filter_map(|s| {
            s.decl
                .as_ref()?
                .note
                .as_ref()
                .map(|n| (s.display_name.as_str(), n))
        })
        .collect();
    let note = records.iter().find(|(_, n)| n.kind == "note").unwrap().1;
    assert_eq!(note.loci_id.as_deref(), Some(ID));
    let subs: Vec<_> = records
        .iter()
        .filter(|(_, n)| n.text.as_deref() == Some("Sub"))
        .collect();
    // The flat name is `Sub~2`. The ordinal of the identity key counts under one parent.
    assert_eq!(subs.len(), 2);
    assert_eq!(subs[0].0, "Sub");
    assert_eq!(subs[1].0, "Sub~2");
    assert_eq!(subs[0].1.ancestors, ["Top"]);
    assert_eq!(subs[0].1.level, Some(2));
    assert_eq!(subs[0].1.ordinal, Some(0));
    assert_eq!(subs[1].1.ordinal, Some(1));
    let top = records.iter().find(|(n, _)| *n == "Top").unwrap().1;
    assert_eq!(top.section_tokens, Some(18));
    assert!(top.section_digest.as_deref().is_some_and(|d| d.len() == 32));
    let block = records.iter().find(|(_, n)| n.kind == "block").unwrap().1;
    assert_eq!(block.block_id.as_deref(), Some("blk1"));
}

#[test]
fn a_note_without_front_matter_has_no_loci_id() {
    let v = Vault::new();
    v.write("a.md", BODY);
    let snapshot = v.snapshot();
    let doc = snapshot
        .documents
        .iter()
        .find(|d| d.path == "a.md")
        .unwrap();
    let note = doc
        .symbols
        .iter()
        .filter_map(|s| s.decl.as_ref()?.note.as_ref())
        .find(|n| n.kind == "note")
        .unwrap();
    assert_eq!(note.loci_id, None);
}

#[test]
fn a_rename_without_a_loci_id_is_moved_by_m2_and_the_locators_resolve() {
    let mut v = Vault::new();
    v.write("a.md", BODY);
    let first = v.publish();
    assert_eq!(first.minted, 4);
    let (note, _) = v.state("a.md");
    let (heading, _) = v.state("a.md#Top/Sub");
    let (block, _) = v.state("a.md^blk1");
    v.remove("a.md");
    v.write("dir/b.md", BODY);
    let second = v.publish();
    assert_eq!(second.minted, 0);
    assert_eq!(second.bound.get("M2"), Some(&1));
    assert_eq!(v.state("dir/b.md"), (note, "moved:M2".to_string()));
    assert_eq!(
        v.state("dir/b.md#Top/Sub"),
        (heading, "matched".to_string())
    );
    assert_eq!(v.state("dir/b.md^blk1"), (block, "matched".to_string()));
}

#[test]
fn a_rename_with_a_loci_id_is_matched_even_with_an_edit() {
    let mut v = Vault::new();
    v.write("a.md", &format!("{}{BODY}", front(ID)));
    v.publish();
    let (note, _) = v.state("a.md");
    let (heading, _) = v.state("a.md#Top");
    v.remove("a.md");
    v.write("b.md", &format!("{}{BODY}\nmore\n", front(ID)));
    let second = v.publish();
    assert_eq!(second.minted, 0);
    assert_eq!(v.state("b.md"), (note, "matched".to_string()));
    assert_eq!(v.state("b.md#Top"), (heading, "matched".to_string()));
}

#[test]
fn a_duplicate_heading_inserted_above_is_never_a_silent_wrong_bind() {
    let mut v = Vault::new();
    let dup = |a: &str, b: &str| format!("# Notes\n\n## Item\n\n{a}\n\n## Item\n\n{b}\n");
    v.write("a.md", &dup("alpha alpha", "beta beta"));
    v.publish();
    let first = v.store().detail("a.md#Notes/Item").unwrap_err();
    // The locator names two headings: it is ambiguous, and it lists both.
    assert!(format!("{first}").contains("Item"), "{first}");

    let ids = |v: &Vault| -> Vec<String> {
        let db = rusqlite::Connection::open(v.identity_path()).unwrap();
        let mut st = db
            .prepare("SELECT entity_id FROM locator WHERE seq = 0 AND name = 'Item' ORDER BY key")
            .unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let old = ids(&v);
    assert_eq!(old.len(), 2);
    // A new duplicate comes first. The old sections keep their text: they bind by digest.
    v.write(
        "a.md",
        "# Notes\n\n## Item\n\ngamma gamma\n\n## Item\n\nalpha alpha\n\n## Item\n\nbeta beta\n",
    );
    let report = v.publish();
    assert_eq!(report.minted, 1);
    assert_eq!(report.bound.get("M5"), Some(&2));
    let new = ids(&v);
    assert_eq!(new.len(), 3);
    assert!(old.iter().all(|id| new.contains(id)));
    // The old sections both change as well: no bind, and an ambiguous outcome.
    v.write(
        "a.md",
        "# Notes\n\n## Item\n\ndelta\n\n## Item\n\nalpha two\n\n## Item\n\nbeta two\n\n## Item\n\nepsilon\n",
    );
    let report = v.publish();
    assert_eq!(report.ambiguous, 3);
}

#[test]
fn a_block_that_moves_to_another_note_binds_by_m3() {
    let mut v = Vault::new();
    v.write("a.md", "# A\n\ntext ^blk1\n");
    v.write("b.md", "# B\n\nother\n");
    v.publish();
    let (block, _) = v.state("a.md^blk1");
    v.write("a.md", "# A\n\ngone\n");
    v.write("b.md", "# B\n\nother\ntext ^blk1\n");
    let report = v.publish();
    assert_eq!(report.bound.get("M3"), Some(&1));
    assert_eq!(v.state("b.md^blk1"), (block, "moved:M3".to_string()));
}
