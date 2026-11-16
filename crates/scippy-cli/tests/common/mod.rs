//! Helpers that every test of the `scippy` binary shares.

use std::process::Command;

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
