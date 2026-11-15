//! The `scippy metalayer` command group.
//!
//! One module holds one command. A command module has an `Args` struct (`clap::Args`, with
//! `#[command(flatten)] common: Common`) and `fn run(Args) -> ExitCode`. It calls
//! [`common::execute`]. Every command is registered here, so a lane edits its own module only.

mod check;
mod common;
mod edges;
mod explain;
mod files;
mod relations;
mod show;

use std::process::ExitCode;

use clap::{Args, Subcommand};

/// Read a metalayer database.
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
}

pub fn run(args: MetalayerArgs) -> ExitCode {
    match args.command {
        MetalayerCommand::Show(args) => show::run(args),
        MetalayerCommand::Files(args) => files::run(args),
        MetalayerCommand::Relations(args) => relations::run(args),
        MetalayerCommand::Edges(args) => edges::run(args),
        MetalayerCommand::Explain(args) => explain::run(args),
        MetalayerCommand::Check(args) => check::run(args),
    }
}
