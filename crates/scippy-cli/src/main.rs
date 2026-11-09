//! The `scippy` command.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use scippy_core::{IndexRequest, LanguagePack, index_project};
use scippy_python::PythonPack;
use scippy_typescript::TypeScriptPack;

#[derive(Parser)]
#[command(version, about = "Produce a SCIP index of a source tree")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index a project and write a SCIP file.
    Index {
        /// The project root directory.
        project_root: PathBuf,
        /// The SCIP file to write.
        #[arg(long, short, default_value = "index.scip")]
        output: PathBuf,
    },
    /// List the built-in language packs.
    Languages,
}

/// The static registry of built-in language packs.
fn registry() -> [&'static dyn LanguagePack; 2] {
    [&PythonPack, &TypeScriptPack]
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Languages => {
            for pack in registry() {
                println!("{}", pack.id());
            }
            ExitCode::SUCCESS
        }
        Command::Index {
            project_root,
            output,
        } => {
            let request = IndexRequest {
                root: project_root,
                output,
                strict: false,
            };
            match index_project(&request, &registry()) {
                Ok(report) => {
                    for diagnostic in &report.diagnostics {
                        eprintln!("{diagnostic}");
                    }
                    println!(
                        "files discovered: {}\nfiles indexed: {}\ndeclarations: {}\nreferences resolved: {}\nreferences unresolved: {}\nreferences ambiguous: {}",
                        report.files_discovered,
                        report.files_indexed,
                        report.declarations,
                        report.references_resolved,
                        report.references_unresolved,
                        report.references_ambiguous,
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("scippy: {error}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
