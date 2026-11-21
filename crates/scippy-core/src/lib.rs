//! Language-neutral Scippy indexing API.
//!
//! [`index`] runs the whole pipeline: it walks the project, routes each file to a
//! language pack, extracts declarations, resolves references with a private binding engine,
//! reconciles the engine results with the declarations by source span, assigns symbol
//! identities, and writes a SCIP index.
//!
//! The public surface has three parts:
//! - the operation and its request and report: [`index`], [`IndexRequest`], [`IndexMode`],
//!   [`IndexReport`], with the shorthands [`index_project`], [`index_project_with_snapshot`],
//!   and [`index_project_for_publish`];
//! - the language-pack contract: [`LanguagePack`] and the types that a pack returns;
//! - the error and diagnostic types.
//!
//! The binding engine, the parse trees, and the SCIP protobuf types stay private.

mod bindings;
mod cache;
mod emit;
mod error;
mod extract;
mod identity;
mod inherit;
mod language;
mod pipeline;
mod positions;
mod project;
mod reconcile;
mod report;
mod schedule;
mod snapshot;
mod source;

pub use snapshot::{
    AnalysisDiagnostic, AnalysisDocument, AnalysisEdge, AnalysisFile, AnalysisInput,
    AnalysisOccurrence, AnalysisReference, AnalysisSnapshot, AnalysisSymbol, DeclRecord,
    INCOMPLETE_CODES, OutlineKind, OutlineRecord, ReferenceOutcome, file_status,
};

pub use bindings::{Bound, ResolveLimits, prebuild_assets};
pub use emit::{SymbolKind, write_scip};
pub use error::ScippyError;
pub use identity::{Descriptor, Package};
pub use language::{
    Analysis, BindingRules, Builtins, DeclKind, Dialect, Engine, ExternalAsset, ExternalIdentity,
    ExtraFile, ExtraFiles, ExtractionSpec, Identity, IdentityFamily, LanguagePack, Manifest,
    NativeAnalysis, NativeDeclaration, NativeDocument, NativeInput, NativeLink, NativeOutput,
    NativeReference, NativeSource, NativeTarget, ProjectFile, ProjectModel, Reach, ReferenceKind,
    ReferenceRole, Scope, SupportFile, VirtualSymbol, WalkRules,
};
pub use pipeline::discover_files;
pub use project::{InputLimits, ProjectRelativePath};
pub use report::{
    BoundedCounts, Diagnostic, IndexReport, LostFile, MultiTargetStats, SearchWork, Severity, Stage,
};
#[doc(hidden)]
pub use source::ReadHook;

use std::path::PathBuf;

/// What to index and where to write the result.
#[derive(Clone, Debug)]
pub struct IndexRequest {
    pub root: PathBuf,
    pub output: PathBuf,
    /// Fail when the run lost input: the snapshot field `input_complete` is false. A failed
    /// run writes no output file.
    pub strict: bool,
    pub limits: InputLimits,
    /// Replace the package name that the language pack chooses for a project without a manifest.
    pub package_name: Option<String>,
    /// Replace the package version that the language pack chooses.
    pub package_version: Option<String>,
    /// Write one line for each reference location to this file. It is a debugging report.
    /// The file is not part of the index.
    pub trace: Option<PathBuf>,
    /// The number of shards that run at once. The output does not depend on it.
    pub jobs: usize,
    /// The shards run together only while the sum of their memory estimates stays under this
    /// many bytes. A shard over the budget runs alone.
    pub memory_budget: u64,
    /// Reuse file graphs from this disposable SQLite cache.
    pub graph_cache: Option<PathBuf>,
    /// The cache file keeps under this many bytes. After a run, the cache deletes its least
    /// recently used rows to fit. Zero means no cap.
    pub graph_cache_max_bytes: u64,
    /// How long a cache query waits for a locked cache file. A timeout turns the cache off.
    pub graph_cache_timeout: std::time::Duration,
    /// Limits for the work of the search for each reference. A bounded search has no target.
    /// They do not change the graph cache.
    pub resolve_limits: ResolveLimits,
    /// A test hook. The run calls it after it reads a project source file. See [`ReadHook`].
    #[doc(hidden)]
    pub read_hook: Option<ReadHook>,
}

impl IndexRequest {
    pub fn new(root: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            output: output.into(),
            strict: false,
            limits: InputLimits::default(),
            package_name: None,
            package_version: None,
            trace: None,
            jobs: schedule::default_jobs(),
            memory_budget: schedule::DEFAULT_MEMORY_BUDGET,
            graph_cache: None,
            graph_cache_max_bytes: cache::DEFAULT_MAX_BYTES,
            graph_cache_timeout: std::time::Duration::from_secs(30),
            resolve_limits: ResolveLimits::default(),
            read_hook: None,
        }
    }
}

/// What a run does with its results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexMode {
    /// Write `request.output`. Build no snapshot, unless the request is strict.
    Write,
    /// Write `request.output` and return the snapshot.
    WriteWithSnapshot,
    /// Return the snapshot and do not write `request.output`. The caller writes `snapshot.scip`
    /// with [`write_scip`] when it chooses.
    SnapshotOnly,
}

/// The result of [`index`].
#[derive(Debug)]
pub struct IndexOutput {
    pub report: IndexReport,
    /// The facts of the run. It is `Some` for every mode but [`IndexMode::Write`].
    pub snapshot: Option<AnalysisSnapshot>,
}

/// Index the project at `request.root` as `mode` says.
pub fn index(
    request: &IndexRequest,
    languages: &[&dyn LanguagePack],
    mode: IndexMode,
) -> Result<IndexOutput, ScippyError> {
    let (report, snapshot) = match mode {
        IndexMode::Write => (pipeline::run(request, languages)?, None),
        IndexMode::WriteWithSnapshot | IndexMode::SnapshotOnly => {
            let write_output = mode == IndexMode::WriteWithSnapshot;
            let (report, snapshot) = pipeline::run_with_snapshot(request, languages, write_output)?;
            (report, Some(snapshot))
        }
    };
    Ok(IndexOutput { report, snapshot })
}

/// Index the project at `request.root` and write the SCIP file to `request.output`.
pub fn index_project(
    request: &IndexRequest,
    languages: &[&dyn LanguagePack],
) -> Result<IndexReport, ScippyError> {
    pipeline::run(request, languages)
}

/// Index the project and return the facts used to write the SCIP file.
pub fn index_project_with_snapshot(
    request: &IndexRequest,
    languages: &[&dyn LanguagePack],
) -> Result<(IndexReport, AnalysisSnapshot), ScippyError> {
    pipeline::run_with_snapshot(request, languages, true)
}

/// Index the project like [`index_project_with_snapshot`], but do not write `request.output`.
/// The caller writes `snapshot.scip` with [`write_scip`] when it chooses.
pub fn index_project_for_publish(
    request: &IndexRequest,
    languages: &[&dyn LanguagePack],
) -> Result<(IndexReport, AnalysisSnapshot), ScippyError> {
    pipeline::run_with_snapshot(request, languages, false)
}
