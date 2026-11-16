//! Tests of the note family of the rebinding engine (tiers M2 to M5 and the duplicate rule).
//!
//! The tests write notes as Markdown text. A small reader in this file makes the records that
//! the Loci pack makes: the real pack has its own tests, which use the same engine.

use rusqlite::Connection;
use scippy_core::{
    AnalysisDocument, AnalysisSnapshot, AnalysisSymbol, DeclRecord, IdentityFamily, OutlineKind,
    OutlineRecord,
};

use crate::publish::sha256_hex;
use crate::*;

/// One note of a test run.
#[derive(Clone)]
struct Note {
    path: String,
    text: String,
    id: Option<String>,
}

fn note(path: &str, text: &str) -> Note {
    Note {
        path: path.into(),
        text: text.into(),
        id: None,
    }
}

impl Note {
    fn id(mut self, id: &str) -> Self {
        self.id = Some(id.into());
        self
    }
}

/// Twelve words. A section of this size is long enough to bind by its digest.
const LONG: &str = "one two three four five six seven eight nine ten eleven twelve";

fn digest(text: &str) -> (String, u32) {
    let words: Vec<&str> = text.split_whitespace().collect();
    (
        sha256_hex(words.join(" ").as_bytes())[..32].to_owned(),
        words.len() as u32,
    )
}

fn symbol(desc: &str, name: &str, kind: &str, record: OutlineRecord) -> AnalysisSymbol {
    let mut decl = DeclRecord::default();
    decl.outline = Some(record);
    AnalysisSymbol {
        symbol: format!("scip-loci loci vault . {desc}"),
        kind: kind.into(),
        display_name: name.into(),
        implements: Vec::new(),
        references: Vec::new(),
        decl: Some(decl),
    }
}

/// The document of a note, as the Loci pack gives it.
fn document(n: &Note) -> AnalysisDocument {
    let mut symbols = vec![symbol(
        &format!("{}/", n.path),
        &n.path,
        "File",
        OutlineRecord {
            kind: OutlineKind::Document,
            external_id: n.id.clone(),
            ..OutlineRecord::default()
        },
    )];
    let lines: Vec<&str> = n.text.lines().collect();
    // `(line, level, text)` of each heading.
    let heads: Vec<(usize, u8, String)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let level = l.chars().take_while(|c| *c == '#').count();
            (level > 0 && l[level..].starts_with(' '))
                .then(|| (i, level as u8, l[level..].trim().to_owned()))
        })
        .collect();
    let mut stack: Vec<(u8, String)> = Vec::new();
    let mut ordinals: std::collections::HashMap<(Vec<String>, String), u32> = Default::default();
    for (k, (line, level, text)) in heads.iter().enumerate() {
        while stack.last().is_some_and(|(l, _)| l >= level) {
            stack.pop();
        }
        let ancestors: Vec<String> = stack.iter().map(|(_, t)| t.clone()).collect();
        stack.push((*level, text.clone()));
        let count = ordinals
            .entry((ancestors.clone(), text.clone()))
            .or_default();
        let ordinal = *count;
        *count += 1;
        let end = heads[k + 1..]
            .iter()
            .find(|h| h.1 <= *level)
            .map_or(lines.len(), |h| h.0);
        let (section, tokens) = digest(&lines[line + 1..end].join("\n"));
        symbols.push(symbol(
            &format!("{}/{text}-{k}.", n.path),
            text,
            "Namespace",
            OutlineRecord {
                kind: OutlineKind::Section,
                level: Some(*level),
                ancestors,
                text: Some(text.clone()),
                ordinal: Some(ordinal),
                section_digest: Some(section),
                section_tokens: Some(tokens),
                ..OutlineRecord::default()
            },
        ));
    }
    for l in &lines {
        if let Some((_, id)) = l.rsplit_once(" ^") {
            symbols.push(symbol(
                &format!("{}/`^{id}`.", n.path),
                &format!("^{id}"),
                "Key",
                OutlineRecord {
                    kind: OutlineKind::Block,
                    block_id: Some(id.into()),
                    ..OutlineRecord::default()
                },
            ));
        }
    }
    AnalysisDocument {
        path: n.path.clone(),
        language: "Markdown".into(),
        pack: Some("loci".into()),
        identity: IdentityFamily::Notes,
        real_path: None,
        sha256: Some(sha256_hex(n.text.as_bytes())),
        source: n.text.clone(),
        stale: false,
        occurrences: Vec::new(),
        symbols,
    }
}

#[derive(Debug, Clone)]
struct Row {
    id: String,
    kind: String,
    path: String,
    name: String,
    container: String,
    key: String,
    parent: Option<String>,
    outcome: Option<String>,
    tier: Option<String>,
}

impl Row {
    /// `matched`, `moved:M2`, `ambiguous:M5`, or `orphaned`.
    fn state(&self) -> String {
        match (&self.outcome, &self.tier) {
            (None, _) => "matched".into(),
            (Some(o), Some(t)) => format!("{o}:{t}"),
            (Some(o), None) => o.clone(),
        }
    }
}

struct World {
    dir: tempfile::TempDir,
    layer: Metalayer,
    ids: FixedIds,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let layer = Metalayer::open(dir.path().join("layer.sqlite")).unwrap();
        Self {
            dir,
            layer,
            ids: FixedIds::default(),
        }
    }

    fn publish(&mut self, notes: &[Note]) -> RebindReport {
        let snapshot = AnalysisSnapshot {
            documents: notes.iter().map(document).collect(),
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
            project_root: "/vault".into(),
            shards: Vec::new(),
            options: String::new(),
            support_inputs: Vec::new(),
        };
        let identity = self.dir.path().join("layer.identity.db");
        self.layer
            .publish_with_identity_ids(&snapshot, &PublishOptions::default(), &identity, &self.ids)
            .unwrap()
            .rebind
            .expect("a rebind report")
    }

    fn rows(&self) -> Vec<Row> {
        let db = Connection::open(self.dir.path().join("layer.identity.db")).unwrap();
        let mut st = db
            .prepare(
                "SELECT e.id, e.kind, l.path, l.name, l.container, l.key, e.parent_id,
                        o.outcome, o.tier
                 FROM entity e JOIN locator l ON l.entity_id = e.id AND l.seq = 0
                 LEFT JOIN outcome o ON o.entity_id = e.id
                 WHERE e.family = 'note' ORDER BY e.id",
            )
            .unwrap();
        st.query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                kind: r.get(1)?,
                path: r.get(2)?,
                name: r.get(3)?,
                container: r.get(4)?,
                key: r.get(5)?,
                parent: r.get(6)?,
                outcome: r.get(7)?,
                tier: r.get(8)?,
            })
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    /// The one row of a kind and a name. The name of a heading is its text.
    fn row(&self, kind: &str, name: &str) -> Row {
        let mut found: Vec<Row> = self
            .rows()
            .into_iter()
            .filter(|r| r.kind == kind && r.name == name)
            .collect();
        assert_eq!(found.len(), 1, "one {kind} named {name}: {found:?}");
        found.remove(0)
    }

    fn at(&self, kind: &str, path: &str, name: &str) -> Row {
        let mut found: Vec<Row> = self
            .rows()
            .into_iter()
            .filter(|r| r.kind == kind && r.path == path && r.name == name)
            .collect();
        assert_eq!(found.len(), 1, "one {kind} {name} in {path}: {found:?}");
        found.remove(0)
    }

    fn note(&self, path: &str) -> Row {
        let mut found: Vec<Row> = self
            .rows()
            .into_iter()
            .filter(|r| r.kind == "note" && r.path == path)
            .collect();
        assert_eq!(found.len(), 1, "one note at {path}");
        found.remove(0)
    }

    fn candidates(&self, id: &str) -> Vec<String> {
        let db = Connection::open(self.dir.path().join("layer.identity.db")).unwrap();
        let text: Option<String> = db
            .query_row(
                "SELECT candidates FROM outcome WHERE entity_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        text.map(|t| serde_json::from_str(&t).unwrap())
            .unwrap_or_default()
    }

    fn count(&self) -> usize {
        self.rows().len()
    }
}

fn long_text(first: &str) -> String {
    format!("{first}\n\n# Top\n\n{LONG}\n\n## Sub\n\nshort ^blk1\n")
}

#[test]
fn a_run_mints_a_note_a_heading_and_a_block() {
    let mut w = World::new();
    let report = w.publish(&[note("a.md", &long_text("intro"))]);
    assert_eq!(report.minted, 4);
    let n = w.note("a.md");
    assert_eq!(n.key, "path:a.md");
    let h = w.row("heading", "Sub");
    assert_eq!(h.parent.as_deref(), Some(n.id.as_str()));
    assert_eq!(h.key, format!("{}/Top/Sub#0", n.id));
    assert_eq!(h.container, "Top");
    let b = w.row("block", "^blk1");
    assert_eq!(b.key, format!("{}/^blk1", n.id));
    let top = w.row("heading", "Top");
    assert_eq!(top.key, format!("{}//Top#0", n.id));
}

#[test]
fn publishing_the_same_notes_twice_changes_nothing() {
    let mut w = World::new();
    let notes = [note("a.md", &long_text("intro"))];
    w.publish(&notes);
    let before = w.rows();
    let second = w.publish(&notes);
    assert!(second.skipped);
    let after = w.rows();
    assert_eq!(before.len(), after.len());
    assert!(after.iter().all(|r| r.state() == "matched"));
}

#[test]
fn an_edit_keeps_every_id_by_t1() {
    let mut w = World::new();
    w.publish(&[note("a.md", &long_text("intro"))]);
    let ids: Vec<String> = w.rows().into_iter().map(|r| r.id).collect();
    let report = w.publish(&[note("a.md", &long_text("a different intro"))]);
    assert_eq!(report.minted, 0);
    assert_eq!(report.bound.get("T1"), Some(&4));
    let after: Vec<String> = w.rows().into_iter().map(|r| r.id).collect();
    assert_eq!(ids, after);
}

#[test]
fn a_rename_without_a_loci_id_binds_the_note_by_m2() {
    let mut w = World::new();
    w.publish(&[note("a.md", &long_text("intro"))]);
    let ids: Vec<String> = w.rows().into_iter().map(|r| r.id).collect();
    let report = w.publish(&[note("sub/b.md", &long_text("intro"))]);
    assert_eq!(report.minted, 0);
    assert_eq!(report.bound.get("M2"), Some(&1));
    let n = w.note("sub/b.md");
    assert_eq!(n.state(), "moved:M2");
    assert_eq!(n.id, ids[0..1].join(""));
    // The headings and the block follow the note and stay matched.
    assert_eq!(w.row("heading", "Top").state(), "matched");
    assert_eq!(w.row("heading", "Top").path, "sub/b.md");
    assert_eq!(w.row("block", "^blk1").state(), "matched");
    let after: Vec<String> = w.rows().into_iter().map(|r| r.id).collect();
    assert_eq!(ids, after);
}

#[test]
fn a_rename_with_a_loci_id_is_matched() {
    let mut w = World::new();
    w.publish(&[note("a.md", &long_text("intro")).id("u1")]);
    let first = w.note("a.md");
    assert_eq!(first.key, "id:u1");
    // The text changes too: the file hash would not bind it.
    let report = w.publish(&[note("b.md", &long_text("new intro")).id("u1")]);
    assert_eq!(report.minted, 0);
    assert_eq!(report.bound.get("T1"), Some(&4));
    let n = w.note("b.md");
    assert_eq!(n.id, first.id);
    assert_eq!(n.state(), "matched");
}

#[test]
fn a_rename_with_an_edit_and_no_loci_id_orphans_the_note_and_its_headings() {
    let mut w = World::new();
    w.publish(&[note("a.md", &long_text("intro"))]);
    let old = w.note("a.md");
    let report = w.publish(&[note("b.md", &long_text("changed"))]);
    // The block id is unique in the vault, so the block follows to the new note (M3).
    assert_eq!(report.minted, 3);
    assert_eq!(report.bound.get("M3"), Some(&1));
    assert_eq!(w.note("a.md").state(), "orphaned");
    assert_eq!(w.note("b.md").state(), "matched");
    assert_ne!(w.note("b.md").id, old.id);
    assert_eq!(w.row("block", "^blk1").state(), "moved:M3");
    let left: Vec<(String, String)> = w
        .rows()
        .into_iter()
        .filter(|r| r.path == "a.md")
        .map(|r| (r.kind.clone(), r.state()))
        .collect();
    assert_eq!(left.len(), 3);
    assert!(left.iter().all(|(_, s)| s == "orphaned"), "{left:?}");
}

#[test]
fn m2_needs_one_candidate() {
    let mut w = World::new();
    w.publish(&[note("a.md", "same text")]);
    // The file moves to two places: a copy and a move cannot be told apart.
    w.publish(&[note("b.md", "same text"), note("c.md", "same text")]);
    let a = w.note("a.md");
    assert_eq!(a.state(), "ambiguous:M2");
    assert_eq!(w.candidates(&a.id), ["path:b.md", "path:c.md"]);
    assert_eq!(w.note("b.md").state(), "matched");
    assert_ne!(w.note("b.md").id, a.id);
}

#[test]
fn a_copy_mints_a_new_note_and_keeps_the_original() {
    let mut w = World::new();
    w.publish(&[note("a.md", "same text")]);
    let a = w.note("a.md");
    let report = w.publish(&[note("a.md", "same text"), note("b.md", "same text")]);
    assert_eq!(report.minted, 1);
    assert_eq!(w.note("a.md").id, a.id);
    assert_eq!(w.note("a.md").state(), "matched");
}

#[test]
fn adding_a_loci_id_to_a_note_binds_it_by_its_path() {
    let mut w = World::new();
    w.publish(&[note("a.md", &long_text("intro"))]);
    let before = w.note("a.md");
    let report = w.publish(&[note("a.md", &long_text("intro")).id("u1")]);
    assert_eq!(report.bound.get("T2"), Some(&1));
    let after = w.note("a.md");
    assert_eq!(after.id, before.id);
    assert_eq!(after.key, "id:u1");
    assert_eq!(after.state(), "moved:T2");
    assert_eq!(w.row("heading", "Top").state(), "matched");
}

#[test]
fn another_loci_id_at_the_same_path_is_another_note() {
    let mut w = World::new();
    w.publish(&[note("a.md", "x").id("u1")]);
    let first = w.note("a.md");
    w.publish(&[note("a.md", "y").id("u2")]);
    assert_eq!(w.count(), 2);
    assert_eq!(
        w.rows().iter().find(|r| r.id == first.id).unwrap().state(),
        "orphaned"
    );
}

#[test]
fn a_loci_id_in_two_notes_is_ignored() {
    let mut w = World::new();
    w.publish(&[note("a.md", "x").id("u1"), note("b.md", "y").id("u1")]);
    assert_eq!(w.note("a.md").key, "path:a.md");
    assert_eq!(w.note("b.md").key, "path:b.md");
    let ids = (w.note("a.md").id, w.note("b.md").id);
    let second = w.publish(&[note("a.md", "x").id("u1"), note("b.md", "y").id("u1")]);
    assert!(second.skipped);
    assert_eq!((w.note("a.md").id, w.note("b.md").id), ids);
}

#[test]
fn a_block_moves_to_another_note_by_m3() {
    let mut w = World::new();
    w.publish(&[
        note("a.md", &format!("# A\n\ntext ^blk1\n{LONG}\n")),
        note("b.md", "# B\n\nother\n"),
    ]);
    let old = w.row("block", "^blk1");
    let b = w.note("b.md");
    let report = w.publish(&[
        note("a.md", &format!("# A\n\n{LONG}\n")),
        note("b.md", "# B\n\nother\ntext ^blk1\n"),
    ]);
    assert_eq!(report.bound.get("M3"), Some(&1));
    let moved = w.row("block", "^blk1");
    assert_eq!(moved.id, old.id);
    assert_eq!(moved.state(), "moved:M3");
    assert_eq!(moved.path, "b.md");
    assert_eq!(moved.parent.as_deref(), Some(b.id.as_str()));
    assert_eq!(moved.key, format!("{}/^blk1", b.id));
}

#[test]
fn m3_leaves_a_block_id_that_two_notes_claim_ambiguous() {
    let mut w = World::new();
    w.publish(&[
        note("a.md", "# A\n\ntext ^blk1\n"),
        note("b.md", "# B\n\nx\n"),
    ]);
    let old = w.row("block", "^blk1");
    w.publish(&[
        note("a.md", "# A\n\ngone\n"),
        note("b.md", "# B\n\nx ^blk1\n"),
        note("c.md", "# C\n\ny ^blk1\n"),
    ]);
    let rows = w.rows();
    let old_row = rows.iter().find(|r| r.id == old.id).unwrap();
    assert_eq!(old_row.state(), "ambiguous:M3");
    assert_eq!(w.candidates(&old.id).len(), 2);
    // The two new blocks have ids of their own.
    assert_eq!(rows.iter().filter(|r| r.kind == "block").count(), 3);
}

#[test]
fn a_heading_moves_under_another_parent_by_m4() {
    let mut w = World::new();
    w.publish(&[note("a.md", "# One\n\n## Item\n\nfoo\n\n# Two\n\nbar\n")]);
    let old = w.row("heading", "Item");
    assert_eq!(old.container, "One");
    let report = w.publish(&[note(
        "a.md",
        "# One\n\nfoo\n\n# Two\n\nbar\n\n## Item\n\nchanged\n",
    )]);
    assert_eq!(report.bound.get("M4"), Some(&1));
    let moved = w.row("heading", "Item");
    assert_eq!(moved.id, old.id);
    assert_eq!(moved.state(), "moved:M4");
    assert_eq!(moved.container, "Two");
}

#[test]
fn m4_does_not_cross_notes() {
    let mut w = World::new();
    w.publish(&[
        note("a.md", "# One\n\n## Item\n\nfoo\n"),
        note("b.md", "# Two\n\nbar\n"),
    ]);
    let old = w.row("heading", "Item");
    w.publish(&[
        note("a.md", "# One\n\nfoo\n"),
        note("b.md", "# Two\n\n## Item\n\nfoo\n"),
    ]);
    let rows = w.rows();
    assert_eq!(
        rows.iter().find(|r| r.id == old.id).unwrap().state(),
        "orphaned"
    );
}

#[test]
fn a_renamed_heading_binds_by_its_section_digest_m5() {
    let mut w = World::new();
    w.publish(&[note("a.md", &format!("# Top\n\n## Old name\n\n{LONG}\n"))]);
    let old = w.row("heading", "Old name");
    let report = w.publish(&[note("a.md", &format!("# Top\n\n## New name\n\n{LONG}\n"))]);
    assert_eq!(report.bound.get("M5"), Some(&1));
    let moved = w.row("heading", "New name");
    assert_eq!(moved.id, old.id);
    assert_eq!(moved.state(), "moved:M5");
}

#[test]
fn a_short_section_does_not_bind_by_m5() {
    let mut w = World::new();
    w.publish(&[note("a.md", "# Top\n\n## Old name\n\nshort text\n")]);
    let old = w.row("heading", "Old name");
    w.publish(&[note("a.md", "# Top\n\n## New name\n\nshort text\n")]);
    let rows = w.rows();
    assert_eq!(
        rows.iter().find(|r| r.id == old.id).unwrap().state(),
        "orphaned"
    );
    assert_ne!(w.row("heading", "New name").id, old.id);
}

#[test]
fn m5_does_not_bind_when_two_sections_have_the_same_digest() {
    let mut w = World::new();
    w.publish(&[note("a.md", &format!("# Top\n\n## Old\n\n{LONG}\n"))]);
    let old = w.row("heading", "Old");
    w.publish(&[note(
        "a.md",
        &format!("# Top\n\n## First\n\n{LONG}\n\n## Second\n\n{LONG}\n"),
    )]);
    let rows = w.rows();
    let old_row = rows.iter().find(|r| r.id == old.id).unwrap();
    assert_eq!(old_row.state(), "ambiguous:M5");
    assert_eq!(w.candidates(&old.id).len(), 2);
}

fn dup(first: &str, second: &str) -> String {
    format!("# Notes\n\n## Item\n\n{first}\n\n## Item\n\n{second}\n")
}

#[test]
fn a_duplicate_heading_inserted_above_binds_by_digest() {
    let mut w = World::new();
    w.publish(&[note("a.md", &dup("alpha alpha", "beta beta"))]);
    let n = w.note("a.md");
    let items = |w: &World| -> Vec<Row> {
        let mut v: Vec<Row> = w.rows().into_iter().filter(|r| r.name == "Item").collect();
        v.sort_by(|a, b| a.key.cmp(&b.key));
        v
    };
    let before = items(&w);
    assert_eq!(before.len(), 2);
    let (first, second) = (before[0].clone(), before[1].clone());
    assert!(first.key.ends_with("#0") && second.key.ends_with("#1"));
    // A new duplicate comes first.
    let text =
        "# Notes\n\n## Item\n\ngamma gamma\n\n## Item\n\nalpha alpha\n\n## Item\n\nbeta beta\n";
    let report = w.publish(&[note("a.md", text)]);
    assert_eq!(report.minted, 1);
    let after = items(&w);
    assert_eq!(after.len(), 3);
    let find = |id: &str| after.iter().find(|r| r.id == id).unwrap().clone();
    // Each old heading keeps its content: it moved one place or two.
    assert_eq!(find(&first.id).key, format!("{}/Notes/Item#1", n.id));
    assert_eq!(find(&second.id).key, format!("{}/Notes/Item#2", n.id));
    assert_eq!(find(&first.id).state(), "moved:M5");
    assert_eq!(find(&second.id).state(), "moved:M5");
}

#[test]
fn a_duplicate_heading_inserted_above_with_an_edit_is_ambiguous_not_wrong() {
    let mut w = World::new();
    w.publish(&[note("a.md", &dup("alpha alpha", "beta beta"))]);
    let old: Vec<Row> = w.rows().into_iter().filter(|r| r.name == "Item").collect();
    // The new duplicate comes first, and the old sections both change.
    let text =
        "# Notes\n\n## Item\n\ngamma gamma\n\n## Item\n\nalpha edited\n\n## Item\n\nbeta edited\n";
    w.publish(&[note("a.md", text)]);
    for o in old {
        let row = w.rows().into_iter().find(|r| r.id == o.id).unwrap();
        assert_eq!(row.state(), "ambiguous:M5", "{row:?}");
        assert_eq!(w.candidates(&o.id).len(), 3);
    }
    assert_eq!(w.rows().iter().filter(|r| r.name == "Item").count(), 5);
}

#[test]
fn a_duplicate_group_of_the_same_size_binds_by_ordinal_after_an_edit() {
    let mut w = World::new();
    w.publish(&[note("a.md", &dup("alpha alpha", "beta beta"))]);
    let old: Vec<Row> = {
        let mut v: Vec<Row> = w.rows().into_iter().filter(|r| r.name == "Item").collect();
        v.sort_by(|a, b| a.key.cmp(&b.key));
        v
    };
    // Both sections change. The ordinal decides.
    let report = w.publish(&[note("a.md", &dup("alpha edited", "beta edited"))]);
    assert_eq!(report.minted, 0);
    for o in &old {
        let row = w.rows().into_iter().find(|r| r.id == o.id).unwrap();
        assert_eq!(row.key, o.key);
        assert_eq!(row.state(), "matched");
    }
}

#[test]
fn a_duplicate_group_of_the_same_size_binds_a_swap_by_digest() {
    let mut w = World::new();
    w.publish(&[note("a.md", &dup("alpha alpha", "beta beta"))]);
    let mut old: Vec<Row> = w.rows().into_iter().filter(|r| r.name == "Item").collect();
    old.sort_by(|a, b| a.key.cmp(&b.key));
    w.publish(&[note("a.md", &dup("beta beta", "alpha alpha"))]);
    let rows = w.rows();
    let first = rows.iter().find(|r| r.id == old[0].id).unwrap();
    let second = rows.iter().find(|r| r.id == old[1].id).unwrap();
    assert!(first.key.ends_with("#1") && second.key.ends_with("#0"));
    assert_eq!(first.state(), "moved:M5");
}

#[test]
fn a_duplicate_removed_leaves_the_other_bound_by_digest() {
    let mut w = World::new();
    w.publish(&[note("a.md", &dup("alpha alpha", "beta beta"))]);
    let mut old: Vec<Row> = w.rows().into_iter().filter(|r| r.name == "Item").collect();
    old.sort_by(|a, b| a.key.cmp(&b.key));
    let text = "# Notes\n\n## Item\n\nbeta beta\n";
    w.publish(&[note("a.md", text)]);
    let rows = w.rows();
    let first = rows.iter().find(|r| r.id == old[0].id).unwrap();
    let second = rows.iter().find(|r| r.id == old[1].id).unwrap();
    assert_eq!(second.state(), "moved:M5");
    assert!(second.key.ends_with("#0"));
    assert_eq!(first.state(), "orphaned");
}

#[test]
fn a_duplicate_removed_with_no_digest_left_is_ambiguous() {
    let mut w = World::new();
    w.publish(&[note("a.md", &dup("alpha alpha", "beta beta"))]);
    let old: Vec<Row> = w.rows().into_iter().filter(|r| r.name == "Item").collect();
    w.publish(&[note("a.md", "# Notes\n\n## Item\n\ngamma gamma\n")]);
    for o in old {
        let row = w.rows().into_iter().find(|r| r.id == o.id).unwrap();
        assert_eq!(row.state(), "ambiguous:M5");
    }
}

#[test]
fn the_children_of_a_removed_note_are_orphaned() {
    let mut w = World::new();
    w.publish(&[note("a.md", &long_text("x")), note("b.md", "# B\n")]);
    w.publish(&[note("b.md", "# B\n")]);
    for r in w.rows().into_iter().filter(|r| r.path == "a.md") {
        assert_eq!(r.state(), "orphaned", "{r:?}");
    }
    assert_eq!(w.at("heading", "b.md", "B").state(), "matched");
}

#[test]
fn code_and_notes_publish_together() {
    // A note document does not make a code entity.
    let mut w = World::new();
    let snapshot = AnalysisSnapshot {
        documents: vec![document(&note("a.md", "# A\n"))],
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
        project_root: "/vault".into(),
        shards: Vec::new(),
        options: String::new(),
        support_inputs: Vec::new(),
    };
    let identity = w.dir.path().join("layer.identity.db");
    w.layer
        .publish_with_identity_ids(&snapshot, &PublishOptions::default(), &identity, &w.ids)
        .unwrap();
    let db = Connection::open(identity).unwrap();
    let code: i64 = db
        .query_row(
            "SELECT count(*) FROM entity WHERE family = 'code'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(code, 0);
}

#[test]
fn the_rebind_selects_the_family_by_the_declared_identity() {
    use crate::identity::Family;
    use crate::rebind::RebindInput;

    let notes = [note("a.md", &long_text("intro"))];
    let snapshot = |identity: IdentityFamily| {
        let mut documents: Vec<AnalysisDocument> = notes.iter().map(document).collect();
        for doc in &mut documents {
            doc.identity = identity;
        }
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
            project_root: "/vault".into(),
            shards: Vec::new(),
            options: String::new(),
            support_inputs: Vec::new(),
        }
    };
    // A document of the notes family feeds the notes input and not the code input.
    let declared = snapshot(IdentityFamily::Notes);
    let code = RebindInput::from_snapshot(&declared, "d", None);
    let from_notes = RebindInput::notes_from_snapshot(&declared, "d");
    assert!(code.items.is_empty());
    assert_eq!(from_notes.family, Family::Note);
    assert_eq!(from_notes.items.len(), 4);
    // The same records under the code family feed the code input and not the notes input.
    let other = snapshot(IdentityFamily::Code);
    assert!(
        RebindInput::notes_from_snapshot(&other, "d")
            .items
            .is_empty()
    );
    assert!(
        !RebindInput::from_snapshot(&other, "d", None)
            .items
            .is_empty()
    );
}
