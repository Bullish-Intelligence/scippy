//! Structured analysis and an experimental SQLite read model.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::bindings::Bound;
use crate::emit::{self, IndexData, Role, SymbolInfo};
use crate::error::ScippyError;
use crate::reconcile::{BindingOutcome, Reconciled, UnresolvedReason};
use crate::report::IndexReport;

/// A document and its indexed occurrences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisDocument {
    pub path: String,
    pub language: String,
    pub source: String,
    pub occurrences: Vec<AnalysisOccurrence>,
    pub symbols: Vec<AnalysisSymbol>,
}

/// An indexed occurrence. Positions are byte offsets in the document source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisOccurrence {
    pub start: u32,
    pub end: u32,
    pub symbol: String,
    pub role: String,
}

/// Information about a declared symbol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisSymbol {
    pub symbol: String,
    pub kind: String,
    pub display_name: String,
    pub implements: Vec<String>,
    pub references: Vec<String>,
}

fn symbol_info(s: &SymbolInfo) -> AnalysisSymbol {
    AnalysisSymbol {
        symbol: s.symbol.as_str().to_owned(),
        kind: format!("{:?}", s.kind),
        display_name: s.display_name.clone(),
        implements: s.implements.iter().map(|s| s.as_str().to_owned()).collect(),
        references: s.references.iter().map(|s| s.as_str().to_owned()).collect(),
    }
}

/// A diagnostic with its machine-readable code and source range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisDiagnostic {
    pub severity: String,
    pub stage: String,
    pub file: Option<String>,
    pub range: Option<(u32, u32)>,
    pub code: String,
    pub message: String,
}

/// The result of one reference search.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceOutcome {
    Resolved(String),
    Ambiguous(Vec<String>),
    Unresolved(String),
    Bounded(String),
}

/// A reference, including references that have no target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisReference {
    pub path: String,
    pub start: u32,
    pub end: u32,
    pub outcome: ReferenceOutcome,
}

impl AnalysisReference {
    pub(crate) fn from_reconciled(r: &Reconciled) -> Self {
        let outcome = match &r.outcome {
            BindingOutcome::Resolved(s) => ReferenceOutcome::Resolved(s.as_str().to_owned()),
            BindingOutcome::Ambiguous(s) => {
                ReferenceOutcome::Ambiguous(s.iter().map(|s| s.as_str().to_owned()).collect())
            }
            BindingOutcome::Unresolved(r) => ReferenceOutcome::Unresolved(
                match r {
                    UnresolvedReason::NoPath => "no-path",
                    UnresolvedReason::NoDeclaration => "no-declaration",
                    UnresolvedReason::NotAddressable => "not-addressable",
                    UnresolvedReason::NotTargetable => "not-targetable",
                }
                .to_owned(),
            ),
            BindingOutcome::Bounded(b) => ReferenceOutcome::Bounded(
                match b {
                    Bound::Work => "work",
                    Bound::QueueLength => "queue-length",
                    Bound::ArenaCells => "arena-cells",
                    Bound::Failed => "failed",
                }
                .to_owned(),
            ),
        };
        Self {
            path: r.reference.path.as_str().to_owned(),
            start: r.reference.start_byte,
            end: r.reference.end_byte,
            outcome,
        }
    }
}

/// The facts and diagnostics of one completed indexing run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisSnapshot {
    pub documents: Vec<AnalysisDocument>,
    pub references: Vec<AnalysisReference>,
    pub external_symbols: Vec<AnalysisSymbol>,
    pub diagnostics: Vec<AnalysisDiagnostic>,
    /// False when the run skipped files or raised an error.
    pub complete: bool,
    /// The SCIP index from the same run.
    pub scip: Vec<u8>,
}

impl AnalysisSnapshot {
    pub(crate) fn new(
        data: &IndexData,
        mut references: Vec<AnalysisReference>,
        report: &IndexReport,
    ) -> Result<Self, ScippyError> {
        let documents = data
            .documents
            .iter()
            .map(|d| AnalysisDocument {
                path: d.path.as_str().to_owned(),
                language: d.language.clone(),
                source: d.source.clone(),
                occurrences: d
                    .occurrences
                    .iter()
                    .map(|o| AnalysisOccurrence {
                        start: o.start_byte,
                        end: o.end_byte,
                        symbol: o.symbol.as_str().to_owned(),
                        role: match o.role {
                            Role::Reference => "reference",
                            Role::Definition => "definition",
                            Role::Import => "import",
                            Role::ReadAccess => "read",
                        }
                        .to_owned(),
                    })
                    .collect(),
                symbols: d.symbols.iter().map(symbol_info).collect(),
            })
            .collect();
        references.sort_by(|a, b| (&a.path, a.start, a.end).cmp(&(&b.path, b.start, b.end)));
        Ok(Self {
            documents,
            references,
            external_symbols: data.external_symbols.iter().map(symbol_info).collect(),
            diagnostics: report
                .diagnostics
                .iter()
                .map(|d| AnalysisDiagnostic {
                    severity: format!("{:?}", d.severity),
                    stage: format!("{:?}", d.stage),
                    file: d.file.clone(),
                    range: d.range,
                    code: d.code.to_owned(),
                    message: d.message.clone(),
                })
                .collect(),
            complete: report.files_skipped == 0
                && !report.diagnostics.iter().any(|d| {
                    d.severity == crate::Severity::Error || d.code == "support-file-error"
                }),
            scip: emit::encode(data.clone())?,
        })
    }
}

/// Marks a database file as a Scippy read model. The bytes spell "SCPY".
const APPLICATION_ID: i32 = 0x5343_5059;
/// The storage format version that this code reads and writes.
const SCHEMA_VERSION: i32 = 1;

/// A local, experimental read model. Each publication replaces the complete snapshot.
pub struct Metalayer {
    db: Connection,
}

impl Metalayer {
    /// Open or create the read model.
    ///
    /// The database has no migrations. Delete a database with another format and rebuild it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ScippyError> {
        let mut db = Connection::open(path).map_err(storage)?;
        db.busy_timeout(Duration::from_secs(30)).map_err(storage)?;
        db.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(storage)?;
        // An immediate transaction makes concurrent first opens take turns.
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let application_id: i32 = tx
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(storage)?;
        let version: i32 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        let has_tables: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table')",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if application_id == 0 && version == 0 {
            if has_tables {
                return Err(ScippyError::Storage(
                    "the metalayer database has the old unversioned format; delete it and rebuild it"
                        .into(),
                ));
            }
            tx.execute_batch(&format!(
                "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version={SCHEMA_VERSION};"
            ))
            .map_err(storage)?;
        } else if application_id != APPLICATION_ID {
            return Err(ScippyError::Storage(
                "the file is not a Scippy metalayer database".into(),
            ));
        } else if version != SCHEMA_VERSION {
            return Err(ScippyError::Storage(format!(
                "the metalayer database has format version {version}, but this build reads version {SCHEMA_VERSION}; delete it and rebuild it"
            )));
        }
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS snapshot (id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL, value BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS reference_support (path TEXT NOT NULL, start INTEGER NOT NULL, end INTEGER NOT NULL, target TEXT NOT NULL, PRIMARY KEY(path,start,end,target));
            CREATE TABLE IF NOT EXISTS file_reference (path TEXT NOT NULL, target TEXT NOT NULL, supports INTEGER NOT NULL, PRIMARY KEY(path,target));",
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(Self { db })
    }

    /// Atomically replace the snapshot and its derived file-reference relation.
    pub fn publish(&mut self, snapshot: &AnalysisSnapshot) -> Result<i64, ScippyError> {
        if !snapshot.complete {
            return Err(ScippyError::Storage(
                "cannot publish an incomplete analysis".into(),
            ));
        }
        let value =
            serde_json::to_vec(snapshot).map_err(|e| ScippyError::Storage(e.to_string()))?;
        // An immediate transaction takes the write lock before it reads the revision.
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let previous: i64 = tx
            .query_row("SELECT revision FROM snapshot WHERE id=1", [], |r| r.get(0))
            .optional()
            .map_err(storage)?
            .unwrap_or(0);
        let revision = previous
            .checked_add(1)
            .ok_or_else(|| ScippyError::Storage("revision overflow".into()))?;
        tx.execute("DELETE FROM reference_support", [])
            .map_err(storage)?;
        for reference in &snapshot.references {
            if let ReferenceOutcome::Resolved(target) = &reference.outcome {
                tx.execute(
                    "INSERT OR IGNORE INTO reference_support VALUES (?1, ?2, ?3, ?4)",
                    params![reference.path, reference.start, reference.end, target],
                )
                .map_err(storage)?;
            }
        }
        tx.execute("DELETE FROM file_reference", [])
            .map_err(storage)?;
        tx.execute("INSERT INTO file_reference SELECT path, target, count(*) FROM reference_support GROUP BY path,target", []).map_err(storage)?;
        tx.execute(
            "INSERT OR REPLACE INTO snapshot VALUES (1, ?1, ?2)",
            params![revision, value],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(revision)
    }

    /// Read the snapshot and its relationships in one database transaction.
    pub fn read(&mut self) -> Result<Option<PublishedView>, ScippyError> {
        let tx = self.db.transaction().map_err(storage)?;
        let row: Option<(i64, Vec<u8>)> = tx
            .query_row("SELECT revision,value FROM snapshot WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
            .map_err(storage)?;
        let Some((revision, bytes)) = row else {
            return Ok(None);
        };
        let snapshot =
            serde_json::from_slice(&bytes).map_err(|e| ScippyError::Storage(e.to_string()))?;
        let references = {
            let mut statement = tx
                .prepare("SELECT path,target,supports FROM file_reference ORDER BY path,target")
                .map_err(storage)?;
            let rows = statement
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(storage)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage)?
        };
        tx.commit().map_err(storage)?;
        Ok(Some(PublishedView {
            revision,
            snapshot,
            file_references: references,
        }))
    }
}

/// Facts and derived relationships from one published revision.
pub struct PublishedView {
    pub revision: i64,
    pub snapshot: AnalysisSnapshot,
    pub file_references: Vec<(String, String, usize)>,
}

fn storage(error: rusqlite::Error) -> ScippyError {
    ScippyError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(target: &str) -> AnalysisSnapshot {
        AnalysisSnapshot {
            documents: Vec::new(),
            references: vec![AnalysisReference {
                path: "a.py".into(),
                start: 0,
                end: 1,
                outcome: ReferenceOutcome::Resolved(target.into()),
            }],
            external_symbols: Vec::new(),
            diagnostics: Vec::new(),
            complete: true,
            scip: Vec::new(),
        }
    }

    #[test]
    fn a_failed_publication_keeps_the_previous_snapshot_and_relations() {
        let dir = tempfile::tempdir().unwrap();
        let mut layer = Metalayer::open(dir.path().join("layer.sqlite")).unwrap();
        let first = snapshot("first");
        layer.publish(&first).unwrap();
        layer.db.execute_batch("CREATE TRIGGER reject_publish BEFORE INSERT ON file_reference BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
        assert!(layer.publish(&snapshot("second")).is_err());
        let view = layer.read().unwrap().unwrap();
        assert_eq!(view.revision, 1);
        assert_eq!(view.snapshot, first);
        assert_eq!(
            view.file_references,
            vec![("a.py".into(), "first".into(), 1)]
        );
        let mut incomplete = snapshot("third");
        incomplete.complete = false;
        assert!(layer.publish(&incomplete).is_err());
        assert_eq!(layer.read().unwrap().unwrap().revision, 1);
    }

    #[test]
    fn eight_concurrent_publishers_all_succeed_with_distinct_revisions() {
        use std::sync::{Arc, Barrier};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layer.sqlite");
        let mut layers: Vec<_> = (0..8).map(|_| Metalayer::open(&path).unwrap()).collect();
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = layers
            .drain(..)
            .enumerate()
            .map(|(i, mut layer)| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let snapshot = snapshot(&format!("t{i}"));
                    barrier.wait();
                    layer.publish(&snapshot)
                })
            })
            .collect();
        let mut revisions: Vec<i64> = handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect();
        revisions.sort_unstable();
        assert_eq!(revisions, (1..=8).collect::<Vec<_>>());
    }

    #[test]
    fn a_reader_sees_one_revision_while_a_writer_publishes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layer.sqlite");
        let mut reader = Metalayer::open(&path).unwrap();
        let mut writer = Metalayer::open(&path).unwrap();
        let handle = std::thread::spawn(move || {
            for i in 0..100 {
                writer.publish(&snapshot(&format!("t{i}"))).unwrap();
            }
        });
        let mut last = 0;
        while last < 100 {
            let Some(view) = reader.read().unwrap() else {
                continue;
            };
            let expected = format!("t{}", view.revision - 1);
            assert_eq!(
                view.snapshot.references[0].outcome,
                ReferenceOutcome::Resolved(expected.clone())
            );
            assert_eq!(view.file_references, vec![("a.py".into(), expected, 1)]);
            last = view.revision;
        }
        handle.join().unwrap();
    }

    #[test]
    fn a_new_database_gets_the_application_id_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layer.sqlite");
        drop(Metalayer::open(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        let id: i32 = db
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .unwrap();
        let version: i32 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!((id, version), (APPLICATION_ID, SCHEMA_VERSION));
    }

    #[test]
    fn reopening_a_current_database_keeps_its_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layer.sqlite");
        Metalayer::open(&path)
            .unwrap()
            .publish(&snapshot("a"))
            .unwrap();
        let mut layer = Metalayer::open(&path).unwrap();
        assert_eq!(layer.read().unwrap().unwrap().revision, 1);
    }

    fn open_error(path: &Path) -> String {
        match Metalayer::open(path) {
            Err(ScippyError::Storage(message)) => message,
            _ => panic!("the open must fail with a storage error"),
        }
    }

    #[test]
    fn a_newer_database_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layer.sqlite");
        drop(Metalayer::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .execute_batch("PRAGMA user_version=99;")
            .unwrap();
        assert!(open_error(&path).contains("version 99"));
    }

    #[test]
    fn a_database_with_another_application_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch("PRAGMA application_id=1234; PRAGMA user_version=1;")
            .unwrap();
        assert!(open_error(&path).contains("not a Scippy"));
    }

    #[test]
    fn an_unversioned_database_with_tables_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE snapshot (id INTEGER PRIMARY KEY, revision INTEGER NOT NULL, value BLOB NOT NULL);")
            .unwrap();
        assert!(open_error(&path).contains("delete it and rebuild"));
    }
}
