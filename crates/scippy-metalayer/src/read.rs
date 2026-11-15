//! Read one revision in one transaction.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use rusqlite::{OptionalExtension, Row, Transaction, params};
use scippy_core::ScippyError;

use crate::publish::sha256_hex;
use crate::records::{
    DiagnosticRecord, EdgeKind, EdgeRecord, FileRecord, FileReferenceRecord, FileStatus,
    OccurrenceRecord, OccurrenceRole, Outcome, OutcomeKind, ReferenceQuery, ReferenceRecord,
    RelationKind, RelationQuery, RevisionInfo, RootSetInfo, Source, SupportRecord, SymbolRef,
    parse_bound, parse_reference_kind,
};
use crate::schema::storage;

/// A read transaction. Every method reads the revision that [`crate::Metalayer::view`] pinned.
pub struct View<'a> {
    tx: Transaction<'a>,
    revision: RevisionInfo,
}

/// The columns of a symbol and of the file that holds it, when the symbol is local.
const SYMBOL_COLUMNS: &str = "{s}.symbol, {s}.local, {sf}.path";

fn symbol_columns(symbol: &str, file: &str) -> String {
    SYMBOL_COLUMNS.replace("{s}", symbol).replace("{sf}", file)
}

/// Read a symbol from three columns that start at `at`: the string, the local flag, and the path.
fn symbol_at(row: &Row<'_>, at: usize) -> rusqlite::Result<SymbolRef> {
    let local: bool = row.get(at + 1)?;
    Ok(SymbolRef {
        symbol: row.get(at)?,
        file: if local { row.get(at + 2)? } else { None },
    })
}

fn optional_symbol_at(row: &Row<'_>, at: usize) -> rusqlite::Result<Option<SymbolRef>> {
    let symbol: Option<String> = row.get(at)?;
    match symbol {
        None => Ok(None),
        Some(_) => symbol_at(row, at).map(Some),
    }
}

fn span(start: u32, end: u32) -> (u32, u32) {
    (start, end)
}

impl<'a> View<'a> {
    pub(crate) fn begin(tx: Transaction<'a>) -> Result<Option<Self>, ScippyError> {
        let revision = tx
            .query_row(
                "SELECT id, parent, input_digest, tool_version, input_complete, analysis_exact,
                        strict, scip_sha256, created_at
                 FROM revision ORDER BY id DESC LIMIT 1",
                [],
                |r| {
                    Ok(RevisionInfo {
                        id: r.get(0)?,
                        parent: r.get(1)?,
                        input_digest: r.get(2)?,
                        tool_version: r.get(3)?,
                        input_complete: r.get(4)?,
                        analysis_exact: r.get(5)?,
                        strict: r.get(6)?,
                        scip_sha256: r.get(7)?,
                        created_at: r.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(storage)?;
        Ok(revision.map(|revision| Self { tx, revision }))
    }

    /// The revision that this view reads.
    pub fn revision(&self) -> &RevisionInfo {
        &self.revision
    }

    pub fn root_set(&self) -> Result<RootSetInfo, ScippyError> {
        let (key, project_root) = self
            .tx
            .query_row(
                "SELECT key, project_root FROM root_set WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(storage)?;
        let mut statement = self
            .tx
            .prepare("SELECT path FROM root_member WHERE path <> '' ORDER BY path")
            .map_err(storage)?;
        let shards = statement
            .query_map([], |r| r.get(0))
            .map_err(storage)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(storage)?;
        Ok(RootSetInfo {
            key,
            project_root,
            shards,
        })
    }

    /// The files of the input manifest, sorted by path.
    pub fn files(&self, status: Option<FileStatus>) -> Result<Vec<FileRecord>, ScippyError> {
        let mut statement = self
            .tx
            .prepare(
                "SELECT path, status, reason, pack, sha256, size, stale FROM file
                 WHERE ?1 IS NULL OR status = ?1 ORDER BY path",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map([status.map(FileStatus::as_str)], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get::<_, Option<u64>>(5)?,
                    r.get::<_, bool>(6)?,
                ))
            })
            .map_err(storage)?;
        let mut files = Vec::new();
        for row in rows {
            let (path, status, reason, pack, sha256, size, stale) = row.map_err(storage)?;
            files.push(FileRecord {
                path,
                status: FileStatus::parse(&status)?,
                reason,
                pack,
                sha256,
                size,
                stale,
            });
        }
        Ok(files)
    }

    /// The occurrences of one file, sorted by span.
    pub fn occurrences(&self, path: &str) -> Result<Vec<OccurrenceRecord>, ScippyError> {
        let sql = format!(
            "SELECT o.start, o.end, o.role, {}
             FROM occurrence o
             JOIN file f ON f.id = o.file_id
             JOIN symbol s ON s.id = o.symbol_id
             LEFT JOIN file sf ON sf.id = s.file_id
             WHERE f.path = ?1
             ORDER BY o.start, o.end, s.symbol, o.role",
            symbol_columns("s", "sf")
        );
        let mut statement = self.tx.prepare(&sql).map_err(storage)?;
        let rows = statement
            .query_map([path], |r| {
                Ok((
                    span(r.get(0)?, r.get(1)?),
                    r.get::<_, String>(2)?,
                    symbol_at(r, 3)?,
                ))
            })
            .map_err(storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (span, role, symbol) = row.map_err(storage)?;
            out.push(OccurrenceRecord {
                span,
                symbol,
                role: OccurrenceRole::parse(&role)?,
            });
        }
        Ok(out)
    }

    /// The references that match the query, sorted by path and span.
    pub fn references(&self, q: ReferenceQuery) -> Result<Vec<ReferenceRecord>, ScippyError> {
        let sql = format!(
            "SELECT f.path, r.start, r.end, r.kind, r.outcome, r.reason, r.limited, r.id, {}
             FROM reference r
             JOIN file f ON f.id = r.file_id
             LEFT JOIN symbol t ON t.id = r.target_id
             LEFT JOIN file tf ON tf.id = t.file_id
             WHERE (?1 IS NULL OR f.path = ?1) AND (?2 IS NULL OR r.outcome = ?2)
             ORDER BY f.path, r.start, r.end",
            symbol_columns("t", "tf")
        );
        let mut statement = self.tx.prepare(&sql).map_err(storage)?;
        type Raw = (
            String,
            (u32, u32),
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            i64,
            Option<SymbolRef>,
        );
        let rows = statement
            .query_map(params![q.path, q.outcome.map(OutcomeKind::as_str)], |r| {
                Ok::<Raw, _>((
                    r.get(0)?,
                    span(r.get(1)?, r.get(2)?),
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    optional_symbol_at(r, 8)?,
                ))
            })
            .map_err(storage)?;
        let mut candidates = self
            .tx
            .prepare(&format!(
                "SELECT {} FROM reference_candidate c
                 JOIN symbol s ON s.id = c.symbol_id
                 LEFT JOIN file sf ON sf.id = s.file_id
                 WHERE c.reference_id = ?1 ORDER BY c.ordinal",
                symbol_columns("s", "sf")
            ))
            .map_err(storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (path, span, kind, outcome, reason, limited, id, target) = row.map_err(storage)?;
            let outcome = match OutcomeKind::parse(&outcome)? {
                OutcomeKind::Resolved => {
                    Outcome::Resolved(target.ok_or_else(|| missing("a resolved target"))?)
                }
                OutcomeKind::Ambiguous => Outcome::Ambiguous(
                    candidates
                        .query_map([id], |r| symbol_at(r, 0))
                        .map_err(storage)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(storage)?,
                ),
                OutcomeKind::Unresolved => {
                    Outcome::Unresolved(reason.ok_or_else(|| missing("a reason"))?)
                }
                OutcomeKind::Bounded => Outcome::Bounded(reason.ok_or_else(|| missing("a bound"))?),
            };
            out.push(ReferenceRecord {
                path,
                span,
                kind: kind.as_deref().map(parse_reference_kind).transpose()?,
                outcome,
                limited: limited.as_deref().map(parse_bound).transpose()?,
            });
        }
        Ok(out)
    }

    /// The edges of one kind, or of every kind, sorted by file and span.
    pub fn edges(&self, kind: Option<EdgeKind>) -> Result<Vec<EdgeRecord>, ScippyError> {
        let sql = format!(
            "SELECT e.kind, f.path, e.start, e.end, e.link_kind, e.label, e.state, e.anchor_state,
                    {}, {}, e.id
             FROM edge e
             JOIN file f ON f.id = e.file_id
             LEFT JOIN symbol s ON s.id = e.source_id
             LEFT JOIN file sf ON sf.id = s.file_id
             LEFT JOIN symbol t ON t.id = e.target_id
             LEFT JOIN file tf ON tf.id = t.file_id
             WHERE ?1 IS NULL OR e.kind = ?1
             ORDER BY f.path, e.start, e.end, e.kind, s.symbol, t.symbol, e.id",
            symbol_columns("s", "sf"),
            symbol_columns("t", "tf")
        );
        let mut statement = self.tx.prepare(&sql).map_err(storage)?;
        let rows = statement
            .query_map([kind.map(EdgeKind::as_str)], |r| {
                let start: Option<u32> = r.get(2)?;
                let end: Option<u32> = r.get(3)?;
                Ok((
                    r.get::<_, String>(0)?,
                    EdgeRecord {
                        kind: EdgeKind::Import,
                        path: r.get(1)?,
                        span: start.zip(end),
                        link_kind: r.get(4)?,
                        label: r.get(5)?,
                        state: r.get(6)?,
                        anchor_state: r.get(7)?,
                        source: optional_symbol_at(r, 8)?,
                        target: optional_symbol_at(r, 11)?,
                        candidates: Vec::new(),
                    },
                    r.get::<_, i64>(14)?,
                ))
            })
            .map_err(storage)?;
        let mut candidates = self
            .tx
            .prepare(&format!(
                "SELECT {} FROM edge e
                 JOIN reference_candidate c ON c.reference_id = e.reference_id
                 JOIN symbol s ON s.id = c.symbol_id
                 LEFT JOIN file sf ON sf.id = s.file_id
                 WHERE e.id = ?1 ORDER BY c.ordinal",
                symbol_columns("s", "sf")
            ))
            .map_err(storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (kind, mut edge, id) = row.map_err(storage)?;
            edge.kind = EdgeKind::parse(&kind)?;
            edge.candidates = candidates
                .query_map([id], |r| symbol_at(r, 0))
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            out.push(edge);
        }
        Ok(out)
    }

    /// The diagnostics of the run, in the order that the run raised them.
    pub fn diagnostics(&self) -> Result<Vec<DiagnosticRecord>, ScippyError> {
        let mut statement = self
            .tx
            .prepare(
                "SELECT severity, stage, code, message, path, start, end FROM diagnostic ORDER BY id",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map([], |r| {
                let start: Option<u32> = r.get(5)?;
                let end: Option<u32> = r.get(6)?;
                Ok(DiagnosticRecord {
                    severity: r.get(0)?,
                    stage: r.get(1)?,
                    code: r.get(2)?,
                    message: r.get(3)?,
                    path: r.get(4)?,
                    span: start.zip(end),
                })
            })
            .map_err(storage)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage)
    }

    /// The file-to-symbol relations that match the query, sorted by path, target, and kind.
    pub fn file_references(
        &self,
        q: RelationQuery,
    ) -> Result<Vec<FileReferenceRecord>, ScippyError> {
        let target = match &q.target {
            None => None,
            Some(symbol) => match self.symbol_id(symbol)? {
                Some(id) => Some(id),
                None => return Ok(Vec::new()),
            },
        };
        let sql = format!(
            "SELECT f.path, fr.kind, fr.supports, fr.limited, {}
             FROM file_reference fr
             JOIN file f ON f.id = fr.file_id
             JOIN symbol t ON t.id = fr.target_id
             LEFT JOIN file tf ON tf.id = t.file_id
             WHERE (?1 IS NULL OR f.path = ?1) AND (?2 IS NULL OR fr.target_id = ?2)
               AND (?3 IS NULL OR fr.kind = ?3)
             ORDER BY f.path, t.symbol, fr.kind",
            symbol_columns("t", "tf")
        );
        let mut statement = self.tx.prepare(&sql).map_err(storage)?;
        let rows = statement
            .query_map(
                params![q.path, target, q.kind.map(RelationKind::as_str)],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        symbol_at(r, 4)?,
                    ))
                },
            )
            .map_err(storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (path, kind, supports, limited, target) = row.map_err(storage)?;
            out.push(FileReferenceRecord {
                path,
                target,
                kind: RelationKind::parse(&kind)?,
                supports,
                limited,
            });
        }
        Ok(out)
    }

    /// The source locations in `path` that support the relation to `target`.
    pub fn supports(
        &self,
        path: &str,
        target: &SymbolRef,
    ) -> Result<Vec<SupportRecord>, ScippyError> {
        let Some(target) = self.symbol_id(target)? else {
            return Ok(Vec::new());
        };
        let mut statement = self
            .tx
            .prepare(
                "SELECT s.start, s.end, s.kind, s.limited FROM support s
                 JOIN file f ON f.id = s.file_id
                 WHERE f.path = ?1 AND s.target_id = ?2 ORDER BY s.start, s.end",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map(params![path, target], |r| {
                Ok((
                    span(r.get(0)?, r.get(1)?),
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (span, kind, limited) = row.map_err(storage)?;
            out.push(SupportRecord {
                path: path.to_owned(),
                span,
                kind: RelationKind::parse(&kind)?,
                limited: limited.as_deref().map(parse_bound).transpose()?,
            });
        }
        Ok(out)
    }

    /// Read the text of a file again, and check its SHA-256 against the manifest.
    ///
    /// A file below the project root is read from `project_root`. A file that the run read
    /// from outside the root is read from the path that the run recorded.
    pub fn source(&self, project_root: &Path, path: &str) -> Result<Source, ScippyError> {
        let row: Option<(Option<String>, Option<String>)> = self
            .tx
            .query_row(
                "SELECT real_path, sha256 FROM file WHERE path = ?1",
                [path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(storage)?;
        let Some((real_path, sha256)) = row else {
            return Err(ScippyError::Storage(format!(
                "no file {path:?} in the manifest"
            )));
        };
        let Some(sha256) = sha256 else {
            return Err(ScippyError::Storage(format!(
                "the manifest holds no hash for {path:?}"
            )));
        };
        let full: PathBuf = match real_path {
            Some(real) => PathBuf::from(real),
            None => {
                let relative = Path::new(path);
                if relative
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
                {
                    return Err(ScippyError::Storage(format!("bad file path {path:?}")));
                }
                project_root.join(relative)
            }
        };
        match fs::read(&full) {
            Ok(bytes) if sha256_hex(&bytes) == sha256 => match String::from_utf8(bytes) {
                Ok(text) => Ok(Source::Verified(text)),
                Err(_) => Ok(Source::Changed),
            },
            Ok(_) => Ok(Source::Changed),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Source::Missing),
            Err(source) => Err(ScippyError::Io { path: full, source }),
        }
    }

    /// The id of a symbol in this revision. Ids stay inside the crate.
    fn symbol_id(&self, symbol: &SymbolRef) -> Result<Option<i64>, ScippyError> {
        match &symbol.file {
            None => self
                .tx
                .query_row(
                    "SELECT id FROM symbol WHERE symbol = ?1 AND local = 0",
                    [&symbol.symbol],
                    |r| r.get(0),
                )
                .optional(),
            Some(file) => self
                .tx
                .query_row(
                    "SELECT s.id FROM symbol s JOIN file f ON f.id = s.file_id
                     WHERE s.symbol = ?1 AND s.local = 1 AND f.path = ?2",
                    params![symbol.symbol, file],
                    |r| r.get(0),
                )
                .optional(),
        }
        .map_err(storage)
    }
}

fn missing(what: &str) -> ScippyError {
    ScippyError::Storage(format!("the database lacks {what}"))
}
