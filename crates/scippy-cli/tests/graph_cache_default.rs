//! Check the default graph cache of the `scippy` binary: where it lives, how to turn it off,
//! the order of the choices, and the failure of the default file.
//!
//! Each test sets `XDG_CACHE_HOME` and `HOME` to a scratch folder. No test touches the real
//! cache of the user.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DEFAULT_NAME: &str = "scippy/graph-cache.sqlite";

fn project(scratch: &Path) -> PathBuf {
    let root = scratch.join("project");
    fs::create_dir_all(&root).unwrap();
    for n in 0..2 {
        let text = format!("def f{n}():\n    return {n}\n\nf{n}()\n");
        fs::write(root.join(format!("m{n}.py")), text).unwrap();
    }
    root
}

/// A command that starts with the default choice: no variable, and a scratch cache folder.
fn command(scratch: &Path, root: &Path, output: &str) -> Command {
    let mut command = common::scippy();
    command
        .env_remove("SCIPPY_GRAPH_CACHE")
        .env("XDG_CACHE_HOME", scratch.join("xdg"))
        .env("HOME", scratch.join("home"))
        .arg("index")
        .arg(root)
        .arg("--output")
        .arg(scratch.join(output));
    command
}

fn ok(out: Output) -> Output {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn warnings(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|line| line.starts_with("warning:"))
        .count()
}

fn same(scratch: &Path, a: &str, b: &str) {
    assert_eq!(
        fs::read(scratch.join(a)).unwrap(),
        fs::read(scratch.join(b)).unwrap(),
        "{a} and {b} differ"
    );
}

#[test]
fn the_default_file_is_created_and_reused_and_the_output_is_the_same() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch = scratch.path();
    let root = project(scratch);
    let default = scratch.join("xdg").join(DEFAULT_NAME);
    assert!(!default.exists());
    ok(command(scratch, &root, "clean.scip")
        .arg("--no-graph-cache")
        .output()
        .unwrap());
    assert!(!default.exists());
    let cold = ok(command(scratch, &root, "cold.scip").output().unwrap());
    assert!(default.is_file());
    assert!(stdout(&cold).contains("file graphs: 0 cached, 2 built"));
    let warm = ok(command(scratch, &root, "warm.scip").output().unwrap());
    assert!(stdout(&warm).contains("file graphs: 2 cached, 0 built"));
    assert_eq!(warnings(&warm), 0);
    same(scratch, "clean.scip", "cold.scip");
    same(scratch, "clean.scip", "warm.scip");
}

#[test]
fn without_xdg_the_default_file_is_under_home() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch = scratch.path();
    let root = project(scratch);
    ok(command(scratch, &root, "out.scip")
        .env_remove("XDG_CACHE_HOME")
        .output()
        .unwrap());
    let file = scratch.join("home/.cache").join(DEFAULT_NAME);
    assert!(file.is_file());
    assert!(!scratch.join("xdg").exists());
}

#[test]
fn no_graph_cache_writes_nothing_and_wins_over_the_variable() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch = scratch.path();
    let root = project(scratch);
    let named = scratch.join("named.sqlite");
    let out = ok(command(scratch, &root, "out.scip")
        .env("SCIPPY_GRAPH_CACHE", &named)
        .arg("--no-graph-cache")
        .output()
        .unwrap());
    assert!(!scratch.join("xdg").exists());
    assert!(!scratch.join("home").exists());
    assert!(!named.exists());
    assert!(!stdout(&out).contains("file graphs"));
}

#[test]
fn the_flag_and_no_graph_cache_exclude_each_other() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch = scratch.path();
    let root = project(scratch);
    let out = command(scratch, &root, "out.scip")
        .arg("--no-graph-cache")
        .arg("--graph-cache")
        .arg(scratch.join("c.sqlite"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!scratch.join("out.scip").exists());
}

#[test]
fn the_flag_wins_over_the_variable_and_the_variable_over_the_default() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch = scratch.path();
    let root = project(scratch);
    let flag = scratch.join("flag.sqlite");
    let var = scratch.join("var.sqlite");
    let default = scratch.join("xdg").join(DEFAULT_NAME);

    ok(command(scratch, &root, "a.scip")
        .env("SCIPPY_GRAPH_CACHE", &var)
        .arg("--graph-cache")
        .arg(&flag)
        .output()
        .unwrap());
    assert!(flag.is_file() && !var.exists() && !default.exists());

    // The flag also wins over `off`.
    fs::remove_file(&flag).unwrap();
    ok(command(scratch, &root, "b.scip")
        .env("SCIPPY_GRAPH_CACHE", "off")
        .arg("--graph-cache")
        .arg(&flag)
        .output()
        .unwrap());
    assert!(flag.is_file() && !default.exists());

    ok(command(scratch, &root, "c.scip")
        .env("SCIPPY_GRAPH_CACHE", &var)
        .output()
        .unwrap());
    assert!(var.is_file() && !default.exists());

    let off = ok(command(scratch, &root, "d.scip")
        .env("SCIPPY_GRAPH_CACHE", "off")
        .output()
        .unwrap());
    assert!(!default.exists());
    assert!(!stdout(&off).contains("file graphs"));
}

#[test]
fn an_unusable_default_location_warns_once_and_the_index_succeeds() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch = scratch.path();
    let root = project(scratch);
    ok(command(scratch, &root, "clean.scip")
        .arg("--no-graph-cache")
        .output()
        .unwrap());

    // The cache folder cannot be created: `XDG_CACHE_HOME` holds a file.
    let blocked = scratch.join("blocked");
    fs::write(&blocked, "not a folder").unwrap();
    let out = ok(command(scratch, &root, "folder.scip")
        .env("XDG_CACHE_HOME", &blocked)
        .output()
        .unwrap());
    assert_eq!(
        warnings(&out),
        1,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!stdout(&out).contains("file graphs"));
    same(scratch, "clean.scip", "folder.scip");

    // The cache file cannot be opened: a folder has its name.
    let xdg = scratch.join("xdg");
    fs::create_dir_all(xdg.join(DEFAULT_NAME)).unwrap();
    let out = ok(command(scratch, &root, "file.scip").output().unwrap());
    assert_eq!(
        warnings(&out),
        1,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    same(scratch, "clean.scip", "file.scip");

    // The environment has no cache folder at all.
    let out = ok(command(scratch, &root, "none.scip")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("HOME")
        .output()
        .unwrap());
    assert_eq!(
        warnings(&out),
        1,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    same(scratch, "clean.scip", "none.scip");
}

#[test]
fn an_explicit_bad_file_keeps_its_behavior() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch = scratch.path();
    let root = project(scratch);
    // The explicit flag does not create folders. The core warns and the run goes on.
    let out = ok(command(scratch, &root, "out.scip")
        .arg("--graph-cache")
        .arg(scratch.join("missing/folder/c.sqlite"))
        .output()
        .unwrap());
    assert_eq!(
        warnings(&out),
        1,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!scratch.join("missing").exists());
}
