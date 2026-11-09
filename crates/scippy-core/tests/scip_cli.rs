//! End-to-end test of the emitter against the official `scip` CLI.
//!
//! The facts are built by hand. They stand in for the output of a binding stage.
//! The CLI steps skip with a message when `scip` is not on PATH.
//! Set `SCIPPY_REQUIRE_SCIP=1` to fail instead of skip.
//! Set `UPDATE_GOLDEN=1` to rewrite the golden files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use protobuf::{Message, text_format};
use scip::types::Index;
use scippy_core::ProjectRelativePath;
use scippy_core::emit::{Document, IndexData, Occurrence, Role, SymbolInfo, SymbolKind, encode};
use scippy_core::identity::{Descriptor, IdentityBuilder, Package, SymbolId};

const PROJECT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/handbuilt");
const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");

/// Find `word` inside the first `context` of `source`. Return a byte span.
fn span(source: &str, context: &str, word: &str) -> (u32, u32) {
    let start = source.find(context).expect("context") + context.find(word).expect("word");
    (start as u32, (start + word.len()) as u32)
}

fn occurrence(
    source: &str,
    context: &str,
    word: &str,
    symbol: &SymbolId,
    role: Role,
) -> Occurrence {
    let (start_byte, end_byte) = span(source, context, word);
    Occurrence {
        start_byte,
        end_byte,
        symbol: symbol.clone(),
        role,
    }
}

/// Build the facts of the fixture project. `reverse` changes the input order only.
fn facts(root: &Path, reverse: bool) -> IndexData {
    let read = |path: &str| fs::read_to_string(root.join(path)).unwrap();
    let (services, app) = (read("src/services.py"), read("src/app.py"));

    let identity = IdentityBuilder::new(
        "scippy",
        Package {
            manager: "pip".into(),
            name: "proj".into(),
            version: "0.1".into(),
        },
    );
    let process = identity
        .symbol(&[
            Descriptor::Namespace("services".into()),
            Descriptor::Method {
                name: "process".into(),
                disambiguator: String::new(),
            },
        ])
        .unwrap();
    let item = SymbolId::local(0);

    let mut services_occurrences = vec![
        occurrence(
            &services,
            "def process",
            "process",
            &process,
            Role::Definition,
        ),
        occurrence(&services, "for item", "item", &item, Role::Definition),
        occurrence(&services, "print(item)", "item", &item, Role::Reference),
    ];
    let mut app_occurrences = vec![
        occurrence(&app, "import process", "process", &process, Role::Import),
        occurrence(
            &app,
            "naïve = process",
            "process",
            &process,
            Role::Reference,
        ),
    ];
    let mut services_symbols = vec![
        SymbolInfo {
            symbol: process.clone(),
            kind: SymbolKind::Function,
            display_name: "process".into(),
        },
        SymbolInfo {
            symbol: item,
            kind: SymbolKind::Variable,
            display_name: "item".into(),
        },
    ];
    let mut documents = vec![
        Document {
            path: ProjectRelativePath::new("src/services.py").unwrap(),
            language: "Python".into(),
            source: services,
            occurrences: services_occurrences.clone(),
            symbols: services_symbols.clone(),
        },
        Document {
            path: ProjectRelativePath::new("src/app.py").unwrap(),
            language: "Python".into(),
            source: app,
            occurrences: app_occurrences.clone(),
            symbols: vec![],
        },
    ];
    if reverse {
        services_occurrences.reverse();
        app_occurrences.reverse();
        services_symbols.reverse();
        documents[0].occurrences = services_occurrences;
        documents[0].symbols = services_symbols;
        documents[1].occurrences = app_occurrences;
        documents.reverse();
    }
    IndexData {
        project_root: root.to_path_buf(),
        documents,
    }
}

/// Parse the bytes and replace the location-sensitive root with a fixed value.
fn normalised(bytes: &[u8]) -> Index {
    let mut index = Index::parse_from_bytes(bytes).unwrap();
    let metadata = index.metadata.as_mut().unwrap();
    metadata.project_root = "file:///ROOT".to_string();
    metadata.tool_info.as_mut().unwrap().version = "VERSION".to_string();
    index
}

fn root() -> PathBuf {
    PathBuf::from(PROJECT)
}

#[test]
fn input_order_does_not_change_the_bytes() {
    let forward = encode(facts(&root(), false)).unwrap();
    let backward = encode(facts(&root(), true)).unwrap();
    assert_eq!(forward, backward);
}

#[test]
fn a_different_root_changes_only_the_root_uri() {
    let copy = tempfile::tempdir().unwrap();
    for file in ["src/services.py", "src/app.py"] {
        fs::create_dir_all(copy.path().join(file).parent().unwrap()).unwrap();
        fs::copy(root().join(file), copy.path().join(file)).unwrap();
    }
    let here = encode(facts(&root(), false)).unwrap();
    let there = encode(facts(copy.path(), true)).unwrap();
    assert_ne!(here, there, "the root URI differs");
    let (here, there) = (normalised(&here), normalised(&there));
    assert_eq!(
        here.write_to_bytes().unwrap(),
        there.write_to_bytes().unwrap()
    );
}

fn check_golden(name: &str, actual: &str) {
    let path = Path::new(GOLDEN).join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
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
fn golden_index_text() {
    let index = normalised(&encode(facts(&root(), false)).unwrap());
    check_golden("index.txtpb", &text_format::print_to_string_pretty(&index));
}

// ---- official scip CLI ----

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

/// Write the index into a scratch directory and return the directory and the index path.
fn written_index() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("index.scip");
    scippy_core::emit::write_index(facts(&root(), false), &path).unwrap();
    (scratch, path)
}

#[test]
fn scip_lint_accepts_the_index() {
    let Some(scip) = scip() else { return };
    let (_scratch, index) = written_index();
    let output = run(&scip, &root(), &["lint", index.to_str().unwrap()]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(!text.to_lowercase().contains("error"), "{text}");
}

#[test]
fn scip_test_accepts_the_annotated_sources() {
    let Some(scip) = scip() else { return };
    let (_scratch, index) = written_index();
    let output = run(
        &scip,
        &root(),
        // `scip test` reads the directory from its first argument. With no argument it fails silently.
        &[
            "test",
            "--from",
            index.to_str().unwrap(),
            "--comment-syntax",
            "#",
            "--check-documents",
            ".",
        ],
    );
    assert!(output.status.success());
}

#[test]
fn scip_snapshot_matches_the_golden_files() {
    let Some(scip) = scip() else { return };
    let (scratch, index) = written_index();
    let out = scratch.path().join("snapshot");
    let output = run(
        &scip,
        &root(),
        &[
            "snapshot",
            "--from",
            index.to_str().unwrap(),
            "--to",
            out.to_str().unwrap(),
            "--comment-syntax",
            "#",
            // The default `--strict=true` fails on every index in scip 0.10.0.
            "--strict=false",
        ],
    );
    assert!(output.status.success());
    for file in ["src/services.py", "src/app.py"] {
        let actual = fs::read_to_string(out.join(file)).unwrap();
        check_golden(&format!("snapshot-{}", file.replace('/', "-")), &actual);
    }
}
