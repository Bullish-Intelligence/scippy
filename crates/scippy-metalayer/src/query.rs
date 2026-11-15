//! The read API additions for the query commands.
//!
//! The module holds the counts, the single-record lookups, and the database probe. The
//! design is in `METALAYER_QUERY.md`, section 7.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use scippy_core::ScippyError;
use sha2::{Digest, Sha256};

use crate::read::{optional_symbol_at, symbol_at, symbol_columns};
use crate::schema::{APPLICATION_ID, storage};
use crate::{
    EdgeKind, EdgeRecord, FileRecord, Metalayer, Outcome, OutcomeKind, ReferenceQuery,
    ReferenceRecord, RelationKind, SymbolRef, View,
};

/// The counts of one revision. Each map holds every known key, also with the count zero.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Counts {
    /// The files by status.
    pub files: BTreeMap<String, u64>,
    pub symbols: u64,
    /// The references by outcome.
    pub references: BTreeMap<String, u64>,
    /// The edges by kind.
    pub edges: BTreeMap<String, u64>,
    /// The diagnostics by severity.
    pub diagnostics: BTreeMap<String, u64>,
}

/// A symbol with its data.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SymbolRecord {
    pub symbol: SymbolRef,
    /// The SCIP kind name, such as `Class`. A symbol that no record declares has `Unspecified`.
    pub kind: String,
    /// The empty string when no record declares the symbol.
    pub display_name: String,
    /// No document of the project defines the symbol.
    pub external: bool,
    /// The path of the defining document. It is `None` for an external symbol.
    pub defined_in: Option<String>,
}

/// What a path is, as far as the reader can tell without opening it for use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Probe {
    /// The path does not exist.
    Missing,
    /// The file is not a Scippy metalayer database.
    NotScippy,
    /// A Scippy metalayer database with this storage format version.
    Version(i32),
}

/// The statuses, outcomes, edge kinds, and severities that the schema allows.
const FILE_STATUSES: [&str; 4] = ["indexed", "skipped", "failed", "shadowed"];
const OUTCOMES: [&str; 4] = ["resolved", "ambiguous", "unresolved", "bounded"];
const EDGE_KINDS: [&str; 6] = [
    "import",
    "inheritance",
    "override",
    "alias",
    "link",
    "embed",
];
const SEVERITIES: [&str; 3] = ["error", "warning", "info"];

impl Metalayer {
    /// Look at a path without opening it for use. It never writes, creates, or rebuilds.
    pub fn probe(path: impl AsRef<Path>) -> Result<Probe, ScippyError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Probe::Missing);
        }
        let Ok(db) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
            return Ok(Probe::NotScippy);
        };
        let number = |pragma: &str| db.query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0));
        // A file that SQLite cannot read fails here, so it is not a database of ours.
        let Ok(application_id) = number("application_id") else {
            return Ok(Probe::NotScippy);
        };
        if application_id != APPLICATION_ID {
            return Ok(Probe::NotScippy);
        }
        Ok(Probe::Version(number("user_version").map_err(storage)?))
    }
}

impl View<'_> {
    /// The storage format version of the database. It is informational.
    pub fn schema_version(&self) -> Result<i32, ScippyError> {
        self.tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)
    }

    /// The counts of the revision.
    pub fn counts(&self) -> Result<Counts, ScippyError> {
        let group = |sql: &str, known: &[&str]| -> Result<BTreeMap<String, u64>, ScippyError> {
            let mut map: BTreeMap<String, u64> =
                known.iter().map(|k| ((*k).to_owned(), 0)).collect();
            let mut statement = self.tx.prepare(sql).map_err(storage)?;
            let rows = statement
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
                .map_err(storage)?;
            for row in rows {
                let (key, count) = row.map_err(storage)?;
                map.insert(key, count);
            }
            Ok(map)
        };
        Ok(Counts {
            files: group(
                "SELECT status, COUNT(*) FROM file GROUP BY status",
                &FILE_STATUSES,
            )?,
            symbols: self
                .tx
                .query_row("SELECT COUNT(*) FROM symbol", [], |r| r.get(0))
                .map_err(storage)?,
            references: group(
                "SELECT outcome, COUNT(*) FROM reference GROUP BY outcome",
                &OUTCOMES,
            )?,
            edges: group("SELECT kind, COUNT(*) FROM edge GROUP BY kind", &EDGE_KINDS)?,
            diagnostics: group(
                "SELECT severity, COUNT(*) FROM diagnostic GROUP BY severity",
                &SEVERITIES,
            )?,
        })
    }

    /// The manifest row of one file, or `None` when the manifest has no such path.
    pub fn file(&self, path: &str) -> Result<Option<FileRecord>, ScippyError> {
        Ok(self.files_where(None, Some(path))?.into_iter().next())
    }

    /// One symbol with its data, or `None` when this revision does not know the symbol.
    pub fn symbol(&self, symbol: &SymbolRef) -> Result<Option<SymbolRecord>, ScippyError> {
        let Some(id) = self.symbol_id(symbol)? else {
            return Ok(None);
        };
        let (kind, display_name, defined_in) = self
            .tx
            .query_row(
                "SELECT s.kind, s.display_name, f.path FROM symbol s
                 LEFT JOIN file f ON f.id = s.file_id WHERE s.id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<String>>(2)?)),
            )
            .optional()
            .map_err(storage)?
            .ok_or_else(|| ScippyError::Storage("the symbol row vanished".into()))?;
        Ok(Some(SymbolRecord {
            symbol: symbol.clone(),
            kind,
            display_name,
            external: defined_in.is_none(),
            defined_in,
        }))
    }
}

// ---- The file check: is the index fresh (`check`, section 6) ----

/// What a file on disk is, compared with the size and the SHA-256 that the manifest stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileState {
    /// The file exists, and its size and hash equal the stored values.
    Ok,
    /// The file exists with another size or another hash.
    Changed,
    /// The file does not exist.
    Missing,
    /// The manifest stores no hash for the file, so nothing can be compared.
    Unchecked,
}

/// Compare the file at `path` with a stored size and a stored SHA-256.
///
/// The function compares the size first, and hashes the content only when the sizes agree.
/// It hashes in a stream and never holds the whole file. A stored size of `None` skips the
/// size test. A file that is not a regular file counts as missing.
pub fn compare_file(
    path: &Path,
    size: Option<u64>,
    sha256: &str,
) -> Result<FileState, ScippyError> {
    let io = |source| ScippyError::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(FileState::Missing),
        Err(e) => return Err(io(e)),
    };
    let meta = file.metadata().map_err(io)?;
    if !meta.is_file() {
        return Ok(FileState::Missing);
    }
    if size.is_some_and(|size| size != meta.len()) {
        return Ok(FileState::Changed);
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let digest = hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        });
    Ok(if digest == sha256 {
        FileState::Ok
    } else {
        FileState::Changed
    })
}

impl View<'_> {
    /// Compare one manifest file with the disk. A file inside the project root is read at
    /// `project_root/path`. A file outside it is read at its `real_path`. A file with no stored
    /// hash gives `Unchecked`.
    pub fn file_state(
        &self,
        project_root: &Path,
        file: &FileRecord,
    ) -> Result<FileState, ScippyError> {
        let Some(sha256) = file.sha256.as_deref() else {
            return Ok(FileState::Unchecked);
        };
        let path = match &file.real_path {
            Some(real) => PathBuf::from(real),
            None => project_root.join(&file.path),
        };
        compare_file(&path, file.size, sha256)
    }
}

// ---- The references for `explain` (design section 7, item 8) ----

impl View<'_> {
    /// The reference at exactly this span of `path`, or `None` when the span is not a reference.
    ///
    /// A first version: it filters the references of the file. It is enough for one `explain`.
    pub fn references_at(
        &self,
        path: &str,
        span: (u32, u32),
    ) -> Result<Option<ReferenceRecord>, ScippyError> {
        Ok(self
            .references(ReferenceQuery {
                path: Some(path.to_owned()),
                outcome: None,
            })?
            .into_iter()
            .find(|r| r.span == span))
    }

    /// The ambiguous references in `path` that list `symbol` as a candidate, sorted by span.
    ///
    /// The search did not pick the symbol. These references show where the file may depend on it.
    pub fn ambiguous_with(
        &self,
        path: &str,
        symbol: &SymbolRef,
    ) -> Result<Vec<ReferenceRecord>, ScippyError> {
        Ok(self
            .references(ReferenceQuery {
                path: Some(path.to_owned()),
                outcome: Some(OutcomeKind::Ambiguous),
            })?
            .into_iter()
            .filter(|r| matches!(&r.outcome, Outcome::Ambiguous(c) if c.contains(symbol)))
            .collect())
    }
}

#[cfg(test)]
mod explain_tests {
    use super::*;
    use crate::tests::{layer, publish, rich};

    #[test]
    fn references_at_and_ambiguous_with_read_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut layer = layer(dir.path());
        publish(&mut layer, &rich());
        let view = layer.view().unwrap().unwrap();
        let at = view.references_at("a.py", (14, 15)).unwrap().unwrap();
        assert_eq!(at.outcome, Outcome::Resolved(SymbolRef::global("g f().")));
        assert!(view.references_at("a.py", (14, 16)).unwrap().is_none());
        assert!(view.references_at("b.py", (14, 15)).unwrap().is_none());

        let ambiguous = view
            .ambiguous_with("a.py", &SymbolRef::global("g ext."))
            .unwrap();
        assert_eq!(ambiguous.len(), 1);
        assert_eq!(ambiguous[0].span, (44, 45));
        assert!(
            view.ambiguous_with("a.py", &SymbolRef::global("g f()."))
                .unwrap()
                .is_empty()
        );
        assert!(
            view.ambiguous_with("b.py", &SymbolRef::global("g ext."))
                .unwrap()
                .is_empty()
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Relations and edges (`relations` and `edges`): `View::relations` and `View::edges_where`.
// ---------------------------------------------------------------------------------------------

/// Which file-to-symbol relations to read. A field that is `None` or empty does not filter.
#[derive(Clone, Debug, Default)]
pub struct RelationQuery2 {
    /// The source file.
    pub path: Option<String>,
    /// The target symbol.
    pub target: Option<SymbolRef>,
    /// The exact display name of the target.
    pub target_name: Option<String>,
    /// The kinds to keep. An empty list keeps every kind.
    pub kinds: Vec<RelationKind>,
    /// `Some(true)` keeps the external targets, and `Some(false)` keeps the internal targets.
    pub external: Option<bool>,
}

/// A file-to-symbol relation, with the data of its target.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RelationRecord {
    pub path: String,
    pub target: SymbolRecord,
    pub kind: RelationKind,
    /// The count of supports.
    pub supports: u32,
    /// The supports that rest on a stopped search.
    pub limited: u32,
}

/// Which edges to read. A field that is `None` or empty does not filter.
#[derive(Clone, Debug, Default)]
pub struct EdgeQuery {
    /// The kinds to keep. An empty list keeps every kind.
    pub kinds: Vec<EdgeKind>,
    /// The file of the source site.
    pub path: Option<String>,
    pub target: Option<SymbolRef>,
    /// The state of a link, such as `resolved`, `missing`, `ambiguous`, or `none`.
    pub state: Option<String>,
}

impl View<'_> {
    /// The relations that match the query, sorted by path, target symbol, and kind.
    pub fn relations(&self, q: RelationQuery2) -> Result<Vec<RelationRecord>, ScippyError> {
        let target = match &q.target {
            None => None,
            Some(symbol) => match self.symbol_id(symbol)? {
                Some(id) => Some(id),
                None => return Ok(Vec::new()),
            },
        };
        let sql = format!(
            "SELECT f.path, fr.kind, fr.supports, fr.limited, {}, t.kind, t.display_name, tf.path
             FROM file_reference fr
             JOIN file f ON f.id = fr.file_id
             JOIN symbol t ON t.id = fr.target_id
             LEFT JOIN file tf ON tf.id = t.file_id
             WHERE (?1 IS NULL OR f.path = ?1) AND (?2 IS NULL OR fr.target_id = ?2)
               AND (?3 IS NULL OR t.display_name = ?3)
               AND (?4 IS NULL OR (t.file_id IS NULL) = ?4)
             ORDER BY f.path, t.symbol, fr.kind",
            symbol_columns("t", "tf")
        );
        let mut statement = self.tx.prepare(&sql).map_err(storage)?;
        let rows = statement
            .query_map(params![q.path, target, q.target_name, q.external], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    symbol_at(r, 4)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ))
            })
            .map_err(storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (path, kind, supports, limited, symbol, symbol_kind, display_name, defined_in) =
                row.map_err(storage)?;
            let kind = RelationKind::parse(&kind)?;
            if !q.kinds.is_empty() && !q.kinds.contains(&kind) {
                continue;
            }
            out.push(RelationRecord {
                path,
                target: SymbolRecord {
                    symbol,
                    kind: symbol_kind,
                    display_name,
                    external: defined_in.is_none(),
                    defined_in,
                },
                kind,
                supports,
                limited,
            });
        }
        Ok(out)
    }

    /// The edges that match the query, sorted by path, span, kind, source, and target.
    pub fn edges_where(&self, q: &EdgeQuery) -> Result<Vec<EdgeRecord>, ScippyError> {
        let target = match &q.target {
            None => None,
            Some(symbol) => match self.symbol_id(symbol)? {
                Some(id) => Some(id),
                None => return Ok(Vec::new()),
            },
        };
        let sql = format!(
            "SELECT e.kind, f.path, e.start, e.end, e.link_kind, e.label, e.state, e.anchor_state,
                    {}, {}, e.id, {}
             FROM edge e
             JOIN file f ON f.id = e.file_id
             LEFT JOIN symbol s ON s.id = e.source_id
             LEFT JOIN file sf ON sf.id = s.file_id
             LEFT JOIN symbol t ON t.id = e.target_id
             LEFT JOIN file tf ON tf.id = t.file_id
             LEFT JOIN reference ar ON ar.id = e.anchor_reference_id
             LEFT JOIN symbol a ON a.id = ar.target_id
             LEFT JOIN file af ON af.id = a.file_id
             WHERE (?1 IS NULL OR f.path = ?1) AND (?2 IS NULL OR e.target_id = ?2)
               AND (?3 IS NULL OR e.state = ?3)
             ORDER BY f.path, e.start, e.end, e.kind, s.symbol, t.symbol, e.id",
            symbol_columns("s", "sf"),
            symbol_columns("t", "tf"),
            symbol_columns("a", "af"),
        );
        let mut statement = self.tx.prepare(&sql).map_err(storage)?;
        let rows = statement
            .query_map(params![q.path, target, q.state], |r| {
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
                        anchor: optional_symbol_at(r, 15)?,
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
            if !q.kinds.is_empty() && !q.kinds.contains(&edge.kind) {
                continue;
            }
            edge.candidates = candidates
                .query_map([id], |r| symbol_at(r, 0))
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            out.push(edge);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod relation_tests {
    use super::*;
    use crate::tests::{layer, publish, rich, vault};

    #[test]
    fn relations_filter_by_name_kind_and_origin() {
        let dir = tempfile::tempdir().unwrap();
        let mut layer = layer(dir.path());
        publish(&mut layer, &rich());
        let view = layer.view().unwrap().unwrap();
        let all = view.relations(RelationQuery2::default()).unwrap();
        assert!(!all.is_empty());
        let external = view
            .relations(RelationQuery2 {
                external: Some(true),
                ..Default::default()
            })
            .unwrap();
        let internal = view
            .relations(RelationQuery2 {
                external: Some(false),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(external.len() + internal.len(), all.len());
        assert!(external.iter().all(|r| r.target.external));
        assert!(internal.iter().all(|r| !r.target.external));
        let named = view
            .relations(RelationQuery2 {
                target_name: Some(all[0].target.display_name.clone()),
                ..Default::default()
            })
            .unwrap();
        assert!(named.contains(&all[0]));
        let none = view
            .relations(RelationQuery2 {
                kinds: vec![RelationKind::Embed],
                ..Default::default()
            })
            .unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn edges_filter_by_state_and_carry_the_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let mut layer = layer(dir.path());
        publish(&mut layer, &vault());
        let view = layer.view().unwrap().unwrap();
        let missing = view
            .edges_where(&EdgeQuery {
                state: Some("missing".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].span, Some((70, 80)));
        let links = view
            .edges_where(&EdgeQuery {
                kinds: vec![EdgeKind::Link],
                ..Default::default()
            })
            .unwrap();
        let anchored = links.iter().find(|e| e.span == Some((10, 22))).unwrap();
        assert_eq!(anchored.anchor, Some(SymbolRef::global("g t.md/Heading.")));
        let missing_anchor = links.iter().find(|e| e.span == Some((30, 40))).unwrap();
        assert_eq!(missing_anchor.anchor, None);
        let to_u = view
            .edges_where(&EdgeQuery {
                target: Some(SymbolRef::global("g u.md/")),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(to_u.len(), 1);
        assert_eq!(to_u[0].kind, EdgeKind::Embed);
        let in_other_file = view
            .edges_where(&EdgeQuery {
                path: Some("t.md".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(in_other_file.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{layer, publish, rich};

    #[test]
    fn probe_tells_a_missing_path_a_foreign_file_and_a_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layer.sqlite");
        assert_eq!(Metalayer::probe(&path).unwrap(), Probe::Missing);
        std::fs::write(&path, "text, not SQLite\n").unwrap();
        assert_eq!(Metalayer::probe(&path).unwrap(), Probe::NotScippy);
        std::fs::remove_file(&path).unwrap();
        let _ = layer(dir.path());
        assert_eq!(
            Metalayer::probe(&path).unwrap(),
            Probe::Version(crate::SCHEMA_VERSION)
        );
    }

    #[test]
    fn counts_file_and_symbol_read_one_revision() {
        let dir = tempfile::tempdir().unwrap();
        let mut layer = layer(dir.path());
        publish(&mut layer, &rich());
        let view = layer.view().unwrap().unwrap();
        assert_eq!(view.schema_version().unwrap(), crate::SCHEMA_VERSION);
        assert_eq!(view.revision().position_encoding, "utf8-byte");
        assert_eq!(view.revision().scip_size, 4);
        let counts = view.counts().unwrap();
        assert_eq!(counts.files["indexed"], 2);
        assert_eq!(counts.files["failed"], 0);
        assert_eq!(counts.references["ambiguous"], 1);
        assert_eq!(counts.references["bounded"], 1);
        assert_eq!(counts.edges["link"], 0);
        assert_eq!(counts.diagnostics["error"], 0);
        assert!(counts.symbols >= 4);

        let file = view.file("a.py").unwrap().unwrap();
        assert_eq!(file.language.as_deref(), Some("Python"));
        assert_eq!(file.real_path, None);
        assert!(view.file("nope.py").unwrap().is_none());

        let defined = view.symbol(&SymbolRef::global("g f().")).unwrap().unwrap();
        assert_eq!(defined.kind, "Function");
        assert_eq!(defined.defined_in.as_deref(), Some("a.py"));
        assert!(!defined.external);
        let external = view.symbol(&SymbolRef::global("g ext.")).unwrap().unwrap();
        assert!(external.external);
        assert_eq!(external.defined_in, None);
        let local = view
            .symbol(&SymbolRef::local("local 0", "b.py"))
            .unwrap()
            .unwrap();
        assert_eq!(local.defined_in.as_deref(), Some("b.py"));
        assert!(
            view.symbol(&SymbolRef::global("g none."))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn file_state_compares_the_size_and_then_the_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut layer = layer(dir.path());
        publish(&mut layer, &rich());
        let view = layer.view().unwrap().unwrap();
        let mut file = view.file("a.py").unwrap().unwrap();
        let on_disk = dir.path().join("a.py");
        let text = "x = 1\n";
        // The SHA-256 of the text, as the stored hash.
        let digest = {
            let mut hasher = Sha256::new();
            hasher.update(text.as_bytes());
            hasher.finalize().iter().fold(String::new(), |mut out, b| {
                let _ = write!(out, "{b:02x}");
                out
            })
        };
        file.sha256 = Some(digest);
        file.size = Some(text.len() as u64);
        assert_eq!(
            view.file_state(dir.path(), &file).unwrap(),
            FileState::Missing
        );
        std::fs::write(&on_disk, text).unwrap();
        assert_eq!(view.file_state(dir.path(), &file).unwrap(), FileState::Ok);
        std::fs::write(&on_disk, "x = 22\n").unwrap();
        assert_eq!(
            view.file_state(dir.path(), &file).unwrap(),
            FileState::Changed
        );
        std::fs::write(&on_disk, "x = 2\n").unwrap();
        assert_eq!(
            view.file_state(dir.path(), &file).unwrap(),
            FileState::Changed
        );
        file.sha256 = None;
        assert_eq!(
            view.file_state(dir.path(), &file).unwrap(),
            FileState::Unchecked
        );
    }
}
