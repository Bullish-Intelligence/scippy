//! Helpers that every test of the `scippy` binary shares.

use std::process::Command;

#[path = "../../../../test-support/copy_tree.rs"]
#[allow(dead_code)]
mod copy_tree;
#[allow(unused_imports)]
pub use copy_tree::copy_tree;

/// A command for the `scippy` binary that never touches the cache of the user.
///
/// The default graph cache lives in the user cache folder. This command turns the cache off with
/// `SCIPPY_GRAPH_CACHE=off`, and it also points `XDG_CACHE_HOME` and `HOME` into the Cargo
/// scratch folder. A flag on the command line still wins over the variable.
pub fn scippy() -> Command {
    let scratch = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let mut command = Command::new(env!("CARGO_BIN_EXE_scippy"));
    command
        .env("SCIPPY_GRAPH_CACHE", "off")
        .env("XDG_CACHE_HOME", scratch.join("xdg-cache"))
        .env("HOME", scratch.join("home"));
    command
}

/// The exit code of `scippy index` for a run that finished with lost input or an inexact analysis.
#[allow(dead_code)]
pub const EXIT_INCOMPLETE: i32 = 3;

/// True when `scippy index` finished: exit 0 (complete) or exit 3 (incomplete). A test that
/// indexes a fixture with lost input as setup uses this. A test of the exit code asserts it.
#[allow(dead_code)]
pub fn finished(output: &std::process::Output) -> bool {
    matches!(output.status.code(), Some(0 | 3))
}
