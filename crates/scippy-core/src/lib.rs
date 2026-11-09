//! Language-neutral Scippy indexing API.
//!
//! [`index_project`] runs the whole pipeline: it walks the project, routes each file to a
//! language pack, extracts declarations, resolves references with a private binding engine,
//! reconciles the engine results with the declarations by source span, assigns symbol
//! identities, and writes a SCIP index.
//!
//! The public surface has three parts:
//! - the operation and its request and report: [`index_project`], [`IndexRequest`], [`IndexReport`];
//! - the language-pack contract: [`LanguagePack`] and the types that a pack returns;
//! - the error and diagnostic types.
//!
//! The binding engine, the parse trees, and the SCIP protobuf types stay private.

mod bindings;
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

pub use emit::SymbolKind;
pub use error::ScippyError;
pub use identity::{Descriptor, Package};
pub use language::{
    Analysis, BindingRules, Builtins, DeclKind, Dialect, ExternalAsset, ExternalIdentity,
    ExtractionSpec, Identity, LanguagePack, ProjectModel, Reach, ReferenceKind, Scope, SupportFile,
};
pub use project::{InputLimits, ProjectRelativePath};
pub use report::{Diagnostic, IndexReport, MultiTargetStats, Severity, Stage};

use std::path::PathBuf;

/// What to index and where to write the result.
#[derive(Clone, Debug)]
pub struct IndexRequest {
    pub root: PathBuf,
    pub output: PathBuf,
    /// Fail when the run raises an error diagnostic.
    pub strict: bool,
    pub limits: InputLimits,
    /// Replace the package name that the language pack chooses for a project without a manifest.
    pub package_name: Option<String>,
    /// Replace the package version that the language pack chooses.
    pub package_version: Option<String>,
    /// Write one line for each reference location to this file. It is a debugging report.
    /// The file is not part of the index.
    pub trace: Option<PathBuf>,
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
        }
    }
}

/// Index the project at `request.root` and write the SCIP file to `request.output`.
pub fn index_project(
    request: &IndexRequest,
    languages: &[&dyn LanguagePack],
) -> Result<IndexReport, ScippyError> {
    pipeline::run(request, languages)
}
