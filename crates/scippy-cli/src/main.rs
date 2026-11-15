//! The `scippy` command.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use scippy_core::{
    IndexRequest, InputLimits, LanguagePack, ScippyError, index_project, index_project_for_publish,
    write_scip,
};
use scippy_loci::LociPack;
use scippy_metalayer::{Metalayer, PublishOptions};
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
    Index(Box<IndexArgs>),
    /// List the built-in language packs.
    Languages,
}

#[derive(Args)]
struct IndexArgs {
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
    /// size, such as `8G` or `512M`. A shard over the budget runs alone. The default is 50G.
    #[arg(long, value_parser = parse_size)]
    memory_budget: Option<u64>,
    /// Reuse file graphs in this SQLite database.
    #[arg(long)]
    graph_cache: Option<PathBuf>,
    /// Keep the graph cache file under this size, such as `1G` or `512M`. After a run, the cache
    /// deletes its least recently used rows to fit. Zero means no cap. The default is 1G.
    #[arg(long, value_parser = parse_size)]
    graph_cache_max_bytes: Option<u64>,
    /// Publish the analysis to this metalayer SQLite database.
    #[arg(long)]
    metalayer: Option<PathBuf>,
    /// Fail when the run lost input, with or without `--metalayer`. Lost input means a file that
    /// the run skipped or could not parse, or an error diagnostic. A failed run writes no SCIP
    /// file and opens no database. Both stay as they were.
    #[arg(long)]
    strict: bool,
    /// Replace the root set of the metalayer database when it holds another project root.
    /// Without this flag, the run fails and keeps the database.
    #[arg(long, requires = "metalayer")]
    replace_root_set: bool,
    /// Stop the search for one reference after this many units of work. The reference has no
    /// target. The default is in `ResolveLimits`.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    max_work: Option<u64>,
    /// Stop the search for one reference when this many paths wait for extension.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    max_queue_len: Option<u64>,
    /// Stop the search for one reference after it adds this many cells to the path arenas.
    /// This bounds the memory of one search. Each of `--jobs` shards runs one search at a time.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    max_arena_cells: Option<u64>,
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

/// Make a path absolute and resolve the links of its longest existing parent.
/// The file at the path may not exist yet.
fn resolve(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut tail: Vec<OsString> = Vec::new();
    let mut head = absolute.as_path();
    loop {
        if let Ok(real) = head.canonicalize() {
            return tail.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (head.parent(), head.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_owned());
                head = parent;
            }
            _ => return absolute,
        }
    }
}

/// The path with `suffix` added to its file name.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(suffix);
    path.with_file_name(name)
}

/// Refuse two output paths that name the same file. A SQLite database also owns the
/// `-wal`, `-shm`, and `-journal` files beside it.
fn check_paths(args: &IndexArgs) -> Result<(), String> {
    let mut named: Vec<(&str, PathBuf)> = vec![("--output", resolve(&args.output))];
    if let Some(path) = &args.trace {
        named.push(("--trace", resolve(path)));
    }
    if let Some(path) = &args.metalayer {
        named.push(("--metalayer", resolve(path)));
    }
    if let Some(path) = &args.graph_cache {
        named.push(("--graph-cache", resolve(path)));
    }
    for (i, (first, a)) in named.iter().enumerate() {
        for (second, b) in &named[i + 1..] {
            if a == b {
                return Err(format!(
                    "{first} and {second} name the same file: {}",
                    a.display()
                ));
            }
        }
    }
    for (database, base) in &named {
        if *database != "--metalayer" && *database != "--graph-cache" {
            continue;
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            let sidecar = with_suffix(base, suffix);
            if let Some((other, _)) = named.iter().find(|(_, path)| *path == sidecar) {
                return Err(format!(
                    "{other} names a SQLite file of the {database} database: {}",
                    sidecar.display()
                ));
            }
        }
    }
    Ok(())
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
        Command::Index(args) => {
            if let Err(message) = check_paths(&args) {
                eprintln!("scippy: {message}");
                return ExitCode::FAILURE;
            }
            let IndexArgs {
                project_root,
                output,
                max_file_bytes,
                max_total_bytes,
                package_name,
                package_version,
                trace,
                jobs,
                memory_budget,
                graph_cache,
                graph_cache_max_bytes,
                metalayer,
                strict,
                replace_root_set,
                max_work,
                max_queue_len,
                max_arena_cells,
            } = *args;
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
            request.graph_cache = graph_cache;
            if let Some(bytes) = graph_cache_max_bytes {
                request.graph_cache_max_bytes = bytes;
            }
            if let Some(jobs) = jobs {
                request.jobs = usize::try_from(jobs).unwrap_or(usize::MAX);
            }
            let to_usize = |n: u64| usize::try_from(n).unwrap_or(usize::MAX);
            if let Some(n) = max_work {
                request.resolve_limits.max_work = to_usize(n);
            }
            if let Some(n) = max_queue_len {
                request.resolve_limits.max_queue_len = to_usize(n);
            }
            if let Some(n) = max_arena_cells {
                request.resolve_limits.max_arena_cells = to_usize(n);
            }
            if let Some(bytes) = memory_budget {
                request.memory_budget = bytes;
            }
            // The CLI applies `--strict` itself, so that it can print the diagnostics first.
            let indexed = if metalayer.is_some() || strict {
                // The SCIP file follows the check and the publication. A failed run keeps the
                // old file.
                index_project_for_publish(&request, &registry()).and_then(|(report, snapshot)| {
                    // A strict run opens no database and writes no file for lost input.
                    if strict && !snapshot.input_complete {
                        for diagnostic in &report.diagnostics {
                            eprintln!("{diagnostic}");
                        }
                        return Err(ScippyError::Incomplete);
                    }
                    let revision = match metalayer {
                        Some(path) => {
                            let options = PublishOptions {
                                strict,
                                replace_root_set,
                            };
                            Some(
                                Metalayer::open(path)?
                                    .publish(&snapshot, &options)?
                                    .revision,
                            )
                        }
                        None => None,
                    };
                    write_scip(&request.output, &snapshot.scip).map_err(|e| match revision {
                        Some(revision) => ScippyError::Storage(format!(
                            "revision {revision} is committed, but the SCIP file failed: {e}"
                        )),
                        None => e,
                    })?;
                    Ok((report, revision))
                })
            } else {
                index_project(&request, &registry()).map(|report| (report, None))
            };
            match indexed {
                Ok((report, revision)) => {
                    for diagnostic in &report.diagnostics {
                        eprintln!("{diagnostic}");
                    }
                    println!(
                        "files discovered: {}\nfiles indexed: {}\nfiles skipped: {}\nfiles shadowed: {}\ndeclarations: {}\noccurrences: {}\nreferences resolved: {}\nreferences external: {}\nreferences unresolved: {}\nreferences ambiguous: {}\nreferences bounded: {}",
                        report.files_discovered,
                        report.files_indexed,
                        report.files_skipped,
                        report.files_shadowed,
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
                    let b = &report.bounded_by_kind;
                    println!(
                        "bounded by kind: work {}, queue-length {}, arena-cells {}, failed {}",
                        b.work, b.queue_length, b.arena_cells, b.failed
                    );
                    for (reason, n) in &report.unresolved_reasons {
                        println!("unresolved {reason}: {n}");
                    }
                    for (kind, n) in &report.unjoined_end_kinds {
                        println!("unjoined end {kind}: {n}");
                    }
                    if request.graph_cache.is_some() {
                        println!(
                            "file graphs: {} cached, {} built",
                            report.graph_cache_hits, report.graph_cache_misses
                        );
                    }
                    if let Some(revision) = revision {
                        println!("metalayer revision: {revision}");
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
