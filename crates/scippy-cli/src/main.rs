//! The `scippy` command.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use scippy_core::{IndexRequest, InputLimits, LanguagePack, index_project};
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
        /// Skip a file larger than this many bytes.
        #[arg(long, default_value_t = InputLimits::DEFAULT_MAX_FILE_BYTES)]
        max_file_bytes: u64,
        /// Stop to accept files when the accepted files would exceed this many bytes.
        #[arg(long, default_value_t = InputLimits::DEFAULT_MAX_TOTAL_BYTES)]
        max_total_bytes: u64,
        /// The package name of the symbols. Each language pack has a default.
        #[arg(long)]
        package_name: Option<String>,
        /// The package version of the symbols.
        #[arg(long)]
        package_version: Option<String>,
        /// Write one line for each reference location to this file. It is a debugging report.
        #[arg(long)]
        trace: Option<PathBuf>,
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
            max_file_bytes,
            max_total_bytes,
            package_name,
            package_version,
            trace,
        } => {
            let mut request = IndexRequest::new(project_root, output);
            request.limits = InputLimits {
                max_file_bytes,
                max_total_bytes,
            };
            request.package_name = package_name;
            request.package_version = package_version;
            request.trace = trace;
            match index_project(&request, &registry()) {
                Ok(report) => {
                    for diagnostic in &report.diagnostics {
                        eprintln!("{diagnostic}");
                    }
                    println!(
                        "files discovered: {}\nfiles indexed: {}\nfiles skipped: {}\ndeclarations: {}\noccurrences: {}\nreferences resolved: {}\nreferences external: {}\nreferences unresolved: {}\nreferences ambiguous: {}\nreferences bounded: {}",
                        report.files_discovered,
                        report.files_indexed,
                        report.files_skipped,
                        report.declarations,
                        report.occurrences,
                        report.references_resolved,
                        report.references_external,
                        report.references_unresolved,
                        report.references_ambiguous,
                        report.references_bounded,
                    );
                    let m = &report.multi_target;
                    println!(
                        "multi-target: raw {} -> shadow-filtered {}, pack-refused {}, nearest-binding {}, alias-collapsed {}, builtin-shadow {}, same-symbol {}, ambiguous {}, unresolved {}, bounded {}",
                        m.raw,
                        m.shadow_filtered,
                        m.pack_refused,
                        m.nearest_binding,
                        m.alias_collapsed,
                        m.builtin_shadow,
                        m.same_symbol,
                        m.ambiguous,
                        m.unresolved,
                        m.bounded
                    );
                    for (reason, n) in &report.unresolved_reasons {
                        println!("unresolved {reason}: {n}");
                    }
                    for (kind, n) in &report.unjoined_end_kinds {
                        println!("unjoined end {kind}: {n}");
                    }
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
