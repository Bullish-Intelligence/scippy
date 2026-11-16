//! Exercise the size and the reuse of the graph cache in full Scippy runs.

mod common;

use std::fs;
use std::path::Path;
use std::process::Output;

fn write_files(root: &Path, count: usize) {
    fs::create_dir_all(root).unwrap();
    for n in 0..count {
        let text =
            format!("def f{n}():\n    return {n}\n\nclass C{n}:\n    x = f{n}()\n\nf{n}()\n");
        fs::write(root.join(format!("m{n}.py")), text).unwrap();
    }
}

fn run(root: &Path, output: &Path, cache: Option<&Path>, max_bytes: Option<&str>) -> Output {
    let mut command = common::scippy();
    command.args(["index", root.to_str().unwrap(), "--output"]);
    command.arg(output);
    if let Some(cache) = cache {
        command.arg("--graph-cache").arg(cache);
    }
    if let Some(max_bytes) = max_bytes {
        command.args(["--graph-cache-max-bytes", max_bytes]);
    }
    let out = command.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("warning"));
    out
}

fn graphs(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("file graphs: "))
        .unwrap()
        .to_owned()
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn a_warm_run_equals_a_clean_run() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write_files(&root, 30);
    let cache = scratch.path().join("graphs.sqlite");
    let clean = scratch.path().join("clean.scip");
    run(&root, &clean, None, None);
    let cold = run(&root, &scratch.path().join("cold.scip"), Some(&cache), None);
    assert!(graphs(&cold).ends_with("30 built"), "{}", graphs(&cold));
    let warm = run(&root, &scratch.path().join("warm.scip"), Some(&cache), None);
    assert!(
        graphs(&warm).starts_with("30 cached, 0 built"),
        "{}",
        graphs(&warm)
    );
    assert_eq!(
        fs::read(&clean).unwrap(),
        fs::read(scratch.path().join("cold.scip")).unwrap()
    );
    assert_eq!(
        fs::read(&clean).unwrap(),
        fs::read(scratch.path().join("warm.scip")).unwrap()
    );
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn the_cap_keeps_the_file_small_and_the_output_the_same() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write_files(&root, 300);
    let cache = scratch.path().join("graphs.sqlite");
    let clean = scratch.path().join("clean.scip");
    run(&root, &clean, None, None);
    run(&root, &scratch.path().join("big.scip"), Some(&cache), None);
    let uncapped = fs::metadata(&cache).unwrap().len();
    let cap = uncapped / 4;
    let capped = run(
        &root,
        &scratch.path().join("capped.scip"),
        Some(&cache),
        Some(&cap.to_string()),
    );
    // The run reads the whole cache. The cap applies after the run.
    assert!(
        graphs(&capped).starts_with("300 cached"),
        "{}",
        graphs(&capped)
    );
    let size = fs::metadata(&cache).unwrap().len();
    assert!(size <= cap, "the cache has {size} bytes for the cap {cap}");
    let after = run(
        &root,
        &scratch.path().join("after.scip"),
        Some(&cache),
        Some(&cap.to_string()),
    );
    assert!(
        !graphs(&after).starts_with("300 cached"),
        "{}",
        graphs(&after)
    );
    for name in ["big", "capped", "after"] {
        assert_eq!(
            fs::read(&clean).unwrap(),
            fs::read(scratch.path().join(format!("{name}.scip"))).unwrap(),
            "{name}"
        );
    }
    assert!(fs::metadata(&cache).unwrap().len() <= cap);
}

fn rows(cache: &Path) -> (i64, i64) {
    let db = rusqlite::Connection::open(cache).unwrap();
    let count = |table: &str| {
        db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    (count("file_graph_v4"), count("file_graph_use_v4"))
}

fn same_as_clean(root: &Path, scratch: &Path, cache: &Path, name: &str) -> Output {
    let clean = scratch.join(format!("{name}-clean.scip"));
    run(root, &clean, None, None);
    let out = run(
        root,
        &scratch.join(format!("{name}.scip")),
        Some(cache),
        None,
    );
    assert_eq!(
        fs::read(&clean).unwrap(),
        fs::read(scratch.join(format!("{name}.scip"))).unwrap(),
        "{name}"
    );
    out
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn two_edits_of_one_file_leave_one_row_for_it() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write_files(&root, 5);
    let cache = scratch.path().join("graphs.sqlite");
    same_as_clean(&root, scratch.path(), &cache, "cold");
    assert_eq!(rows(&cache), (5, 5));
    for edit in 1..=2 {
        fs::write(
            root.join("m0.py"),
            format!("def g{edit}():\n    return {edit}\n"),
        )
        .unwrap();
        let out = same_as_clean(&root, scratch.path(), &cache, &format!("edit{edit}"));
        assert!(
            graphs(&out).starts_with("4 cached, 1 built"),
            "{}",
            graphs(&out)
        );
        assert_eq!(rows(&cache), (5, 5));
    }
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn projects_with_the_same_relative_path_keep_their_rows() {
    let scratch = tempfile::tempdir().unwrap();
    let cache = scratch.path().join("graphs.sqlite");
    let mut roots = Vec::new();
    for (name, salt) in [("one", 1), ("two", 2)] {
        let root = scratch.path().join(name);
        fs::create_dir_all(&root).unwrap();
        for n in 0..3 {
            let value = n + salt * 100;
            let text = format!("def f{n}():\n    return {value}\n\nf{n}()\n");
            fs::write(root.join(format!("m{n}.py")), text).unwrap();
        }
        same_as_clean(&root, scratch.path(), &cache, name);
        roots.push(root);
    }
    assert_eq!(rows(&cache), (6, 6));
    // An edit in the first project replaces only its own row.
    fs::write(roots[0].join("m0.py"), "def edited():\n    return 0\n").unwrap();
    same_as_clean(&roots[0], scratch.path(), &cache, "edited");
    assert_eq!(rows(&cache), (6, 6));
    let out = same_as_clean(&roots[1], scratch.path(), &cache, "again");
    assert!(
        graphs(&out).starts_with("3 cached, 0 built"),
        "{}",
        graphs(&out)
    );
}
