//! Exercise the persistent experiment against full Scippy runs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use scippy_core::{
    AnalysisSnapshot, IndexRequest, Metalayer, ReferenceOutcome, index_project_with_snapshot,
};
use scippy_loci::LociPack;
use scippy_python::PythonPack;
use scippy_typescript::TypeScriptPack;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn run(
    root: &Path,
    output: PathBuf,
    cache: Option<PathBuf>,
) -> (scippy_core::IndexReport, AnalysisSnapshot) {
    let mut request = IndexRequest::new(root, output);
    request.graph_cache = cache;
    request.jobs = 4;
    index_project_with_snapshot(&request, &[&PythonPack, &TypeScriptPack, &LociPack]).unwrap()
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

#[test]
fn cli_publishes_and_keeps_the_last_revision_on_a_parse_failure() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    fs::create_dir(&root).unwrap();
    write(&root, "module.py", "def f():\n    return 1\n\nf()\n");
    let cache = scratch.path().join("graphs.sqlite");
    let database = scratch.path().join("layer.sqlite");
    let output = scratch.path().join("index.scip");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_scippy"))
            .args([
                "index",
                root.to_str().unwrap(),
                "--output",
                output.to_str().unwrap(),
                "--graph-cache",
                cache.to_str().unwrap(),
                "--metalayer",
                database.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("metalayer revision: 1"));
    let second = run();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(String::from_utf8_lossy(&second.stdout).contains("file graphs: 1 cached, 0 built"));
    assert!(String::from_utf8_lossy(&second.stdout).contains("metalayer revision: 2"));
    write(&root, "module.py", "def broken(:\n");
    let failed = run();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("syntax-error"));
    assert_eq!(
        Metalayer::open(database)
            .unwrap()
            .latest()
            .unwrap()
            .unwrap()
            .0,
        2
    );
}

#[test]
fn cached_graphs_follow_transitive_repository_dependencies() {
    let scratch = tempfile::tempdir().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../languages/loci/tests/fixtures/deps");
    copy_tree(&fixture, scratch.path());
    let root = scratch.path().join("vault");
    let roots_file = root.join("roots.toml");
    let roots = fs::read_to_string(&roots_file)
        .unwrap()
        .replace("@ROOTS@", scratch.path().join("roots").to_str().unwrap());
    fs::write(roots_file, roots).unwrap();
    let cache = scratch.path().join("graphs.sqlite");
    let check = || {
        let (report, cached) = run(
            &root,
            scratch.path().join("cached.scip"),
            Some(cache.clone()),
        );
        let (_, clean) = run(&root, scratch.path().join("clean.scip"), None);
        assert_eq!(cached, clean);
        (report, cached)
    };
    let (_, first) = check();
    assert!(
        matches!(outcome(&first, "app/src/app/main.py", "engine.stop()", "stop"), ReferenceOutcome::Resolved(s) if s.contains("Base#stop"))
    );
    let (warm, _) = check();
    assert!(warm.graph_cache_hits > 0);
    assert_eq!(warm.graph_cache_misses, 0);
    write(
        &scratch.path().join("roots/base"),
        "src/base/types.py",
        "class Base:\n    def halt(self):\n        return None\n",
    );
    let (changed, next) = check();
    assert!(changed.graph_cache_hits > 0);
    assert!(changed.graph_cache_misses > 0);
    assert!(matches!(
        outcome(&next, "app/src/app/main.py", "engine.stop()", "stop"),
        ReferenceOutcome::Unresolved(_) | ReferenceOutcome::Bounded(_)
    ));
}

fn outcome<'a>(
    snapshot: &'a AnalysisSnapshot,
    file: &str,
    text: &str,
    token: &str,
) -> &'a ReferenceOutcome {
    let source = &snapshot
        .documents
        .iter()
        .find(|d| d.path == file)
        .unwrap()
        .source;
    let start = source.find(text).unwrap() + text.rfind(token).unwrap();
    &snapshot
        .references
        .iter()
        .find(|r| {
            r.path == file && r.start == start as u32 && r.end == (start + token.len()) as u32
        })
        .unwrap()
        .outcome
}

#[test]
fn cached_graphs_and_published_relations_match_clean_runs_after_edits() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    fs::create_dir(&root).unwrap();
    let cache = scratch.path().join("graphs.sqlite");
    let database = scratch.path().join("metalayer.sqlite");
    let mut layer = Metalayer::open(&database).unwrap();
    write(
        &root,
        ".loci/vault.toml",
        "schema = 1\nid = \"01a08b81-648f-70e8-bb5f-cd8c53f0f305\"\n",
    );
    write(
        &root,
        "lib.py",
        "class A:\n    value = 1\n\nclass B:\n    value = 2\n\ndef make():\n    return A()\n",
    );
    write(
        &root,
        "use.py",
        "from lib import make, missing\nitem = make()\nprint(item.value)\nprint(item.value)\nmissing()\n",
    );
    write(&root, "a.py", "def f():\n    return 1\n");
    write(&root, "b.py", "def f():\n    return 2\n");
    write(
        &root,
        "ambiguous.py",
        "import sys\nif sys.argv:\n    from a import f\nelse:\n    from b import f\nvalue = f()\n",
    );
    write(
        &root,
        "service.ts",
        "export function hello(): string { return 'hi'; }\n",
    );
    write(
        &root,
        "app.ts",
        "import { hello } from './service';\nhello();\n",
    );
    write(&root, "note.md", "# Note\n\n[[target#Section]]\n");
    write(&root, "target.md", "# Section\n");

    let check = |step: usize| {
        let (report, cached) = run(
            &root,
            scratch.path().join("cached.scip"),
            Some(cache.clone()),
        );
        let (_, clean) = run(&root, scratch.path().join("clean.scip"), None);
        assert_eq!(cached, clean, "cached analysis at step {step}");
        assert_eq!(
            fs::read(scratch.path().join("cached.scip")).unwrap(),
            cached.scip
        );
        (report, cached)
    };

    let (first, initial) = check(0);
    assert_eq!(first.graph_cache_hits, 0);
    assert!(first.graph_cache_misses >= 7);
    assert!(matches!(
        outcome(&initial, "use.py", "missing()", "missing"),
        ReferenceOutcome::Unresolved(_)
    ));
    assert!(matches!(
        outcome(&initial, "ambiguous.py", "value = f()", "f"),
        ReferenceOutcome::Ambiguous(_)
    ));
    assert!(
        matches!(outcome(&initial, "use.py", "print(item.value)", "value"), ReferenceOutcome::Resolved(s) if s.contains("A#value"))
    );
    assert_eq!(layer.publish(&initial).unwrap(), 1);
    let a = layer
        .file_references()
        .unwrap()
        .into_iter()
        .find(|(path, target, _)| path == "use.py" && target.contains("A#value"))
        .unwrap();
    assert_eq!(a.2, 2);

    let (warm, unchanged) = check(1);
    assert_eq!(warm.graph_cache_misses, 0);
    assert!(warm.graph_cache_hits >= 7);
    assert_eq!(layer.latest().unwrap().unwrap(), (1, unchanged));

    write(
        &root,
        "lib.py",
        "class A:\n    value = 1\n\nclass B:\n    value = 2\n\ndef make():\n    return B()\n\ndef missing():\n    pass\n",
    );
    let (changed, next) = check(2);
    assert!(changed.graph_cache_hits >= 6);
    assert_eq!(changed.graph_cache_misses, 1);
    assert!(
        matches!(outcome(&next, "use.py", "print(item.value)", "value"), ReferenceOutcome::Resolved(s) if s.contains("B#value"))
    );
    assert!(matches!(
        outcome(&next, "use.py", "missing()", "missing"),
        ReferenceOutcome::Resolved(_)
    ));
    assert_eq!(layer.publish(&next).unwrap(), 2);
    assert!(
        !layer
            .file_references()
            .unwrap()
            .iter()
            .any(|(path, target, _)| path == "use.py" && target.contains("A#value"))
    );

    write(
        &root,
        "use.py",
        "from lib import make, missing\nitem = make()\nprint(item.value)\nmissing()\n",
    );
    let (_, one) = check(3);
    layer.publish(&one).unwrap();
    let b = layer
        .file_references()
        .unwrap()
        .into_iter()
        .find(|(path, target, _)| path == "use.py" && target.contains("B#value"))
        .unwrap();
    assert_eq!(b.2, 1);

    write(
        &root,
        "use.py",
        "from lib import make, missing\nitem = make()\nmissing()\n",
    );
    write(&root, "target.md", "# Renamed\n");
    fs::remove_file(root.join("b.py")).unwrap();
    let (_, removed) = check(4);
    layer.publish(&removed).unwrap();
    assert!(
        !layer
            .file_references()
            .unwrap()
            .iter()
            .any(|(path, target, _)| path == "use.py" && target.contains("B#value"))
    );
    assert_eq!(layer.latest().unwrap().unwrap(), (4, removed));
    drop(layer);
    assert_eq!(
        Metalayer::open(&database)
            .unwrap()
            .latest()
            .unwrap()
            .unwrap()
            .0,
        4
    );
}
