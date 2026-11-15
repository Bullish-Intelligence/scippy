//! `scippy metalayer explain`. The command is not built yet.
//!
//! The lane that builds it replaces this file. It keeps `Args` and `run`, adds its options
//! to `Args`, and removes `hide = true`. No other file in `src/metalayer` changes.

use std::process::ExitCode;

use clap::Args as ClapArgs;

use super::common::{CliError, Common, execute};

/// Explain why a file depends on a symbol.
#[derive(ClapArgs, Debug)]
#[command(hide = true)]
pub struct Args {
    #[command(flatten)]
    common: Common,
}

pub fn run(args: Args) -> ExitCode {
    execute("explain", &args.common, |_view| {
        Err(CliError::bad_argument(
            "the command `explain` is not built yet",
        ))
    })
}
