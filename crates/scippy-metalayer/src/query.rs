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

use crate::schema::{APPLICATION_ID, storage};
use crate::{
    FileRecord, Metalayer, Outcome, OutcomeKind, ReferenceQuery, ReferenceRecord, SymbolRef, View,
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
