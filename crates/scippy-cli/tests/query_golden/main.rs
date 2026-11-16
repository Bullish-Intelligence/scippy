//! Golden tests for the `scippy metalayer` commands and the JSON contract.
//!
//! The design is in `METALAYER_QUERY.md`, section 8. To add a command, add a file `<command>.rs`
//! here and one `mod` line below. The file calls `harness::golden`. See `harness.rs` for the
//! helper functions, and `show.rs` for an example.
//!
//! To update the golden files, run
//! `UPDATE_GOLDEN=1 cargo test -p scippy-cli --test query_golden`.
//! A change in a golden file is a contract change. Check `METALAYER_QUERY.md`, section 5.

#[path = "../common/mod.rs"]
mod common;
mod harness;

mod annotate;
mod check;
mod compat;
mod edges;
mod entity;
mod explain;
mod files;
mod index;
mod relations;
mod show;
mod skill;
