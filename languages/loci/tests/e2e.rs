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
use scippy_loci::{LociPack, Settings, export_relations, list_files};
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
    "code",
    "extract_off",
    "fallback",
    "folders",
    "frontmatter",
    "ids",
    "ids_off",
    "links",
    "path_links_only",
    "same_note",
    "tables",
    "tags",
    "ties",
    "unicode",
];

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

struct Indexed {
    _scratch: tempfile::TempDir,
    path: PathBuf,
    report: IndexReport,
    index: Index,
}

fn index_with(mut request: IndexRequest, packs: &[&dyn LanguagePack]) -> Indexed {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("index.scip");
    request.output = path.clone();
    let report = index_project(&request, packs).unwrap();
    let index = Index::parse_from_bytes(&fs::read(&path).unwrap()).unwrap();
    Indexed {
        _scratch: scratch,
        path,
        report,
        index,
    }
}

fn index_at(root: &Path) -> Indexed {
    index_with(IndexRequest::new(root, ""), &[&LociPack])
}

fn index(name: &str) -> Indexed {
    index_at(&fixture(name))
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
    let root = fixture(name);
    let settings = Settings::read(&root).unwrap().unwrap();
    let files = list_files(&root);
    let mut records = export_relations(&settings, &files, |p| fs::read(root.join(p)).ok());
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
fn the_index_has_an_occurrence_for_each_resolved_link_and_for_no_other() {
    for name in ORACLE_FIXTURES {
        let root = fixture(name);
        let indexed = index_at(&root);
        let records = read_oracle(name);
        for r in records.iter().filter(|r| r["kind"] != "tag") {
            let source = r["source"].as_str().unwrap();
            let doc = indexed.document(source).unwrap();
            let text = fs::read(root.join(source)).unwrap();
            let range = range_of(&text, &r["span"]);
            let got: Vec<&Occurrence> = doc
                .occurrences
                .iter()
                .filter(|o| !is_definition(o) && typed_range(o) == range)
                .collect();
            let context = format!("{name}: {source} {}", r["span"]);
            if r["state"] != "resolved" {
                assert!(got.is_empty(), "{context}: unexpected occurrence");
                continue;
            }
            let target = r["target_path"].as_str().unwrap();
            let target_doc = indexed.document(target).unwrap();
            let want = if r["anchor_state"] == "resolved" {
                let target_text = fs::read(root.join(target)).unwrap();
                definitions_at(target_doc, range_of(&target_text, &r["anchor_span"]))
            } else {
                definitions_at(target_doc, (0, 0, 0, 0))
            };
            assert_eq!(want.len(), 1, "{context}: the target has no definition");
            assert_eq!(got.len(), 1, "{context}: expected one occurrence");
            assert_eq!(got[0].symbol, want[0], "{context}");
            assert_eq!(is_read(got[0]), r["embed"] == true, "{context}: role");
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
    // The emoji takes 4 bytes. The link after it starts at byte column 7 (`😀 emoji ` is 4+1+5+1... ).
    let first = doc
        .occurrences
        .iter()
        .filter(|o| !is_definition(o))
        .map(typed_range)
        .min()
        .unwrap();
    let text = fs::read_to_string(fixture("unicode").join("use.md")).unwrap();
    let line = text.lines().next().unwrap();
    assert_eq!(first.1 as usize, line.find("[[").unwrap());
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
fn an_unsupported_setting_is_a_warning_and_a_bad_manifest_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".loci")).unwrap();
    fs::write(
        root.join(".loci/vault.toml"),
        "schema = 1\nid = \"x\"\n[policy]\nignore_file = \"ig\"\n",
    )
    .unwrap();
    fs::write(root.join("n.md"), "# N\n").unwrap();
    let indexed = index_with(IndexRequest::new(root, ""), &[&LociPack]);
    assert!(
        indexed
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "vault-setting-unsupported")
    );
    fs::write(root.join(".loci/vault.toml"), "schema = 7\nid = \"x\"\n").unwrap();
    let indexed = index_with(IndexRequest::new(root, ""), &[&LociPack]);
    assert!(
        indexed
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "vault-manifest-invalid" && d.severity == Severity::Error)
    );
    assert!(indexed.document("n.md").is_none());
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
            &fixture(name),
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
            &fixture(name),
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
    ]);
    for (name, files) in files {
        let indexed = index(name);
        let out = scratch.path().join(name);
        let output = run(
            &scip,
            &fixture(name),
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
