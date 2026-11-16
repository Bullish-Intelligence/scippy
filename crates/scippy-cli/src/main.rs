//! The `scippy` command.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use scippy_core::{IndexRequest, InputLimits};
use scippy_metalayer::{
    IndexFailure, IndexRun, PublishTarget, default_identity_path, index_and_publish,
};

use index_output::RunOutput;
use registry::registry;

mod cache_choice;
mod index_output;
mod metalayer;
mod registry;

#[derive(Parser)]
#[command(
    name = "scippy",
    version,
    about = "Produce a SCIP index of a source tree"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index a project and write a SCIP file.
    ///
    /// Exit codes: 0 when the run is complete; 3 when it finished with lost input or an inexact
    /// analysis; 1 when it failed, also for `--strict` with lost input; 2 for a usage error.
    Index(Box<IndexArgs>),
    /// List the built-in language packs.
    Languages,
    /// Query a metalayer database.
    Metalayer(metalayer::MetalayerArgs),
}

#[derive(Args)]
struct IndexArgs {
    /// The project root directory.
    #[arg(value_name = "PROJECT_ROOT")]
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
    /// Reuse file graphs in this SQLite database. The default file is
    /// `$XDG_CACHE_HOME/scippy/graph-cache.sqlite`, or `$HOME/.cache/scippy/graph-cache.sqlite`.
    /// The variable `SCIPPY_GRAPH_CACHE` (a path, or `off`) replaces the default. This flag
    /// replaces both. A failure of the cache file turns the cache off with one warning, and the
    /// run goes on. The cache is disposable: delete the file at any time.
    #[arg(long, conflicts_with = "no_graph_cache")]
    graph_cache: Option<PathBuf>,
    /// Run without the graph cache. This wins over `SCIPPY_GRAPH_CACHE`.
    #[arg(long)]
    no_graph_cache: bool,
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
    /// Print one JSON envelope to the standard output stream, in place of the text summary. A
    /// failed run prints an error envelope. The diagnostics also go to the standard error
    /// stream.
    #[arg(long)]
    json: bool,
    /// Replace the root set of the metalayer database when it holds another project root.
    /// Without this flag, the run fails and keeps the database.
    #[arg(long, requires = "metalayer")]
    replace_root_set: bool,
    /// Keep entity ids in this identity store. The default is beside the metalayer database:
    /// `<name>.identity.db`. The store keeps the ids across edits, moves, and renames.
    #[arg(long, requires = "metalayer", conflicts_with = "no_identity")]
    identity: Option<PathBuf>,
    /// Publish without the identity store. The run does not rebind entities.
    #[arg(long, requires = "metalayer")]
    no_identity: bool,
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

/// The identity store of a run: the `--identity` path, or the default beside the metalayer
/// database. `None` when the run has no database or has `--no-identity`.
fn identity_path(args: &IndexArgs) -> Option<PathBuf> {
    let metalayer = args.metalayer.as_ref()?;
    if args.no_identity {
        return None;
    }
    Some(
        args.identity
            .clone()
            .unwrap_or_else(|| default_identity_path(metalayer)),
    )
}

/// Refuse two output paths that name the same file. A SQLite database also owns the
/// `-wal`, `-shm`, and `-journal` files beside it.
fn check_paths(args: &IndexArgs, cache: Option<&Path>) -> Result<(), String> {
    let mut named: Vec<(&str, PathBuf)> = vec![("--output", resolve(&args.output))];
    if let Some(path) = &args.trace {
        named.push(("--trace", resolve(path)));
    }
    if let Some(path) = &args.metalayer {
        named.push(("--metalayer", resolve(path)));
    }
    if let Some(path) = cache {
        named.push(("--graph-cache", resolve(path)));
    }
    if let Some(path) = identity_path(args) {
        named.push(("--identity", resolve(&path)));
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
        if !["--metalayer", "--graph-cache", "--identity"].contains(database) {
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

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Languages => {
            for pack in registry() {
                println!("{}", pack.id());
            }
            ExitCode::SUCCESS
        }
        Command::Metalayer(args) => metalayer::run(args),
        Command::Index(args) => {
            let choice =
                cache_choice::choose(args.graph_cache.clone(), args.no_graph_cache, &|name| {
                    std::env::var_os(name)
                });
            if let Err(message) = check_paths(&args, cache_choice::explicit(&choice)) {
                eprintln!("scippy: {message}");
                return ExitCode::FAILURE;
            }
            let identity = identity_path(&args);
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
                graph_cache: _,
                no_graph_cache: _,
                graph_cache_max_bytes,
                metalayer,
                strict,
                json,
                replace_root_set,
                identity: _,
                no_identity: _,
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
            request.graph_cache = cache_choice::prepare(choice);
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
            // The library applies `--strict` itself, so that the CLI can print the diagnostics
            // first. Every run needs the snapshot flags for its exit code.
            let target = metalayer.map(|database| PublishTarget {
                database,
                identity,
                replace_root_set,
            });
            let indexed = index_and_publish(&request, &registry(), strict, target.as_ref());
            match indexed {
                Ok(IndexRun {
                    report,
                    facts,
                    publication,
                }) => {
                    for diagnostic in &report.diagnostics {
                        eprintln!("{diagnostic}");
                    }
                    let run = RunOutput {
                        report,
                        output: request.output,
                        publication,
                    };
                    if json {
                        println!("{}", run.envelope(&facts));
                    } else {
                        print!("{}", run.text());
                    }
                    index_output::exit_code(&facts)
                }
                Err(IndexFailure { error, diagnostics }) => {
                    for diagnostic in &diagnostics {
                        eprintln!("{diagnostic}");
                    }
                    if json {
                        println!("{}", index_output::error_envelope(error));
                    } else {
                        eprintln!("scippy: {error}");
                    }
                    ExitCode::FAILURE
                }
            }
        }
    }
}
