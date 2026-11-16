//! The publish-then-write flow of one `scippy index` run.
//!
//! The order is fixed. The run indexes the project, applies `--strict`, publishes to the
//! database, and writes the SCIP file last. A failed run keeps the old SCIP file.

use std::path::{Path, PathBuf};

use scippy_core::{
    AnalysisFile, Diagnostic, IndexReport, IndexRequest, LanguagePack, ScippyError,
    index_project_for_publish, write_scip,
};

use crate::{Metalayer, PublishOptions, RebindReport};

/// What the run does with a metalayer database.
#[derive(Clone, Debug)]
pub struct PublishTarget {
    /// The database file.
    pub database: PathBuf,
    /// The identity store. `None` publishes without it.
    pub identity: Option<PathBuf>,
    /// Replace the root set of the database when it holds another project root.
    pub replace_root_set: bool,
}

/// What the snapshot of a run adds to its report.
pub struct SnapshotFacts {
    /// The snapshot flag `input_complete`: the run lost no input.
    pub input_complete: bool,
    /// The snapshot flag `analysis_exact`: no reference search stopped at a limit.
    pub analysis_exact: bool,
    /// Every accepted file that the run did not index, with its status.
    pub lost_files: Vec<AnalysisFile>,
}

/// What a run published to the metalayer database.
pub struct Publication {
    pub database: PathBuf,
    pub revision: i64,
    /// `None` when the run used no identity store.
    pub rebind: Option<RebindReport>,
}

/// A finished run.
pub struct IndexRun {
    pub report: IndexReport,
    pub facts: SnapshotFacts,
    /// `None` when the run had no database.
    pub publication: Option<Publication>,
}

/// A failed run.
pub struct IndexFailure {
    pub error: ScippyError,
    /// The diagnostics of the run, when it failed under `--strict` for lost input. Else empty.
    pub diagnostics: Vec<Diagnostic>,
}

impl From<ScippyError> for IndexFailure {
    fn from(error: ScippyError) -> Self {
        Self {
            error,
            diagnostics: Vec::new(),
        }
    }
}

/// Index the project, publish to the database, and write `request.output`.
///
/// A strict run with lost input opens no database and writes no file.
pub fn index_and_publish(
    request: &IndexRequest,
    packs: &[&dyn LanguagePack],
    strict: bool,
    target: Option<&PublishTarget>,
) -> Result<IndexRun, IndexFailure> {
    let (report, snapshot) = index_project_for_publish(request, packs)?;
    if strict && !snapshot.input_complete {
        return Err(IndexFailure {
            error: ScippyError::Incomplete,
            diagnostics: report.diagnostics,
        });
    }
    let publication = match target {
        Some(target) => Some(publish(&snapshot, strict, target)?),
        None => None,
    };
    write_scip(&request.output, &snapshot.scip).map_err(|e| match &publication {
        Some(p) => ScippyError::Storage(format!(
            "revision {} is committed, but the SCIP file failed: {e}",
            p.revision
        )),
        None => e,
    })?;
    let facts = SnapshotFacts {
        input_complete: snapshot.input_complete,
        analysis_exact: snapshot.analysis_exact,
        lost_files: snapshot.lost_files,
    };
    Ok(IndexRun {
        report,
        facts,
        publication,
    })
}

fn publish(
    snapshot: &scippy_core::AnalysisSnapshot,
    strict: bool,
    target: &PublishTarget,
) -> Result<Publication, ScippyError> {
    let options = PublishOptions {
        strict,
        replace_root_set: target.replace_root_set,
    };
    let mut layer = Metalayer::open(&target.database)?;
    let published = match &target.identity {
        Some(store) => layer.publish_with_identity(snapshot, &options, Path::new(store))?,
        None => layer.publish(snapshot, &options)?,
    };
    Ok(Publication {
        database: target.database.clone(),
        revision: published.revision,
        rebind: published.rebind,
    })
}
