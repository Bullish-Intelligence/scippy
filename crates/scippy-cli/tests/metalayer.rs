//! Exercise the persistent experiment against full Scippy runs.

mod common;

use common::copy_tree;
use std::fs;
use std::path::{Path, PathBuf};

use scippy_core::{AnalysisSnapshot, IndexRequest, ReferenceOutcome, index_project_with_snapshot};
use scippy_loci::LociPack;
use scippy_metalayer::{Metalayer, PublishOptions, RelationQuery};
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

#[test]
#[ignore = "slow: runs in opt:slow"]
fn cli_publishes_each_run_and_a_parse_failure_gives_a_failed_file() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    fs::create_dir(&root).unwrap();
    write(&root, "module.py", "def f():\n    return 1\n\nf()\n");
    let cache = scratch.path().join("graphs.sqlite");
    let database = scratch.path().join("layer.sqlite");
    let output = scratch.path().join("index.scip");
    let run = || {
        common::scippy()
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
    // A syntax error loses one file. The run publishes it with the status `failed`.
    let failed = run();
    assert_eq!(
        failed.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );
    assert!(String::from_utf8_lossy(&failed.stderr).contains("syntax-error"));
    assert!(String::from_utf8_lossy(&failed.stdout).contains("metalayer revision: 3"));
    let mut layer = Metalayer::open(database).unwrap();
    let view = layer.view().unwrap().unwrap();
    assert_eq!(view.revision().id, 3);
    assert!(!view.revision().input_complete);
    assert_eq!(view.files(None).unwrap()[0].path, "module.py");
}

fn revision_of(database: &Path) -> i64 {
    Metalayer::open(database)
        .unwrap()
        .view()
        .unwrap()
        .unwrap()
        .revision()
        .id
}

/// The count of supports that `path` has for a target that contains `part`.
fn supports(layer: &mut Metalayer, path: &str, part: &str) -> Option<u32> {
    let view = layer.view().unwrap().unwrap();
    let query = RelationQuery {
        path: Some(path.to_owned()),
        ..Default::default()
    };
    view.file_references(query)
        .unwrap()
        .into_iter()
        .find(|r| r.target.symbol.contains(part))
        .map(|r| r.supports)
}

fn publish(layer: &mut Metalayer, snapshot: &AnalysisSnapshot) -> i64 {
    layer
        .publish(snapshot, &PublishOptions::default())
        .unwrap()
        .revision
}

#[test]
#[ignore = "slow: runs in opt:slow"]
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
#[ignore = "slow: runs in opt:slow"]
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
    assert_eq!(publish(&mut layer, &initial), 1);
    assert_eq!(supports(&mut layer, "use.py", "A#value"), Some(2));

    let (warm, unchanged) = check(1);
    assert_eq!(warm.graph_cache_misses, 0);
    assert!(warm.graph_cache_hits >= 7);
    {
        let view = layer.view().unwrap().unwrap();
        assert_eq!(view.revision().id, 1);
        // The facts of the published revision equal the facts of the warm run.
        assert_eq!(view.files(None).unwrap().len(), unchanged.documents.len());
        assert_eq!(
            view.references(Default::default()).unwrap().len(),
            unchanged.references.len()
        );
    }

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
    assert_eq!(publish(&mut layer, &next), 2);
    assert_eq!(supports(&mut layer, "use.py", "A#value"), None);

    write(
        &root,
        "use.py",
        "from lib import make, missing\nitem = make()\nprint(item.value)\nmissing()\n",
    );
    let (_, one) = check(3);
    publish(&mut layer, &one);
    assert_eq!(supports(&mut layer, "use.py", "B#value"), Some(1));

    write(
        &root,
        "use.py",
        "from lib import make, missing\nitem = make()\nmissing()\n",
    );
    write(&root, "target.md", "# Renamed\n");
    fs::remove_file(root.join("b.py")).unwrap();
    let (_, removed) = check(4);
    publish(&mut layer, &removed);
    assert_eq!(supports(&mut layer, "use.py", "B#value"), None);
    assert_eq!(layer.view().unwrap().unwrap().revision().id, 4);
    drop(layer);
    assert_eq!(revision_of(&database), 4);
    let db = rusqlite::Connection::open(&database).unwrap();
    let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count("SELECT count(*) FROM pragma_foreign_key_check"), 0);
    assert_eq!(
        count("SELECT count(*) FROM file"),
        removed.documents.len() as i64
    );
    assert_eq!(
        count("SELECT count(*) FROM reference"),
        removed.references.len() as i64
    );
    assert_eq!(
        count("SELECT count(*) FROM occurrence"),
        removed
            .documents
            .iter()
            .map(|d| d.occurrences.len())
            .sum::<usize>() as i64
    );
}

#[test]
fn another_project_root_is_refused_until_the_flag_replaces_it() {
    let scratch = tempfile::tempdir().unwrap();
    let first = scratch.path().join("first");
    let second = scratch.path().join("second");
    write(&first, "a.py", "x = 1\n");
    write(&second, "b.py", "y = 2\n");
    let database = scratch.path().join("layer.sqlite");
    let output = scratch.path().join("index.scip");
    let index = |root: &Path, replace: bool| {
        let mut command = common::scippy();
        command
            .arg("index")
            .arg(root)
            .arg("-o")
            .arg(&output)
            .arg("--metalayer")
            .arg(&database);
        if replace {
            command.arg("--replace-root-set");
        }
        command.output().unwrap()
    };
    assert!(index(&first, false).status.success());
    let scip = fs::read(&output).unwrap();
    let refused = index(&second, false);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("root set"));
    assert_eq!(revision_of(&database), 1);
    assert_eq!(fs::read(&output).unwrap(), scip);
    let replaced = index(&second, true);
    assert!(
        replaced.status.success(),
        "{}",
        String::from_utf8_lossy(&replaced.stderr)
    );
    assert_eq!(revision_of(&database), 2);
}

fn scippy_index(root: &Path, flags: &[(&str, &Path)]) -> std::process::Output {
    let mut command = common::scippy();
    command.arg("index").arg(root);
    for (flag, path) in flags {
        command.arg(flag).arg(path);
    }
    command.output().unwrap()
}

#[test]
fn output_and_metalayer_on_one_path_fail_and_keep_the_database() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "module.py", "x = 1\n");
    let database = scratch.path().join("layer.sqlite");
    let first = scippy_index(
        &root,
        &[
            ("-o", &scratch.path().join("a.scip")),
            ("--metalayer", &database),
        ],
    );
    assert!(first.status.success());
    let before = fs::read(&database).unwrap();
    let clash = scippy_index(&root, &[("-o", &database), ("--metalayer", &database)]);
    assert!(!clash.status.success());
    assert!(String::from_utf8_lossy(&clash.stderr).contains("name the same file"));
    assert_eq!(fs::read(&database).unwrap(), before);
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn output_and_graph_cache_on_one_path_fail() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "module.py", "x = 1\n");
    let path = scratch.path().join("shared.sqlite");
    let clash = scippy_index(&root, &[("-o", &path), ("--graph-cache", &path)]);
    assert!(!clash.status.success());
    assert!(!path.exists());
}

#[test]
fn a_path_that_reaches_the_same_file_through_dots_fails() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "module.py", "x = 1\n");
    fs::create_dir(scratch.path().join("sub")).unwrap();
    let plain = scratch.path().join("layer.sqlite");
    let dotted = scratch.path().join("sub/../layer.sqlite");
    let clash = scippy_index(&root, &[("-o", &dotted), ("--metalayer", &plain)]);
    assert!(!clash.status.success());
}

#[test]
fn a_trace_on_a_sqlite_sidecar_path_fails() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "module.py", "x = 1\n");
    let database = scratch.path().join("layer.sqlite");
    let wal = scratch.path().join("layer.sqlite-wal");
    let clash = scippy_index(
        &root,
        &[
            ("-o", &scratch.path().join("a.scip")),
            ("--metalayer", &database),
            ("--trace", &wal),
        ],
    );
    assert!(!clash.status.success());
    assert!(String::from_utf8_lossy(&clash.stderr).contains("SQLite"));
}

#[test]
fn a_failed_publish_keeps_the_old_output_file() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "module.py", "x = 1\n");
    let output = scratch.path().join("index.scip");
    fs::write(&output, b"old index").unwrap();
    let missing = scratch.path().join("missing/layer.sqlite");
    let failed = scippy_index(&root, &[("-o", &output), ("--metalayer", &missing)]);
    assert!(!failed.status.success());
    assert_eq!(fs::read(&output).unwrap(), b"old index");
}

fn write_files(root: &Path, count: usize) {
    for n in 0..count {
        write(
            root,
            &format!("m{n}.py"),
            &format!("def f{n}():\n    return {n}\n\nf{n}()\n"),
        );
    }
}

fn run_cli(root: &Path, output: &Path, cache: Option<&Path>) -> std::process::Output {
    let mut command = common::scippy();
    command.args(["index", root.to_str().unwrap(), "--output"]);
    command.arg(output);
    if let Some(cache) = cache {
        command.arg("--graph-cache").arg(cache);
    }
    command.output().unwrap()
}

fn warnings(output: &std::process::Output) -> usize {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| line.contains("graph cache"))
        .count()
}

/// Run with a broken cache. The run must exit 0, warn once, and match a run with no cache.
fn check_broken_cache(damage: impl Fn(&Path)) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    fs::create_dir(&root).unwrap();
    write_files(&root, 40);
    let cache = scratch.path().join("graphs.sqlite");
    let clean = scratch.path().join("clean.scip");
    assert!(run_cli(&root, &clean, None).status.success());
    let warm = run_cli(&root, &scratch.path().join("warm.scip"), Some(&cache));
    assert!(warm.status.success());
    assert_eq!(warnings(&warm), 0);
    damage(&cache);
    let broken = scratch.path().join("broken.scip");
    let out = run_cli(&root, &broken, Some(&cache));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        warnings(&out),
        1,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(clean).unwrap(), fs::read(broken).unwrap());
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn a_truncated_cache_gives_one_warning_and_the_clean_output() {
    check_broken_cache(|cache| {
        let size = fs::metadata(cache).unwrap().len();
        fs::OpenOptions::new()
            .write(true)
            .open(cache)
            .unwrap()
            .set_len(size / 2)
            .unwrap();
    });
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn a_garbage_cache_moves_aside_and_gives_the_clean_output() {
    check_broken_cache(|cache| fs::write(cache, vec![0x5a; 8192]).unwrap());
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn a_directory_as_the_cache_gives_one_warning_and_the_clean_output() {
    check_broken_cache(|cache| {
        fs::remove_file(cache).unwrap();
        fs::create_dir(cache).unwrap();
    });
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn swapped_cache_rows_are_rebuilt_and_give_the_clean_output() {
    for swap_checksums in [false, true] {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("project");
        fs::create_dir(&root).unwrap();
        write_files(&root, 6);
        let cache = scratch.path().join("graphs.sqlite");
        let clean = scratch.path().join("clean.scip");
        assert!(run_cli(&root, &clean, None).status.success());
        assert!(
            run_cli(&root, &scratch.path().join("warm.scip"), Some(&cache))
                .status
                .success()
        );
        let db = rusqlite::Connection::open(&cache).unwrap();
        let rows: Vec<(String, Vec<u8>, String)> = db
            .prepare("SELECT key, value, sum FROM file_graph_v4 ORDER BY key LIMIT 2")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows.len(), 2);
        for (from, to) in [(0, 1), (1, 0)] {
            let sum = &rows[if swap_checksums { from } else { to }].2;
            db.execute(
                "UPDATE file_graph_v4 SET value = ?1, sum = ?2 WHERE key = ?3",
                rusqlite::params![rows[from].1, sum, rows[to].0],
            )
            .unwrap();
        }
        drop(db);
        let swapped = scratch.path().join("swapped.scip");
        let out = run_cli(&root, &swapped, Some(&cache));
        assert!(out.status.success());
        assert_eq!(warnings(&out), 0);
        assert!(String::from_utf8_lossy(&out.stdout).contains("file graphs: 4 cached, 2 built"));
        assert_eq!(fs::read(clean).unwrap(), fs::read(swapped).unwrap());
    }
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn a_locked_cache_turns_the_cache_off_after_one_timeout() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    fs::create_dir(&root).unwrap();
    write_files(&root, 8);
    let cache = scratch.path().join("graphs.sqlite");
    let (_, clean) = run(&root, scratch.path().join("clean.scip"), None);
    run(&root, scratch.path().join("warm.scip"), Some(cache.clone()));
    let lock = rusqlite::Connection::open(&cache).unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let mut request = IndexRequest::new(&root, scratch.path().join("locked.scip"));
    request.graph_cache = Some(cache.clone());
    request.graph_cache_timeout = std::time::Duration::from_secs(1);
    let (report, locked) =
        index_project_with_snapshot(&request, &[&PythonPack, &TypeScriptPack, &LociPack]).unwrap();
    // Each wait for the lock ends in one failure. One failure means one wait, so the later
    // files did not wait. Eight waits would mean that the cache stayed on. A wall-clock bound
    // fails under machine load.
    assert_eq!(report.graph_cache_failures, 1);
    assert_eq!(report.graph_cache_hits + report.graph_cache_misses, 0);
    assert_eq!(locked, clean);
    assert_eq!(
        fs::read(scratch.path().join("clean.scip")).unwrap(),
        fs::read(scratch.path().join("locked.scip")).unwrap()
    );
}
