//! Golden cases for `scippy index --json`.

use std::fs;

use serde_json::Value;

use crate::harness::{golden_index, project_root, run_index, scratch_db, scratch_dir};

#[test]
fn incomplete_run() {
    // The fixture holds a note that is not UTF-8, so the run lost input.
    golden_index("incomplete", project_root(), &[], 3);
}

#[test]
fn complete_run() {
    let root = scratch_dir("index-complete-project");
    fs::write(root.join("good.py"), "def f():\n    return 1\n\nf()\n").unwrap();
    golden_index("complete", &root, &[], 0);
}

#[test]
fn published_run() {
    let db = scratch_db("index-published");
    golden_index(
        "published",
        project_root(),
        &["--metalayer", db.to_str().unwrap()],
        3,
    );
}

#[test]
fn strict_run_with_lost_input() {
    golden_index("strict", project_root(), &["--strict"], 1);
}

#[test]
fn json_run_reports_the_same_flags_as_the_database() {
    let db = scratch_db("index-flags");
    let out = run_index(
        project_root(),
        "index-flags",
        &["--json", "--metalayer", db.to_str().unwrap()],
    );
    assert_eq!(out.status, 3, "{}", out.stderr);
    let envelope: Value = serde_json::from_str(&out.stdout).unwrap();
    let shown = crate::harness::run(&["show", "--json", "--db", db.to_str().unwrap()]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(envelope["input_complete"], shown["input_complete"]);
    assert_eq!(envelope["analysis_exact"], shown["analysis_exact"]);
    assert_eq!(
        envelope["value"]["metalayer"]["revision"],
        shown["revision"]["id"]
    );
}

/// The `search_work` object of the `--json` envelope of one run.
fn search_work(project: &std::path::Path, name: &str, args: &[&str]) -> Value {
    let out = run_index(project, name, &[&["--json"], args].concat());
    // A run with a bounded reference ends with code 3: its analysis is not exact.
    assert!([0, 3].contains(&out.status), "{}", out.stderr);
    let envelope: Value = serde_json::from_str(&out.stdout).unwrap();
    envelope["value"]["references"]["search_work"].clone()
}

fn count(work: &Value, key: &str) -> u64 {
    work[key]
        .as_u64()
        .unwrap_or_else(|| panic!("{key} is not a count: {work}"))
}

const WORK_KEYS: [&str; 5] = [
    "searches",
    "work_total",
    "work_max",
    "queue_length_max",
    "arena_cells_max",
];

#[test]
fn search_work_has_five_counts_and_they_are_consistent() {
    let root = scratch_dir("index-search-work-project");
    fs::write(
        root.join("a.py"),
        "def f():\n    return 1\n\n\ndef g():\n    return f()\n\n\ng()\n",
    )
    .unwrap();
    let work = search_work(&root, "index-search-work", &[]);
    let mut keys: Vec<&str> = work
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected = WORK_KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert!(count(&work, "searches") > 0);
    assert!(count(&work, "work_total") > 0);
    assert!(count(&work, "work_max") > 0);
    assert!(count(&work, "work_max") <= count(&work, "work_total"));
    // One search for each reference node, so the total is at least the maximum of one search.
    assert!(count(&work, "work_total") >= count(&work, "searches"));
}

#[test]
fn search_work_is_the_same_for_each_run_and_each_number_of_jobs() {
    let root = scratch_dir("index-search-work-jobs-project");
    for (name, body) in [("a", "def f():\n    return 1\n"), ("b", "x = 1\ny = x\n")] {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("m.py"), body).unwrap();
        fs::write(
            dir.join("pyproject.toml"),
            format!("[project]\nname = \"{name}\"\n"),
        )
        .unwrap();
    }
    let one = search_work(&root, "index-search-work-j1", &["--jobs", "1"]);
    let again = search_work(&root, "index-search-work-j1b", &["--jobs", "1"]);
    let four = search_work(&root, "index-search-work-j4", &["--jobs", "4"]);
    assert!(count(&one, "searches") > 0);
    assert_eq!(one, again);
    assert_eq!(one, four);
}

#[test]
fn search_work_is_zero_when_the_project_has_no_reference() {
    let root = scratch_dir("index-search-work-empty-project");
    fs::write(root.join("a.py"), "x = 1\n").unwrap();
    let work = search_work(&root, "index-search-work-empty", &[]);
    for key in WORK_KEYS {
        assert_eq!(count(&work, key), 0, "{key}");
    }
}

#[test]
fn a_work_limit_caps_the_work_of_one_search() {
    let root = scratch_dir("index-search-work-limit-project");
    fs::write(
        root.join("a.py"),
        "def f():\n    return 1\n\n\ndef g():\n    return f()\n\n\ng()\n",
    )
    .unwrap();
    let free = search_work(&root, "index-search-work-free", &[]);
    let capped = search_work(&root, "index-search-work-capped", &["--max-work", "2"]);
    assert_eq!(count(&free, "searches"), count(&capped, "searches"));
    // The limit stops a search soon after it is reached, so the counts are smaller.
    assert!(
        count(&capped, "work_max") < count(&free, "work_max"),
        "{free} {capped}"
    );
    assert!(count(&capped, "work_total") < count(&free, "work_total"));
}
