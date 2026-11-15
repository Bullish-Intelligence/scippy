//! Golden cases for `scippy metalayer files`.

use crate::harness::{golden, golden_exit, golden_text};

#[test]
fn default() {
    golden("default", &["files"]);
}

#[test]
fn human() {
    golden_text("human", &["files"]);
}

#[test]
fn status_skipped() {
    golden("status-skipped", &["files", "--status", "skipped"]);
}

#[test]
fn language_python() {
    golden("language-python", &["files", "--language", "Python"]);
}

#[test]
fn prefix_with_non_ascii_names() {
    golden("prefix-notes", &["files", "--prefix", "notes"]);
}

#[test]
fn limit_keeps_the_first_items() {
    golden("limit-2", &["files", "--limit", "2"]);
}

#[test]
fn bad_status_is_a_bad_argument() {
    golden_exit("bad-status", &["files", "--status", "lost"], 1);
}
