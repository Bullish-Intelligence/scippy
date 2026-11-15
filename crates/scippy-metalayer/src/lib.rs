//! The persistent metalayer: a typed SQLite read model of one Scippy analysis.
//!
//! [`Metalayer::publish`] replaces all facts with the facts of one run and commits one revision.
//! [`Metalayer::view`] reads one revision in one transaction. The tables are in
//! `METALAYER_SCHEMA.md`. The database is rebuildable: the SCIP file stays the canonical output.

mod publish;
mod read;
mod records;
mod schema;

use std::path::Path;

use rusqlite::Connection;
use scippy_core::{AnalysisSnapshot, ScippyError};

pub use publish::{PublishOptions, Published};
pub use read::View;
pub use records::{
    DiagnosticRecord, EdgeKind, EdgeRecord, FileRecord, FileReferenceRecord, FileStatus,
    OccurrenceRecord, OccurrenceRole, Outcome, OutcomeKind, RefKind, ReferenceQuery,
    ReferenceRecord, RelationKind, RelationQuery, RevisionInfo, RootSetInfo, Source, SupportRecord,
    SymbolRef,
};

/// A local read model. Each publish replaces the facts of the previous revision.
pub struct Metalayer {
    db: Connection,
    read_only: bool,
}

impl Metalayer {
    /// Open or create the database.
    ///
    /// A database with an older format version is dropped and rebuilt. The function prints one
    /// line to the standard error stream. A database with a newer version is refused.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ScippyError> {
        Ok(Self {
            db: schema::open(path.as_ref())?,
            read_only: false,
        })
    }

    /// Open the database for reading. It never writes, creates, or rebuilds the file.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, ScippyError> {
        Ok(Self {
            db: schema::open_read_only(path.as_ref())?,
            read_only: true,
        })
    }

    /// Replace all facts with the facts of `snapshot`, and commit one revision.
    pub fn publish(
        &mut self,
        snapshot: &AnalysisSnapshot,
        options: &PublishOptions,
    ) -> Result<Published, ScippyError> {
        if self.read_only {
            return Err(ScippyError::Storage(
                "the metalayer database is open for reading".into(),
            ));
        }
        publish::publish(&mut self.db, snapshot, options)
    }

    /// Start one read transaction. The result is `None` when nothing is published.
    pub fn view(&mut self) -> Result<Option<View<'_>>, ScippyError> {
        let tx = self.db.transaction().map_err(schema::storage)?;
        View::begin(tx)
    }
}

#[cfg(test)]
mod tests;
