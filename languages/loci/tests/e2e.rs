//! End-to-end tests of the Loci pack through the whole pipeline.
//!
//! Each fixture is a small vault under `tests/fixtures`. The expected relations of most
//! fixtures come from `loci-core`: `tests/oracle/<name>.jsonl` is the output of
//! `loci --json graph/relations` for that vault. See `scripts/loci-oracle.sh`.
//! The tests check three things against that oracle:
//! 1. the resolver gives the same records;
//! 2. the index has an occurrence for each resolved link, and none for the others;
//! 3. `scip lint`, `scip test`, and `scip snapshot` accept the index.
//!
//! The scip CLI steps skip with a message when `scip` is not on PATH.
//! Set `SCIPPY_REQUIRE_SCIP=1` to fail instead of skip.
//! Set `UPDATE_GOLDEN=1` to rewrite the golden snapshots.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use protobuf::Message;
use scip::types::{Document, Index, Occurrence, SymbolRole, occurrence::Typed_range};
use scippy_core::{IndexReport, IndexRequest, LanguagePack, Severity, index_project};
use scippy_loci::{Listing, LociPack, Settings, export_workspace};
use scippy_python::PythonPack;
use serde_json::Value;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
const ORACLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/oracle");
const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");

/// The fixtures with an oracle file.
const ORACLE_FIXTURES: &[&str] = &[
    "aliases_off",
    "aliases_on",
    "anchors",
    "attachments",
    "case_insensitive",
    "case_sensitive",
    "clips",
    "clips_custom",
    "code",
    "extract_off",
    "fallback",
    "folders",
    "frontmatter",
    "ids",
    "ids_off",
    "links",
    "path_links_only",
    "roots",
    "same_note",
    "tables",
    "tags",
    "ties",
    "unicode",
];

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

/// A fixture ready to use. A fixture that has a `vault` folder is a workspace. Its folder `roots`
/// holds the roots. The tests copy it to a scratch folder, write the place of the roots into the
/// `.toml` files (`@ROOTS@`), and make a git repository of each root folder named `git_*`.
struct Prepared {
    _scratch: Option<tempfile::TempDir>,
    /// The vault root.
    root: PathBuf,
}

fn copy_tree(from: &Path, to: &Path, roots: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target, roots);
        } else if entry.path().extension().is_some_and(|e| e == "toml") {
            let text = fs::read_to_string(entry.path()).unwrap();
            fs::write(&target, text.replace("@ROOTS@", roots.to_str().unwrap())).unwrap();
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn prepare(name: &str) -> Prepared {
    let source = fixture(name);
    if !source.join("vault").is_dir() {
        return Prepared {
            _scratch: None,
            root: source,
        };
    }
    let scratch = tempfile::tempdir().unwrap();
    let base = scratch.path().canonicalize().unwrap();
    copy_tree(&source, &base, &base.join("roots"));
    for entry in fs::read_dir(base.join("roots")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_string_lossy().starts_with("git_") {
            for args in [&["init", "-q"][..], &["add", "-A"][..]] {
                let status = Command::new("git")
                    .args(args)
                    .current_dir(entry.path())
                    .status()
                    .unwrap();
                assert!(status.success());
            }
        }
    }
    Prepared {
        root: base.join("vault"),
        _scratch: Some(scratch),
    }
}

impl Prepared {
    /// The place of a vault path on disk.
    fn real(&self, vault_path: &str) -> PathBuf {
        let settings = Settings::read(&self.root).unwrap().unwrap();
        Listing::real_path(&settings, vault_path).unwrap_or_else(|| self.root.join(vault_path))
    }
}

struct Indexed {
    _scratch: tempfile::TempDir,
    _prepared: Option<Prepared>,
    path: PathBuf,
    /// The folder that holds every document of the index at its path, for the scip CLI.
    files: PathBuf,
    report: IndexReport,
    index: Index,
}

fn index_with(mut request: IndexRequest, packs: &[&dyn LanguagePack]) -> Indexed {
    index_prepared(None, &mut request, packs)
}

fn index_prepared(
    prepared: Option<Prepared>,
    request: &mut IndexRequest,
    packs: &[&dyn LanguagePack],
) -> Indexed {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("index.scip");
    request.output = path.clone();
    let report = index_project(request, packs).unwrap();
    let index = Index::parse_from_bytes(&fs::read(&path).unwrap()).unwrap();
    let files = match &prepared {
        Some(p) if p._scratch.is_some() => {
            // The documents of a workspace come from several folders. Gather them.
            let flat = scratch.path().join("files");
            for doc in &index.documents {
                let to = flat.join(&doc.relative_path);
                fs::create_dir_all(to.parent().unwrap()).unwrap();
                fs::copy(p.real(&doc.relative_path), to).unwrap();
            }
            flat
        }
        _ => request.root.clone(),
    };
    Indexed {
        _scratch: scratch,
        _prepared: prepared,
        path,
        files,
        report,
        index,
    }
}

fn index_at(root: &Path) -> Indexed {
    index_with(IndexRequest::new(root, ""), &[&LociPack])
}

fn index(name: &str) -> Indexed {
    let prepared = prepare(name);
    let mut request = IndexRequest::new(&prepared.root, "");
    index_prepared(Some(prepared), &mut request, &[&LociPack])
}

impl Indexed {
    fn document(&self, path: &str) -> Option<&Document> {
        self.index
            .documents
            .iter()
            .find(|d| d.relative_path == path)
    }
}

fn read_oracle(name: &str) -> Vec<Value> {
    let text = fs::read_to_string(Path::new(ORACLE).join(format!("{name}.jsonl"))).unwrap();
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn mine(name: &str) -> Vec<Value> {
    let prepared = prepare(name);
    let mut records = export_workspace(&prepared.root).unwrap();
    records.sort_by(|a, b| (&a.source, a.span).cmp(&(&b.source, b.span)));
    records.iter().map(|r| r.to_json()).collect()
}

// ---- the resolver against loci-core ----

#[test]
fn the_resolver_gives_the_records_of_loci_core() {
    for name in ORACLE_FIXTURES {
        let (expected, actual) = (read_oracle(name), mine(name));
        assert_eq!(
            actual.len(),
            expected.len(),
            "{name}: the count of relations differs"
        );
        for (a, e) in actual.iter().zip(&expected) {
            assert_eq!(a, e, "{name}: a record differs");
        }
    }
}

#[test]
fn every_state_and_kind_is_in_the_oracle_fixtures() {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut anchors: HashSet<(String, String)> = HashSet::new();
    for name in ORACLE_FIXTURES {
        for r in read_oracle(name) {
            let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
            seen.insert((s("kind"), s("state")));
            anchors.insert((s("anchor_state"), s("anchor_kind")));
        }
    }
    for kind in [
        "wikilink",
        "embed",
        "markdown_link",
        "markdown_embed",
        "frontmatter_link",
    ] {
        for state in ["resolved", "missing"] {
            assert!(
                seen.contains(&(kind.to_string(), state.to_string())),
                "{kind} {state}"
            );
        }
    }
    assert!(seen.contains(&("tag".to_string(), "none".to_string())));
    assert!(seen.contains(&("wikilink".to_string(), "ambiguous".to_string())));
    for anchor in [
        ("resolved", "heading"),
        ("resolved", "block"),
        ("missing", ""),
        ("none", ""),
    ] {
        assert!(
            anchors.contains(&(anchor.0.to_string(), anchor.1.to_string())),
            "{anchor:?}"
        );
    }
}

// ---- the index against loci-core ----

fn typed_range(o: &Occurrence) -> (i32, i32, i32, i32) {
    match o.typed_range.as_ref().unwrap() {
        Typed_range::SingleLineRange(r) => (r.line, r.start_character, r.line, r.end_character),
        Typed_range::MultiLineRange(r) => {
            (r.start_line, r.start_character, r.end_line, r.end_character)
        }
        _ => panic!("unknown range"),
    }
}

/// The line and the UTF-8 column of a byte offset.
fn position(text: &[u8], at: usize) -> (i32, i32) {
    let before = &text[..at];
    let line = before.iter().filter(|&&b| b == b'\n').count();
    let start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
    (line as i32, (at - start) as i32)
}

fn range_of(text: &[u8], span: &Value) -> (i32, i32, i32, i32) {
    let (a, b) = (
        span[0].as_u64().unwrap() as usize,
        span[1].as_u64().unwrap() as usize,
    );
    let (l1, c1) = position(text, a);
    let (l2, c2) = position(text, b);
    (l1, c1, l2, c2)
}

fn is_definition(o: &Occurrence) -> bool {
    o.symbol_roles & SymbolRole::Definition as i32 != 0
}

fn is_read(o: &Occurrence) -> bool {
    o.symbol_roles & SymbolRole::ReadAccess as i32 != 0
}

fn definitions_at(doc: &Document, range: (i32, i32, i32, i32)) -> Vec<&str> {
    doc.occurrences
        .iter()
        .filter(|o| is_definition(o) && typed_range(o) == range)
        .map(|o| o.symbol.as_str())
        .collect()
}

#[test]
fn the_index_has_the_occurrences_of_each_resolved_link_and_none_for_the_others() {
    for name in ORACLE_FIXTURES {
        let prepared = prepare(name);
        let indexed = index_at(&prepared.root);
        let records = read_oracle(name);
        for r in records.iter().filter(|r| r["kind"] != "tag") {
            let source = r["source"].as_str().unwrap();
            let doc = indexed.document(source).unwrap();
            let text = fs::read(prepared.real(source)).unwrap();
            let range = range_of(&text, &r["span"]);
            let context = format!("{name}: {source} {}", r["span"]);
            // The other links of the note, which can lie inside this one, such as a link in a label.
            let others: HashSet<(i32, i32, i32, i32)> = records
                .iter()
                .filter(|o| o["source"] == r["source"] && o["span"] != r["span"])
                .map(|o| range_of(&text, &o["span"]))
                .collect();
            let inside = |o: &&Occurrence| {
                let (l1, c1, l2, c2) = typed_range(o);
                (l1, c1) >= (range.0, range.1)
                    && (l2, c2) <= (range.2, range.3)
                    && !others.contains(&(l1, c1, l2, c2))
            };
            let got: Vec<&Occurrence> = doc
                .occurrences
                .iter()
                .filter(|o| !is_definition(o))
                .filter(inside)
                .collect();
            if r["state"] != "resolved" {
                assert!(got.is_empty(), "{context}: unexpected occurrence");
                continue;
            }
            let target = r["target_path"].as_str().unwrap();
            let target_doc = indexed.document(target).unwrap();
            let note = definitions_at(target_doc, (0, 0, 0, 0));
            assert_eq!(note.len(), 1, "{context}: the target has no module");
            if r["anchor_state"] == "resolved" {
                // The note and the anchor are two references. The note comes first.
                // A link to its own note has no target part, so it has the anchor reference only.
                let target_text = fs::read(prepared.real(target)).unwrap();
                let anchor = definitions_at(target_doc, range_of(&target_text, &r["anchor_span"]));
                assert_eq!(anchor.len(), 1, "{context}: the anchor has no definition");
                let own = r["target"] == "";
                assert_eq!(got.len(), if own { 1 } else { 2 }, "{context}");
                let last = got.last().unwrap();
                assert_eq!(last.symbol, anchor[0], "{context}: anchor");
                if !own {
                    assert_eq!(got[0].symbol, note[0], "{context}: note");
                    assert!(typed_range(got[0]) < typed_range(last), "{context}: order");
                }
            } else {
                assert_eq!(got.len(), 1, "{context}: expected one occurrence");
                assert_eq!(typed_range(got[0]), range, "{context}: range");
                assert_eq!(got[0].symbol, note[0], "{context}");
            }
            for o in &got {
                assert_eq!(is_read(o), r["embed"] == true, "{context}: role");
            }
        }
    }
}

#[test]
fn an_id_anchor_names_a_footnote_or_an_element_and_a_captured_page_is_unchecked() {
    let indexed = index("ids");
    let doc = indexed.document("target.md").unwrap();
    let wanted = [
        "scippy loci ids . `target.md`/`fn:note`.",
        "scippy loci ids . `target.md`/`id:anchor1`.",
        "scippy loci ids . `target.md`/`id:Box`.",
        "scippy loci ids . `target.md`/`id:inline`.",
    ];
    for symbol in wanted {
        assert!(doc.symbols.iter().any(|s| s.symbol == symbol), "{symbol}");
    }
    // Text in a fence holds no id.
    assert!(
        doc.symbols
            .iter()
            .all(|s| !s.symbol.contains("fenced") && !s.symbol.contains("fn:code"))
    );
    // With the switch off, a link names the note, not the footnote.
    let off = index("ids_off");
    let use_doc = off.document("use.md").unwrap();
    assert!(
        use_doc
            .occurrences
            .iter()
            .all(|o| !o.symbol.contains("fn:note"))
    );
}

#[test]
fn a_same_note_wikilink_names_its_own_note() {
    let indexed = index("same_note");
    let doc = indexed.document("n.md").unwrap();
    // Four links: three name an anchor, and one has a missing anchor, so it names the note.
    let references: Vec<&Occurrence> = doc
        .occurrences
        .iter()
        .filter(|o| !is_definition(o))
        .collect();
    assert_eq!(references.len(), 4);
    assert!(
        references
            .iter()
            .all(|o| o.symbol.starts_with("scippy loci same_note . `n.md`/"))
    );
}

// ---- roots ----

#[test]
fn a_workspace_sends_the_files_of_its_roots_to_the_right_packs() {
    let prepared = prepare("roots");
    let mut request = IndexRequest::new(&prepared.root, "");
    let indexed = index_prepared(Some(prepared), &mut request, &[&PythonPack, &LociPack]);
    let language = |path: &str| indexed.document(path).map(|d| d.language.clone());
    // A Python file of a root gets the Python pack. A note of a root gets the Loci pack.
    assert_eq!(language("code/src/main.py").as_deref(), Some("Python"));
    assert_eq!(language("code/src/util.py").as_deref(), Some("Python"));
    assert_eq!(language("repo/lib.py").as_deref(), Some("Python"));
    assert_eq!(language("code/README.md").as_deref(), Some("Markdown"));
    assert_eq!(language("repo/NOTES.md").as_deref(), Some("Markdown"));
    // The import of the main file reaches the helper in the same root.
    let main = indexed.document("code/src/main.py").unwrap();
    assert!(
        main.occurrences
            .iter()
            .any(|o| !is_definition(o) && o.symbol.contains("helper()."))
    );
    // The link from a note gives the Python document a symbol of the vault.
    assert!(
        main.occurrences
            .iter()
            .any(|o| is_definition(o) && o.symbol.starts_with("scippy loci "))
    );
    // The ignore list, the vault exclude, and the shape rule of the root hide a file from every pack.
    for hidden in [
        "tool.py",
        "code/src/secret.py",
        "code/vendor/dep.py",
        "code/Cargo.lock",
        "code/.env.local",
        "hidden/Hidden.md",
        "private/P.md",
        "areas/ignored/Ignored.md",
        "repo/ignored.txt",
    ] {
        assert!(indexed.document(hidden).is_none(), "{hidden}");
    }
    // A file that is a link target of the vault is a document only when a note links to it.
    assert!(indexed.document("code/logo.png").is_some());
    assert!(indexed.document("repo/bin.dat").is_some());
    assert!(indexed.document("code/src/util.py").is_some());
}

#[test]
fn a_root_lists_its_files_as_loci_core_does() {
    let prepared = prepare("roots");
    let settings = Settings::read(&prepared.root).unwrap().unwrap();
    assert_eq!(
        settings
            .roots
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["inline", "code", "repo"]
    );
    let listing = Listing::of(&settings);
    let sources: Vec<&str> = listing.sources.iter().map(|(p, _)| p.as_str()).collect();
    let attachments: Vec<&str> = listing
        .link_targets
        .iter()
        .map(|(p, _)| p.as_str())
        .collect();
    // A git root lists tracked files. A file that git ignores is not there.
    assert!(sources.contains(&"repo/lib.py"));
    assert!(sources.contains(&"repo/.gitignore"));
    assert!(!sources.contains(&"repo/ignored.txt") && !attachments.contains(&"repo/ignored.txt"));
    // A claimed file that proves binary is an attachment, and so is a claimed file over the cap.
    assert!(attachments.contains(&"repo/bin.dat"));
    assert!(attachments.contains(&"code/src/big.py"));
    assert!(sources.contains(&"code/src/main.py") && sources.contains(&"code/src/lib.rs"));
    // A file that no claim takes is an attachment. Hidden files are in neither tier.
    assert!(attachments.contains(&"code/logo.png"));
    for hidden in [
        "code/src/secret.py",
        "code/vendor/dep.py",
        "code/Cargo.lock",
    ] {
        assert!(
            !sources.contains(&hidden) && !attachments.contains(&hidden),
            "{hidden}"
        );
    }
}

#[test]
fn the_input_limits_follow_the_vault() {
    let prepared = prepare("roots");
    let limits = LociPack.input_limits(&prepared.root).unwrap();
    assert_eq!(limits.max_file_bytes, 1024 * 1024);
    assert!(limits.max_total_bytes > scippy_core::InputLimits::DEFAULT_MAX_TOTAL_BYTES);
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".loci")).unwrap();
    fs::write(
        dir.path().join(".loci/vault.toml"),
        "schema = 1\nid = \"x\"\n[policy]\nnote_max_bytes = 3000000\n",
    )
    .unwrap();
    assert_eq!(
        LociPack.input_limits(dir.path()).unwrap().max_file_bytes,
        3_000_000
    );
    assert!(LociPack.input_limits(&dir.path().join("none")).is_none());
}

// ---- symbols and relationships ----

fn symbol(index: &Index, text: &str) -> Option<scip::types::SymbolInformation> {
    index
        .documents
        .iter()
        .flat_map(|d| d.symbols.iter())
        .chain(index.external_symbols.iter())
        .find(|s| s.symbol == text)
        .cloned()
}

#[test]
fn the_scheme_names_notes_headings_blocks_aliases_attachments_and_tags() {
    let links = index("links");
    let note = "scippy loci links . `target.md`/";
    assert!(symbol(&links.index, note).is_some());
    assert!(symbol(&links.index, &format!("{note}Intro.")).is_some());
    assert!(symbol(&links.index, &format!("{note}`^blk1`.")).is_some());
    // An alias refers to its note.
    let alias = symbol(&links.index, "scippy loci links . `index.md`/`alias:Home`.").unwrap();
    assert_eq!(alias.relationships.len(), 1);
    assert!(alias.relationships[0].is_reference);
    assert_eq!(
        alias.relationships[0].symbol,
        "scippy loci links . `index.md`/"
    );
    // An attachment is a stub document with a definition at 0:0.
    let image = links.document("img.png").unwrap();
    assert_eq!(image.language, "");
    assert_eq!(image.occurrences.len(), 1);
    assert!(is_definition(&image.occurrences[0]));
    assert_eq!(typed_range(&image.occurrences[0]), (0, 0, 0, 0));
    assert_eq!(
        image.occurrences[0].symbol,
        "scippy loci links . `img.png`/"
    );
    // A tag is an external symbol of the tags package. A slash nests it.
    let tags = index("tags");
    assert!(symbol(&tags.index, "scippy loci tags . area/sub/leaf/").is_some());
    assert!(symbol(&tags.index, "scippy loci tags . alpha/").is_some());
    assert_eq!(
        tags.index
            .external_symbols
            .iter()
            .filter(|s| s.symbol.ends_with("alpha/"))
            .count(),
        1
    );
    assert!(tags.report.references_external > 0);
}

#[test]
fn a_tie_has_no_occurrence_and_an_attachment_tie_makes_no_stub() {
    let ties = index("ties");
    let doc = ties.document("link.md").unwrap();
    let references = doc.occurrences.iter().filter(|o| !is_definition(o)).count();
    // Only the two exact paths resolve.
    assert_eq!(references, 2);
    // The stem tie, the alias tie, and the attachment tie. A candidate attachment has a stub.
    assert_eq!(ties.report.references_ambiguous, 3);
    assert!(ties.document("a/pic.png").is_some());
    assert!(ties.document("b/pic.png").is_some());
}

#[test]
fn an_embed_reads_and_a_link_references() {
    let indexed = index("links");
    let doc = indexed.document("index.md").unwrap();
    let reads = doc.occurrences.iter().filter(|o| is_read(o)).count();
    // `![[target]]`, `![[img.png]]`, `![alt](img.png)`.
    assert_eq!(reads, 3);
}

#[test]
fn a_heading_with_a_repeated_text_gets_a_numbered_symbol() {
    let indexed = index("anchors");
    for name in [
        "Repeat",
        "`Repeat~2`",
        "`Child A`",
        "`Child A~2`",
        "Grandchild",
    ] {
        let wanted = format!("scippy loci anchors . `doc.md`/{name}.");
        assert!(
            indexed
                .document("doc.md")
                .unwrap()
                .symbols
                .iter()
                .any(|s| s.symbol == wanted),
            "{wanted}"
        );
    }
}

#[test]
fn the_unicode_fixture_uses_byte_columns() {
    let indexed = index("unicode");
    let doc = indexed.document("use.md").unwrap();
    // The emoji takes 4 bytes. The target part of the link starts after the `[[`, at a byte column.
    let first = doc
        .occurrences
        .iter()
        .filter(|o| !is_definition(o))
        .map(typed_range)
        .min()
        .unwrap();
    let text = fs::read_to_string(fixture("unicode").join("use.md")).unwrap();
    let line = text.lines().next().unwrap();
    assert_eq!(first.1 as usize, line.find("[[").unwrap() + 2);
    assert_ne!(
        line.find("[[").unwrap(),
        line.chars().take_while(|&c| c != '[').count()
    );
}

// ---- selection and mixed roots ----

#[test]
fn a_project_without_a_vault_manifest_gives_its_markdown_to_no_pack() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("README.md"), "# Title\n[[x]]\n").unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let indexed = index_with(IndexRequest::new(dir.path(), ""), &[&PythonPack, &LociPack]);
    assert!(indexed.document("README.md").is_none());
    assert!(indexed.document("a.py").is_some());
    assert!(
        indexed
            .report
            .diagnostics
            .iter()
            .all(|d| d.code != "unsupported-language")
    );
}

#[test]
fn a_mixed_root_indexes_the_vault_and_the_code_and_merges_the_attachment_stub() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".loci")).unwrap();
    fs::write(
        root.join(".loci/vault.toml"),
        "schema = 1\nid = \"01a08b81-648f-70e8-bb5f-cd8c53f0f301\"\n",
    )
    .unwrap();
    fs::write(root.join("script.py"), "def run():\n    return 1\n").unwrap();
    fs::write(root.join("note.md"), "See [[script.py]] and [[note]].\n").unwrap();
    let indexed = index_with(IndexRequest::new(root, ""), &[&PythonPack, &LociPack]);
    // One document for the script. It holds the Python symbols and the stub definition.
    let scripts: Vec<_> = indexed
        .index
        .documents
        .iter()
        .filter(|d| d.relative_path == "script.py")
        .collect();
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].language, "Python");
    assert!(
        scripts[0]
            .occurrences
            .iter()
            .any(|o| o.symbol.starts_with("scippy loci "))
    );
    assert!(
        scripts[0]
            .occurrences
            .iter()
            .any(|o| o.symbol.contains("run()."))
    );
    let note = indexed.document("note.md").unwrap();
    assert!(
        note.occurrences
            .iter()
            .any(|o| o.symbol.ends_with("`script.py`/") && !is_definition(o))
    );
    assert_eq!(note.language, "Markdown");
}

#[test]
fn a_folder_that_the_walker_ignores_is_a_vault_folder_unless_the_vault_excludes_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".loci")).unwrap();
    fs::write(
        root.join(".loci/vault.toml"),
        "schema = 1\nid = \"01a08b81-648f-70e8-bb5f-cd8c53f0f302\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("area/target")).unwrap();
    fs::create_dir_all(root.join("target")).unwrap();
    fs::write(root.join("area/target/deep.md"), "# Deep\n").unwrap();
    fs::write(root.join("target/top.md"), "# Top\n").unwrap();
    fs::write(root.join("n.md"), "[[deep]] [[top]]\n").unwrap();
    let indexed = index_with(IndexRequest::new(root, ""), &[&LociPack]);
    // The default exclude `target/**` matches the root folder only.
    assert!(indexed.document("area/target/deep.md").is_some());
    assert!(indexed.document("target/top.md").is_none());
    let note = indexed.document("n.md").unwrap();
    assert_eq!(
        note.occurrences
            .iter()
            .filter(|o| !is_definition(o))
            .count(),
        1
    );
}

#[test]
fn a_bad_manifest_or_policy_is_an_error_and_the_pack_indexes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".loci")).unwrap();
    fs::write(root.join("n.md"), "# N\n").unwrap();
    let bad = [
        // The ignore list is missing: the vault never means "ignore nothing".
        "schema = 1\nid = \"x\"\n[policy]\nignore_file = \"ig\"\n",
        // A typo in the policy.
        "schema = 1\nid = \"x\"\n[policy]\nignorefile = \"ig\"\n",
        "schema = 7\nid = \"x\"\n",
        // A root with a relative path, a name that clashes with a folder of the vault, a claim
        // for an analyzer that no entry declares.
        "schema = 1\nid = \"x\"\n[[root]]\nname = \"a\"\npath = \"rel\"\n",
        "schema = 1\nid = \"x\"\n[[root]]\nname = \".loci\"\npath = \"/nowhere\"\n",
        "schema = 1\nid = \"x\"\n[[root]]\nname = \"a\"\npath = \"/nowhere\"\nclaims = { c = [\"*\"] }\n",
        "schema = 1\nid = \"x\"\nroots_file = \"missing.toml\"\n",
    ];
    for manifest in bad {
        fs::write(root.join(".loci/vault.toml"), manifest).unwrap();
        let indexed = index_with(IndexRequest::new(root, ""), &[&LociPack]);
        assert!(
            indexed
                .report
                .diagnostics
                .iter()
                .any(|d| d.code == "vault-manifest-invalid" && d.severity == Severity::Error),
            "{manifest}"
        );
        assert!(indexed.document("n.md").is_none(), "{manifest}");
    }
}

#[test]
fn a_root_that_does_not_exist_is_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".loci")).unwrap();
    fs::write(
        root.join(".loci/vault.toml"),
        "schema = 1\nid = \"x\"\n[[root]]\nname = \"gone\"\npath = \"/no/such/folder\"\n",
    )
    .unwrap();
    fs::write(root.join("n.md"), "# N\n").unwrap();
    let indexed = index_with(IndexRequest::new(root, ""), &[&LociPack]);
    assert!(
        indexed
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "root-unavailable" && d.severity == Severity::Warning)
    );
    assert!(indexed.document("n.md").is_some());
}

#[test]
fn the_package_name_is_the_folder_and_the_request_can_replace_it() {
    let mut request = IndexRequest::new(fixture("links"), "");
    request.package_name = Some("notes".to_string());
    let indexed = index_with(request, &[&LociPack]);
    assert!(indexed.index.documents.iter().all(|d| {
        d.symbols
            .iter()
            .all(|s| s.symbol.starts_with("scippy loci notes . "))
    }));
}

#[test]
fn the_same_input_gives_the_same_bytes() {
    let a = index("anchors");
    let b = index("anchors");
    assert_eq!(fs::read(&a.path).unwrap(), fs::read(&b.path).unwrap());
}

// ---- the official scip CLI ----

fn scip() -> Option<PathBuf> {
    let found = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join("scip"))
        .find(|candidate| candidate.is_file());
    if found.is_none() {
        assert!(
            std::env::var_os("SCIPPY_REQUIRE_SCIP").is_none(),
            "the scip CLI is not on PATH and SCIPPY_REQUIRE_SCIP is set"
        );
        eprintln!(
            "SKIPPED: the scip CLI is not on PATH. Install it from https://github.com/scip-code/scip/releases"
        );
    }
    found
}

fn run(scip: &Path, cwd: &Path, args: &[&str]) -> Output {
    let output = Command::new(scip)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    eprintln!(
        "$ scip {} ({})\n{}{}",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn all_fixtures() -> Vec<&'static str> {
    ORACLE_FIXTURES.to_vec()
}

#[test]
fn scip_lint_accepts_every_fixture_index() {
    let Some(scip) = scip() else { return };
    for name in all_fixtures() {
        let indexed = index(name);
        let output = run(
            &scip,
            &indexed.files,
            &["lint", indexed.path.to_str().unwrap()],
        );
        let text = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{name}: {text}");
        assert!(text.trim().is_empty(), "{name}: {text}");
    }
}

#[test]
fn scip_test_accepts_the_annotated_notes() {
    let Some(scip) = scip() else { return };
    for name in all_fixtures() {
        let indexed = index(name);
        // The assertions are Markdown comments of the form `[//]: # ^^^ reference <symbol>`.
        // The fixtures hold files that are not notes, so the document check stays off.
        let output = run(
            &scip,
            &indexed.files,
            &[
                "test",
                "--from",
                indexed.path.to_str().unwrap(),
                "--comment-syntax",
                "[//]: #",
                ".",
            ],
        );
        assert!(output.status.success(), "{name}");
    }
}

fn check_golden(name: &str, actual: &str) {
    let path = Path::new(GOLDEN).join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden file {}", path.display()));
    assert_eq!(
        actual, expected,
        "golden file {name} differs. Run with UPDATE_GOLDEN=1 to update it."
    );
}

#[test]
fn scip_snapshot_matches_the_golden_files() {
    let Some(scip) = scip() else { return };
    let scratch = tempfile::tempdir().unwrap();
    let files: BTreeMap<&str, &[&str]> = BTreeMap::from([
        ("links", &["index.md", "target.md"][..]),
        ("anchors", &["doc.md", "use.md"][..]),
        ("tags", &["tags.md"][..]),
        ("attachments", &["use.md", "files/data.csv"][..]),
        ("ties", &["link.md"][..]),
        ("frontmatter", &["fm.md", "uses.md"][..]),
        ("unicode", &["use.md"][..]),
        ("tables", &["table.md"][..]),
        ("code", &["code.md"][..]),
        ("same_note", &["n.md"][..]),
        ("ids", &["use.md", "target.md"][..]),
        ("roots", &["Home.md", "code/notes/Design.md"][..]),
    ]);
    for (name, files) in files {
        let indexed = index(name);
        let out = scratch.path().join(name);
        let output = run(
            &scip,
            &indexed.files,
            &[
                "snapshot",
                "--from",
                indexed.path.to_str().unwrap(),
                "--to",
                out.to_str().unwrap(),
                "--comment-syntax",
                "[//]:",
                "--strict=false",
            ],
        );
        assert!(output.status.success(), "{name}");
        for file in files {
            let actual = fs::read_to_string(out.join(file)).unwrap();
            check_golden(&format!("{name}/{file}"), &actual);
        }
    }
}
