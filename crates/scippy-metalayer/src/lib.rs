//! The persistent metalayer: a typed SQLite read model of one Scippy analysis.
//!
//! [`Metalayer::publish`] replaces all facts with the facts of one run and commits one revision.
//! [`Metalayer::view`] reads one revision in one transaction. The tables are in
//! `METALAYER_SCHEMA.md`. The database is rebuildable: the SCIP file stays the canonical output.

mod identity;
mod publish;
mod query;
mod read;
mod rebind;
mod records;
mod schema;

use std::path::Path;

use rusqlite::Connection;
use scippy_core::{AnalysisSnapshot, ScippyError};

pub use scippy_core::{Bound, ReferenceKind};

/// The state of an entity binding. It differs from [`Outcome`], the outcome of a reference.
pub use identity::Outcome as BindOutcome;
pub use identity::{
    AcceptAction, AcceptResult, Annotation, AnnotationEntry, AnnotationFilter, CONTAINER_SEPARATOR,
    Entity, EntityDetail, EntityKind, Family, FixedIds, IDENTITY_VERSION, IdSource, Identity,
    IdentityError, IdentityOptions, IdentityProbe, LOCATOR_HISTORY, Locator, LocatorMatch,
    OutcomeRow, StatusEntry, SystemIds, Tier, default_identity_path, join_container,
    split_container,
};
pub use publish::{PublishOptions, Published};
pub use query::{Counts, Probe, SymbolRecord};
pub use query::{EdgeQuery, RelationQuery2, RelationRecord};
pub use query::{FileState, compare_file};
pub use read::View;
pub use rebind::{
    Item as RebindItem, MIN_BODY_TOKENS, MIN_BODY_TOKENS_ENV, ORPHAN_GC_PUBLISHES, RebindInput,
    RebindReport, child_key, entity_kind, min_body_tokens, rebind, rebind_with, symbol_key,
};
pub use records::{
    DiagnosticRecord, EdgeKind, EdgeRecord, FileRecord, FileReferenceRecord, FileStatus,
    OccurrenceRecord, OccurrenceRole, Outcome, OutcomeKind, ReferenceQuery, ReferenceRecord,
    RelationKind, RelationQuery, RevisionInfo, RootSetInfo, Source, SupportRecord, SymbolRef,
};
pub use schema::SCHEMA_VERSION;

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
        publish::publish(&mut self.db, snapshot, options, None)
    }

    /// [`Metalayer::publish`], and rebind the identity store at `identity` to the new facts.
    ///
    /// The store is created when it does not exist. The function commits the identity store
    /// first and the facts store second. When the second commit fails, the next publish keeps
    /// the ids: every entity matches by tier T1. The root-set guard of the identity store uses
    /// `options.replace_root_set`.
    pub fn publish_with_identity(
        &mut self,
        snapshot: &AnalysisSnapshot,
        options: &PublishOptions,
        identity: &Path,
    ) -> Result<Published, ScippyError> {
        self.publish_with_identity_ids(snapshot, options, identity, &SystemIds)
    }

    /// [`Metalayer::publish_with_identity`] with a chosen id source. Tests use a fixed source.
    pub fn publish_with_identity_ids(
        &mut self,
        snapshot: &AnalysisSnapshot,
        options: &PublishOptions,
        identity: &Path,
        ids: &dyn IdSource,
    ) -> Result<Published, ScippyError> {
        if self.read_only {
            return Err(ScippyError::Storage(
                "the metalayer database is open for reading".into(),
            ));
        }
        publish::publish(&mut self.db, snapshot, options, Some((identity, ids)))
    }

    /// Start one read transaction. The result is `None` when nothing is published.
    pub fn view(&mut self) -> Result<Option<View<'_>>, ScippyError> {
        let tx = self.db.transaction().map_err(schema::storage)?;
        View::begin(tx)
    }
}

#[cfg(test)]
mod identity_ops_tests;
#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod rebind_tests;
#[cfg(test)]
mod tests;
