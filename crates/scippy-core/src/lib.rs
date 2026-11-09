//! Language-neutral Scippy indexing API.
//!
//! Gate 1 holds the SCIP producer: identity checking, positions, and the emitter.
//! The project walker routes files to language packs.
//! No stage yet extracts declarations or resolves references,
//! so [`index_project`] writes documents without occurrences.

pub mod emit;
mod error;
mod extract;
pub mod identity;
mod language;
pub mod positions;
mod project;
mod report;

pub use error::ScippyError;
pub use extract::SourceAnchor;
pub use language::LanguagePack;
pub use project::ProjectRelativePath;
pub use report::{Diagnostic, IndexReport, Severity, Stage};

use std::path::PathBuf;

use emit::{Document, IndexData};

pub struct IndexRequest {
    pub root: PathBuf,
    pub output: PathBuf,
    /// Fail when the run raises an error diagnostic.
    pub strict: bool,
}

/// Index the project at `request.root` and write the SCIP file to `request.output`.
pub fn index_project(
    request: &IndexRequest,
    languages: &[&dyn LanguagePack],
) -> Result<IndexReport, ScippyError> {
    let found = project::discover(&request.root, &request.output, languages)?;

    let documents: Vec<Document> = found
        .files
        .iter()
        .map(|(path, pack)| Document {
            path: path.clone(),
            language: languages[*pack].scip_language().to_string(),
            source: String::new(),
            occurrences: Vec::new(),
            symbols: Vec::new(),
        })
        .collect();

    let report = IndexReport {
        files_discovered: found.files_discovered,
        files_indexed: documents.len(),
        diagnostics: found.diagnostics,
        ..Default::default()
    };

    if request.strict {
        let errors = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        if errors > 0 {
            return Err(ScippyError::Strict { errors });
        }
    }

    emit::write_index(
        IndexData {
            project_root: found.root,
            documents,
        },
        &request.output,
    )?;
    Ok(report)
}
