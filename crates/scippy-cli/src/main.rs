//! The `scippy` command.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use scippy_core::{IndexRequest, InputLimits, LanguagePack, index_project};
use scippy_loci::LociPack;
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
        /// Skip a file larger than this many bytes. The default is 1 MiB. In a vault, it is the
        /// largest `note_max_bytes`.
        #[arg(long)]
        max_file_bytes: Option<u64>,
        /// Stop to accept files when the accepted files would exceed this many bytes.
        /// The default is 32 MiB. In a vault, it is 4 GiB.
        #[arg(long)]
        max_total_bytes: Option<u64>,
        /// The package name of the symbols. Each language pack has a default.
        #[arg(long)]
        package_name: Option<String>,
        /// The package version of the symbols.
        #[arg(long)]
        package_version: Option<String>,
        /// Write one line for each reference location to this file. It is a debugging report.
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Index this many shards at once. The default is the number of logical cores. The output is
        /// the same for every value.
        #[arg(long, short, value_parser = clap::value_parser!(u64).range(1..))]
        jobs: Option<u64>,
        /// Start a shard only while the memory estimates of the running shards stay under this
        /// size, such as `8G` or `512M`. A shard over the budget runs alone. The default is half
        /// of the physical memory, and at most 16G.
        #[arg(long, value_parser = parse_size)]
        memory_budget: Option<u64>,
    },
    /// List the built-in language packs.
    Languages,
}

/// Read a size such as `8G`, `512M`, `64K`, or a plain number of bytes.
fn parse_size(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let digits = text.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let unit = text[digits.len()..].to_ascii_lowercase();
    let shift = match unit.as_str() {
        "" | "b" => 0,
        "k" | "kb" | "kib" => 10,
        "m" | "mb" | "mib" => 20,
        "g" | "gb" | "gib" => 30,
        _ => return Err(format!("unknown size unit `{unit}`")),
    };
    let number: u64 = digits
        .parse()
        .map_err(|_| format!("`{text}` is not a size"))?;
    number
        .checked_mul(1 << shift)
        .ok_or_else(|| format!("`{text}` is too large"))
}

/// The static registry of built-in language packs.
fn registry() -> [&'static dyn LanguagePack; 3] {
    [&PythonPack, &TypeScriptPack, &LociPack]
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
            jobs,
            memory_budget,
        } => {
            // A pack that is active for the project can suggest limits. A flag wins over it.
            let suggested = registry()
                .iter()
                .filter(|p| p.enabled(&project_root))
                .find_map(|p| p.input_limits(&project_root))
                .unwrap_or_default();
            let mut request = IndexRequest::new(project_root, output);
            request.limits = InputLimits {
                max_file_bytes: max_file_bytes.unwrap_or(suggested.max_file_bytes),
                max_total_bytes: max_total_bytes.unwrap_or(suggested.max_total_bytes),
            };
            request.package_name = package_name;
            request.package_version = package_version;
            request.trace = trace;
            if let Some(jobs) = jobs {
                request.jobs = usize::try_from(jobs).unwrap_or(usize::MAX);
            }
            if let Some(bytes) = memory_budget {
                request.memory_budget = bytes;
            }
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
