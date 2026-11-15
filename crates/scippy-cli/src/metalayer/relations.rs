//! `scippy metalayer relations`. The command is not built yet.
//!
//! The lane that builds it replaces this file. It keeps `Args` and `run`, adds its options
//! to `Args`, and removes `hide = true`. No other file in `src/metalayer` changes.

use std::process::ExitCode;

use clap::Args as ClapArgs;

use super::common::{CliError, Common, execute};

/// List the aggregated file-to-symbol relations.
#[derive(ClapArgs, Debug)]
#[command(hide = true)]
pub struct Args {
    #[command(flatten)]
    common: Common,
}

pub fn run(args: Args) -> ExitCode {
    execute("relations", &args.common, |_view| {
        Err(CliError::bad_argument(
            "the command `relations` is not built yet",
        ))
    })
}
