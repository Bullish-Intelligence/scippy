//! Golden cases for `scippy metalayer check`.
//!
//! A case that changes files works on a copy of the fixture project with its own database.

use std::fs;
use std::path::PathBuf;

use crate::common::copy_tree;
use crate::harness::{golden, golden_db, golden_text, project_root, run, scratch_db};

/// An indexed copy of the fixture project that a test may change.
pub struct Copy {
    pub root: PathBuf,
    pub db: PathBuf,
    pub scip: PathBuf,
}

pub fn indexed_copy(name: &str) -> Copy {
    let db = scratch_db(name);
    let dir = db.with_file_name(format!("{name}-project"));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_file(&db);
    let root = dir.join("project");
    copy_tree(project_root(), &root);
    let scip = dir.join("index.scip");
    let indexed = crate::common::scippy()
        .args(["index"])
        .arg(&root)
        .arg("--output")
        .arg(&scip)
        .arg("--metalayer")
        .arg(&db)
        .output()
        .unwrap();
    assert!(
        crate::common::finished(&indexed),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    Copy { root, db, scip }
}

#[test]
fn fresh_index_exits_0() {
    golden("fresh", &["check"]);
}

#[test]
fn discover_finds_nothing_new_in_a_fresh_project() {
    // The skipped and shadowed files have a manifest row, so they are not new.
    golden("discover-fresh", &["check", "--discover"]);
}

#[test]
fn human_text() {
    golden_text("human", &["check"]);
}

#[test]
fn changed_files_exit_3() {
    let copy = indexed_copy("check-changed");
    // Another size: the check does not hash this file.
    let base = copy.root.join("pkg/base.py");
    let mut text = fs::read_to_string(&base).unwrap();
    text.push_str("# one more line\n");
    fs::write(&base, text).unwrap();
    // The same size and other content: only the hash tells.
    let right = copy.root.join("pkg/right.py");
    let mut bytes = fs::read(&right).unwrap();
    let last = bytes.iter().rposition(|b| b.is_ascii_lowercase()).unwrap();
    bytes[last] = if bytes[last] == b'z' { b'y' } else { b'z' };
    fs::write(&right, bytes).unwrap();
    golden_db("changed", &copy.db, &["check"], 3);

    let out = run(&["check", "--db", copy.db.to_str().unwrap()]);
    assert_eq!(out.status, 3, "{}", out.stderr);
    assert!(out.stdout.starts_with("stale\n"), "{}", out.stdout);
    assert!(
        out.stdout.contains("changed  pkg/base.py\n"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("changed  pkg/right.py\n"),
        "{}",
        out.stdout
    );
}

#[test]
fn missing_file_exits_3() {
    let copy = indexed_copy("check-missing");
    fs::remove_file(copy.root.join("pkg/left.py")).unwrap();
    golden_db("missing", &copy.db, &["check"], 3);
}

#[test]
fn discover_reports_a_new_file() {
    let copy = indexed_copy("check-new");
    fs::write(copy.root.join("pkg/extra.py"), "VALUE = 1\n").unwrap();
    fs::write(copy.root.join("notes/extra.md"), "# Extra\n").unwrap();
    // Without the option the check does not look for new files.
    golden_db("new-not-discovered", &copy.db, &["check"], 0);
    golden_db("new", &copy.db, &["check", "--discover"], 3);
}

#[test]
fn scip_file_is_compared_with_the_revision() {
    let copy = indexed_copy("check-scip");
    let scip = copy.scip.to_str().unwrap();
    golden_db("scip-ok", &copy.db, &["check", "--scip", scip], 0);
    let mut bytes = fs::read(&copy.scip).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    fs::write(&copy.scip, &bytes).unwrap();
    golden_db("scip-changed", &copy.db, &["check", "--scip", scip], 3);
    fs::remove_file(&copy.scip).unwrap();
    golden_db("scip-missing", &copy.db, &["check", "--scip", scip], 3);
}

#[test]
fn root_option_replaces_the_stored_root() {
    let copy = indexed_copy("check-root");
    let other = copy.root.with_file_name("moved");
    let _ = fs::remove_dir_all(&other);
    fs::rename(&copy.root, &other).unwrap();
    golden_db("root-missing", &copy.db, &["check"], 3);
    let out = run(&[
        "check",
        "--db",
        copy.db.to_str().unwrap(),
        "--root",
        other.to_str().unwrap(),
    ]);
    assert_eq!(out.status, 0, "{}{}", out.stdout, out.stderr);
}

#[test]
fn database_has_no_revision() {
    let db = scratch_db("check-no-revision");
    scippy_metalayer::Metalayer::open(&db).unwrap();
    golden_db("no-revision", &db, &["check"], 4);
}

#[test]
fn stale_flag_of_the_run_exits_3() {
    let copy = indexed_copy("check-stale");
    // The flag comes from a run that saw the file change. The file is unchanged now.
    let conn = rusqlite::Connection::open(&copy.db).unwrap();
    conn.execute("UPDATE file SET stale = 1 WHERE path = 'pkg/base.py'", [])
        .unwrap();
    drop(conn);
    golden_db("stale", &copy.db, &["check"], 3);
}

#[test]
fn an_edited_support_file_makes_the_index_stale() {
    // The pack reads `pyproject.toml`, `tsconfig.json`, and `.loci/vault.toml`. A change in
    // one of them changes the facts, so `check` must report it.
    let copy = indexed_copy("check-support");
    let db = copy.db.to_str().unwrap();
    let fresh = run(&["check", "--db", db]);
    assert_eq!(fresh.status, 0, "{}", fresh.stdout);

    let manifest = copy.root.join("pyproject.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    fs::write(&manifest, format!("{original}\n# edit\n")).unwrap();
    let out = run(&["check", "--db", db]);
    assert_eq!(out.status, 3, "{}", out.stdout);
    assert!(
        out.stdout.contains("changed  pyproject.toml\n"),
        "{}",
        out.stdout
    );
    fs::write(&manifest, original).unwrap();
    assert_eq!(run(&["check", "--db", db]).status, 0);

    // A file that did not exist at the run changes the input when it appears.
    let tsconfig = copy.root.join("tsconfig.json");
    fs::write(&tsconfig, "{}").unwrap();
    let out = run(&["check", "--db", db]);
    assert_eq!(out.status, 3, "{}", out.stdout);
    assert!(
        out.stdout.contains("changed  tsconfig.json\n"),
        "{}",
        out.stdout
    );
    fs::remove_file(&tsconfig).unwrap();

    let vault = copy.root.join(".loci/vault.toml");
    let original = fs::read_to_string(&vault).unwrap();
    fs::write(&vault, format!("{original}\n# edit\n")).unwrap();
    let out = run(&["check", "--db", db]);
    assert_eq!(out.status, 3, "{}", out.stdout);
    assert!(
        out.stdout.contains("changed  .loci/vault.toml\n"),
        "{}",
        out.stdout
    );
}
