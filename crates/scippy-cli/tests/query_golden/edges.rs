//! Golden cases for `scippy metalayer edges`.

use std::fs;

use crate::harness::{golden, golden_db, golden_text, project_root, scratch_db};

#[test]
fn default() {
    golden("default", &["edges"]);
}

#[test]
fn human() {
    golden_text("human", &["edges"]);
}

#[test]
fn loci_links_and_embeds() {
    golden(
        "link-embed",
        &["edges", "--kind", "link", "--kind", "embed"],
    );
}

#[test]
fn links_in_one_file() {
    golden(
        "from-index",
        &["edges", "--from", "notes/index.md", "--kind", "link"],
    );
}

#[test]
fn missing_links() {
    golden("state-missing", &["edges", "--state", "missing"]);
}

#[test]
fn ambiguous_links_list_their_candidates() {
    golden(
        "state-ambiguous",
        &["edges", "--kind", "link", "--state", "ambiguous"],
    );
}

#[test]
fn inheritance() {
    golden("kind-inheritance", &["edges", "--kind", "inheritance"]);
}

#[test]
fn imports() {
    golden("kind-import", &["edges", "--kind", "import"]);
}

#[test]
fn limit_keeps_the_first_items() {
    golden("limit-2", &["edges", "--limit", "2"]);
}

/// A small vault with links to a heading, a block, and an own heading. The shared fixture has
/// no resolved anchor, and a new note there would change the cases of the other commands.
#[test]
fn links_with_a_resolved_anchor_name_it() {
    let db = scratch_db("edges-anchors");
    let dir = db.with_file_name("edges-anchors-vault");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_file(&db);
    fs::create_dir_all(dir.join("notes")).unwrap();
    fs::create_dir_all(dir.join(".loci")).unwrap();
    for entry in fs::read_dir(project_root().join(".loci")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), dir.join(".loci").join(entry.file_name())).unwrap();
    }
    fs::copy(
        project_root().join("pyproject.toml"),
        dir.join("pyproject.toml"),
    )
    .unwrap();
    fs::write(
        dir.join("notes/a.md"),
        "# Top\n\n## Sub\n\nSee [[t#Sub]] and [[t#^blk]] and [[#Own]].\n\n# Own\n",
    )
    .unwrap();
    fs::write(dir.join("notes/t.md"), "# Top\n\n## Sub\n\ntext ^blk\n").unwrap();
    let indexed = crate::common::scippy()
        .arg("index")
        .arg(&dir)
        .arg("--output")
        .arg(dir.with_file_name("edges-anchors.scip"))
        .arg("--metalayer")
        .arg(&db)
        .output()
        .unwrap();
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    golden_db("anchors", &db, &["edges", "--kind", "link"], 0);
}
