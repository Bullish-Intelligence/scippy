//! Golden cases for `scippy metalayer relations`.

use crate::harness::{golden, golden_exit, golden_text};

#[test]
fn default() {
    golden("default", &["relations"]);
}

#[test]
fn human() {
    golden_text("human", &["relations"]);
}

#[test]
fn from_a_file() {
    golden("from-child", &["relations", "--from", "pkg/child.py"]);
}

#[test]
fn to_a_symbol() {
    golden(
        "to-base",
        &["relations", "--to", "scippy python demo . pkg/base/Base#"],
    );
}

#[test]
fn by_target_name() {
    golden("target-name-pick", &["relations", "--target-name", "pick"]);
}

#[test]
fn one_kind() {
    golden("kind-import", &["relations", "--kind", "import"]);
}

#[test]
fn several_kinds() {
    golden(
        "kind-link-embed",
        &["relations", "--kind", "link", "--kind", "embed"],
    );
}

#[test]
fn external_targets() {
    golden("external", &["relations", "--external"]);
}

#[test]
fn internal_targets() {
    golden("internal", &["relations", "--internal"]);
}

#[test]
fn a_local_target_with_its_file() {
    golden(
        "local-target",
        &["relations", "--to", "local 0", "--to-file", "pkg/child.py"],
    );
}

#[test]
fn a_local_target_without_its_file_is_a_bad_argument() {
    golden_exit("local-no-file", &["relations", "--to", "local 0"], 1);
}

#[test]
fn limit_keeps_the_first_items() {
    golden("limit-2", &["relations", "--limit", "2"]);
}
