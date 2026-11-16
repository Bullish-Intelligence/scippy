//! The `scippy metalayer` command group.
//!
//! One module holds one command. A command module has an `Args` struct (`clap::Args`, with
//! `#[command(flatten)] common: Common`) and `fn run(Args) -> ExitCode`. It calls
//! [`common::execute`]. Every command is registered here. The split keeps each command, its
//! arguments, and its output in one file, and keeps the shared envelope code in [`common`].

mod annotate;
mod check;
mod common;
mod edges;
mod entity;
mod explain;
mod files;
mod relations;
mod show;

pub(crate) use common::{CliError, error_envelope, run_envelope};

use std::process::ExitCode;

use clap::{Args, Subcommand};

/// Read a metalayer database, and read and write its annotations.
#[derive(Args)]
pub struct MetalayerArgs {
    #[command(subcommand)]
    command: MetalayerCommand,
}

#[derive(Subcommand)]
enum MetalayerCommand {
    Show(show::Args),
    Files(files::Args),
    Relations(relations::Args),
    Edges(edges::Args),
    Explain(explain::Args),
    Check(check::Args),
    Entity(entity::Args),
    Annotate(annotate::Args),
}

pub fn run(args: MetalayerArgs) -> ExitCode {
    match args.command {
        MetalayerCommand::Show(args) => show::run(args),
        MetalayerCommand::Files(args) => files::run(args),
        MetalayerCommand::Relations(args) => relations::run(args),
        MetalayerCommand::Edges(args) => edges::run(args),
        MetalayerCommand::Explain(args) => explain::run(args),
        MetalayerCommand::Check(args) => check::run(args),
        MetalayerCommand::Entity(args) => entity::run(args),
        MetalayerCommand::Annotate(args) => annotate::run(args),
    }
}
