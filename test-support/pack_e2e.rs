//! The harness that the Python and TypeScript pack end-to-end tests share.
//!
//! A test crate includes this file with `#[path]`, so `env!` names that crate. Each crate keeps
//! thin wrappers that fix the pack and the label of its shared cache folder.

use std::fs;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use protobuf::Message;
use scip::types::{Index, Occurrence, SymbolRole};
use scippy_core::{IndexReport, IndexRequest, LanguagePack, index_project};

use crate::shared_cache;

pub const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
pub const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");

pub fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

/// A parsed index that the tests only read.
pub struct Cached {
    pub root: PathBuf,
    pub path: PathBuf,
    pub index: Index,
}

/// An index that this test made itself. It adds the report of the run.
pub struct Indexed {
    pub _scratch: tempfile::TempDir,
    pub report: IndexReport,
    pub view: Cached,
}

impl Deref for Indexed {
    type Target = Cached;
    fn deref(&self) -> &Cached {
        &self.view
    }
}

impl DerefMut for Indexed {
    fn deref_mut(&mut self) -> &mut Cached {
        &mut self.view
    }
}

pub fn index_at(pack: &dyn LanguagePack, root: &Path) -> Indexed {
    index_with(pack, IndexRequest::new(root, ""))
}

pub fn index_with(pack: &dyn LanguagePack, mut request: IndexRequest) -> Indexed {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("index.scip");
    request.output = path.clone();
    let report = index_project(&request, &[pack]).unwrap();
    let index = Index::parse_from_bytes(&fs::read(&path).unwrap()).unwrap();
    Indexed {
        _scratch: scratch,
        report,
        view: Cached {
            root: request.root,
            path,
            index,
        },
    }
}

/// The index of a fixture, shared by all test processes. Use it in a test that reads the index
/// and the file and does not read the report. The first process that asks for a fixture indexes
/// it into the target directory, and the others read that file. Nextest runs each test in its
/// own process, so the index of one fixture would else run once for each test that reads it.
/// `label` names the cache folder. It is the same for every call of one test crate.
pub fn shared(label: &str, pack: &dyn LanguagePack, name: &str) -> Cached {
    static SHARED: OnceLock<shared_cache::Shared> = OnceLock::new();
    let shared = SHARED.get_or_init(|| {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(label);
        shared_cache::Shared::new(&base, &[Path::new(FIXTURES)])
    });
    let root = fixture(name);
    let path = shared.file(&format!("{name}.scip"), |path| {
        let mut request = IndexRequest::new(&root, "");
        request.output = path.to_path_buf();
        index_project(&request, &[pack]).unwrap();
    });
    let index = Index::parse_from_bytes(&fs::read(&path).unwrap()).unwrap();
    Cached { root, path, index }
}

impl Cached {
    pub fn document(&self, path: &str) -> Option<&scip::types::Document> {
        self.index
            .documents
            .iter()
            .find(|d| d.relative_path == path)
    }

    /// The occurrences of the document that start on `line` (1-based) at `column`.
    pub fn at(&self, path: &str, line: i32, column: i32) -> Vec<&Occurrence> {
        self.document(path)
            .unwrap_or_else(|| panic!("no document {path}"))
            .occurrences
            .iter()
            .filter(|o| match &o.typed_range {
                Some(scip::types::occurrence::Typed_range::SingleLineRange(r)) => {
                    r.line == line - 1 && r.start_character == column
                }
                _ => false,
            })
            .collect()
    }

    pub fn symbols_at(&self, path: &str, line: i32, column: i32) -> Vec<&str> {
        self.at(path, line, column)
            .iter()
            .map(|o| o.symbol.as_str())
            .collect()
    }

    /// The occurrences of `word` inside the first `context` of the source file.
    /// The fixture files hold assertion comments, so the tests find lines by text.
    pub fn find(&self, path: &str, context: &str, word: &str) -> Vec<&Occurrence> {
        let (line, column) = self.locate(path, context, word);
        self.at(path, line, column)
    }

    pub fn symbols_of(&self, path: &str, context: &str, word: &str) -> Vec<&str> {
        self.find(path, context, word)
            .iter()
            .map(|o| o.symbol.as_str())
            .collect()
    }

    /// The 1-based line and the byte column of `word` in the first `context`.
    pub fn locate(&self, path: &str, context: &str, word: &str) -> (i32, i32) {
        let text = fs::read_to_string(self.root.join(path)).unwrap();
        let at = text.find(context).expect("context") + context.rfind(word).expect("word");
        let line = text[..at].matches('\n').count() as i32 + 1;
        let column = at - text[..at].rfind('\n').map_or(0, |n| n + 1);
        (line, column as i32)
    }

    pub fn has_symbol(&self, path: &str, symbol: &str) -> bool {
        self.document(path)
            .unwrap()
            .occurrences
            .iter()
            .any(|o| o.symbol == symbol)
    }
}

pub fn is_definition(o: &Occurrence) -> bool {
    o.symbol_roles & SymbolRole::Definition as i32 != 0
}

pub fn scip() -> Option<PathBuf> {
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

pub fn run(scip: &Path, cwd: &Path, args: &[&str]) -> Output {
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

/// Every symbol that the index defines: in a document or in the external symbols.
pub fn defined_symbols(index: &Index) -> std::collections::HashSet<&str> {
    index
        .documents
        .iter()
        .flat_map(|d| d.symbols.iter())
        .chain(index.external_symbols.iter())
        .map(|info| info.symbol.as_str())
        .collect()
}

/// The target of a relationship that `scip lint` 0.10.0 reports as missing.
///
/// Returns `None` for any other line.
///
/// The CLI reports this message for a target that a later document defines.
/// It checks the relationships of each document in the same loop that fills its
/// symbol table, and it visits the documents in the random order of a Go map.
/// The count of these lines changes from run to run. The `scip-python` index gets them too:
/// 548, 775 and 1172 lines in three runs on the mypy corpus.
/// See https://github.com/scip-code/scip/blob/v0.10.0/cmd/scip/lint.go#L71-L86
/// The comment at line 79 asks for a second pass, but the code makes none.
pub fn lint_missing_relationship_target(line: &str) -> Option<&str> {
    // The CLI writes a log timestamp before the message: `2026/11/09 10:06:05 `.
    let at = line.find("error: symbol '")?;
    let stamp = &line[..at];
    if !stamp.is_empty()
        && !(stamp.len() == 20
            && stamp
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '/' | ':' | ' ')))
    {
        return None;
    }
    let rest = &line[at + "error: symbol '".len()..];
    let rest = rest.split_once("' (#1) (in symbols for file ")?.1;
    let rest = rest.split_once(") has a relationship to '")?.1;
    let (target, tail) = rest.split_once("' (#2), ")?;
    (tail == "but couldn't find #2 in external symbols or some other document").then_some(target)
}

/// The lines of `scip lint` output that matter.
///
/// Tolerates only the message above, and only when this index does define the target.
pub fn lint_problems(text: &str, index: &Index) -> Vec<String> {
    let defined = defined_symbols(index);
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| {
            !lint_missing_relationship_target(line).is_some_and(|target| defined.contains(target))
        })
        .map(str::to_string)
        .collect()
}

pub fn check_golden(name: &str, actual: &str) {
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
