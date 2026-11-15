//! Publish one analysis as one revision.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use scippy_core::{
    AnalysisSnapshot, Bound, INCOMPLETE_CODES, ReferenceKind, ReferenceOutcome, ScippyError,
};
use sha2::{Digest, Sha256};

use crate::schema::{SCHEMA_VERSION, storage};

/// The version of the digest format. Change it when the digest input changes.
const DIGEST_VERSION: u32 = 1;

/// What the caller asks for.
#[derive(Clone, Copy, Debug, Default)]
pub struct PublishOptions {
    /// The run asked for `--strict`. The revision records it.
    pub strict: bool,
    /// Replace the root set of the database when it differs from this run.
    pub replace_root_set: bool,
}

/// The revision that a publish committed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Published {
    pub revision: i64,
    /// The SHA-256 of the SCIP bytes of the same run.
    pub scip_sha256: String,
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// One row of the `file` table.
struct FileRow<'a> {
    pack: Option<&'a str>,
    language: Option<&'a str>,
    status: &'a str,
    reason: Option<&'a str>,
    sha256: Option<&'a str>,
    size: Option<i64>,
    real_path: Option<&'a str>,
    /// The file on disk differs from the content that the run indexed.
    stale: bool,
}

/// One row of the input manifest, as the digest sees it.
struct ManifestRow<'a> {
    path: &'a str,
    status: &'a str,
    reason: Option<&'a str>,
    sha256: Option<&'a str>,
}

/// The digest of the input of a run: the settings and the manifest. It describes an input.
fn input_digest(s: &AnalysisSnapshot, key: &str, manifest: &[ManifestRow<'_>]) -> String {
    let mut text = String::new();
    // Each field has a length prefix, so no field value can imitate a separator.
    let mut field = |value: &str| {
        let _ = writeln!(text, "{}:{value}", value.len());
    };
    field(&DIGEST_VERSION.to_string());
    field(&SCHEMA_VERSION.to_string());
    field(&s.tool_version);
    field(key);
    field(&s.options);
    for row in manifest {
        field(row.path);
        field(row.status);
        field(row.reason.unwrap_or(""));
        field(row.sha256.unwrap_or(""));
    }
    sha256_hex(text.as_bytes())
}

/// The symbols of one publish, by the key that the database enforces.
struct Symbols {
    ids: HashMap<(Option<i64>, String), i64>,
}

impl Symbols {
    /// Add a symbol with its data. The first record of a symbol wins.
    fn insert(
        &mut self,
        tx: &Transaction<'_>,
        file: Option<i64>,
        symbol: &str,
        kind: &str,
        display_name: &str,
    ) -> Result<i64, ScippyError> {
        let local = scip_local(symbol);
        let key = (local.then_some(file).flatten(), symbol.to_owned());
        if let Some(id) = self.ids.get(&key) {
            return Ok(*id);
        }
        if local && file.is_none() {
            return Err(ScippyError::Storage(format!(
                "the local symbol {symbol:?} has no document"
            )));
        }
        tx.prepare_cached(
            "INSERT INTO symbol (root_id, symbol, local, file_id, kind, display_name)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)",
        )
        .and_then(|mut s| s.execute(params![symbol, local, file, kind, display_name]))
        .map_err(storage)?;
        let id = tx.last_insert_rowid();
        self.ids.insert(key, id);
        Ok(id)
    }

    /// The id of a symbol that a fact names. A symbol that no record declares gets a row with
    /// the kind `Unspecified`, so every fact keeps its target. `file` is the document of the
    /// fact. A global symbol that no document defines is external.
    fn of_fact(
        &mut self,
        tx: &Transaction<'_>,
        file: i64,
        symbol: &str,
    ) -> Result<i64, ScippyError> {
        let local = scip_local(symbol);
        let key = (local.then_some(file), symbol.to_owned());
        if let Some(id) = self.ids.get(&key) {
            return Ok(*id);
        }
        self.insert(tx, local.then_some(file), symbol, "Unspecified", "")
    }
}

fn scip_local(symbol: &str) -> bool {
    symbol.starts_with("local ")
}

pub(crate) fn publish(
    db: &mut Connection,
    s: &AnalysisSnapshot,
    o: &PublishOptions,
) -> Result<Published, ScippyError> {
    // A strict run rejects incomplete input before it opens a transaction.
    if o.strict && !s.input_complete {
        return Err(ScippyError::Incomplete);
    }
    let key = sha256_hex(s.project_root.as_bytes());
    // An immediate transaction takes the write lock before it reads the revision.
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;

    // The root set.
    let stored: Option<(String, String)> = tx
        .query_row(
            "SELECT key, project_root FROM root_set WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })
        .map_err(storage)?;
    match stored {
        None => {
            tx.execute(
                "INSERT INTO root_set (id, key, project_root) VALUES (1, ?1, ?2)",
                params![key, s.project_root],
            )
            .map_err(storage)?;
        }
        Some((stored_key, _)) if stored_key == key => {}
        Some((_, stored_root)) => {
            if !o.replace_root_set {
                return Err(ScippyError::RootSetMismatch {
                    stored: stored_root,
                    requested: s.project_root.clone(),
                });
            }
            tx.execute(
                "UPDATE root_set SET key = ?1, project_root = ?2 WHERE id = 1",
                params![key, s.project_root],
            )
            .map_err(storage)?;
        }
    }

    // The revision id. Never reuse an id: the new id follows the highest id.
    let parent: Option<i64> = tx
        .query_row("SELECT max(id) FROM revision", [], |r| r.get(0))
        .map_err(storage)?;
    let revision = parent
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| ScippyError::Storage("revision overflow".into()))?;

    // Replace every fact. Delete the child tables first.
    for table in [
        "file_reference",
        "edge",
        "reference_candidate",
        "reference",
        "occurrence",
        "diagnostic",
        "symbol",
        "file",
        "root_member",
    ] {
        tx.execute(&format!("DELETE FROM {table}"), [])
            .map_err(storage)?;
    }
    for member in std::iter::once("").chain(s.shards.iter().map(String::as_str)) {
        tx.execute(
            "INSERT OR IGNORE INTO root_member (root_id, path) VALUES (1, ?1)",
            [member],
        )
        .map_err(storage)?;
    }

    // Files. A lost file has a row with its status. A document without a lost record is indexed.
    // A document that has a lost record keeps its facts under the row of the lost file.
    let mut documents: Vec<&scippy_core::AnalysisDocument> = s.documents.iter().collect();
    documents.sort_by(|a, b| a.path.cmp(&b.path));
    let mut rows: BTreeMap<&str, FileRow<'_>> = BTreeMap::new();
    for lost in &s.lost_files {
        rows.entry(lost.path.as_str()).or_insert(FileRow {
            pack: lost.pack.as_deref(),
            language: None,
            status: lost.status.as_str(),
            reason: Some(lost.reason.as_str()),
            sha256: lost.sha256.as_deref(),
            size: None,
            real_path: lost.real_path.as_deref(),
            stale: false,
        });
    }
    for d in &documents {
        let language = (!d.language.is_empty()).then_some(d.language.as_str());
        match rows.get_mut(d.path.as_str()) {
            Some(row) => {
                row.language = language;
                if row.pack.is_none() {
                    row.pack = d.pack.as_deref();
                }
            }
            None => {
                rows.insert(
                    d.path.as_str(),
                    FileRow {
                        pack: d.pack.as_deref(),
                        language,
                        status: "indexed",
                        reason: None,
                        // A stub has no hash and no size.
                        sha256: d.sha256.as_deref(),
                        size: d.sha256.is_some().then_some(d.source.len() as i64),
                        real_path: d.real_path.as_deref(),
                        stale: d.stale,
                    },
                );
            }
        }
    }
    let mut files: HashMap<&str, i64> = HashMap::with_capacity(rows.len());
    let mut manifest = Vec::with_capacity(rows.len());
    for (path, row) in &rows {
        tx.prepare_cached(
            "INSERT INTO file (root_id, path, pack, language, status, reason, sha256, size, real_path, stale)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .and_then(|mut st| {
            st.execute(params![
                path,
                row.pack,
                row.language,
                row.status,
                row.reason,
                row.sha256,
                row.size,
                row.real_path,
                row.stale,
            ])
        })
        .map_err(storage)?;
        files.insert(path, tx.last_insert_rowid());
        manifest.push(ManifestRow {
            path,
            status: row.status,
            reason: row.reason,
            sha256: row.sha256,
        });
    }

    // Symbols that the documents declare, then the external symbols.
    let mut symbols = Symbols {
        ids: HashMap::new(),
    };
    for d in &documents {
        let file = files[d.path.as_str()];
        for info in &d.symbols {
            symbols.insert(
                &tx,
                Some(file),
                &info.symbol,
                &info.kind,
                &info.display_name,
            )?;
        }
    }
    for info in &s.external_symbols {
        if scip_local(&info.symbol) {
            return Err(ScippyError::Storage(format!(
                "the external symbol {:?} is local",
                info.symbol
            )));
        }
        symbols.insert(&tx, None, &info.symbol, &info.kind, &info.display_name)?;
    }

    // Occurrences and the edges that the declarations carry. An import occurrence marks the
    // reference at the same span as an import reference.
    let mut imports: HashSet<(i64, u32, u32, i64)> = HashSet::new();
    for d in &documents {
        let file = files[d.path.as_str()];
        for occurrence in &d.occurrences {
            let symbol = symbols.of_fact(&tx, file, &occurrence.symbol)?;
            if occurrence.role == "import" {
                imports.insert((file, occurrence.start, occurrence.end, symbol));
            }
            tx.prepare_cached(
                "INSERT OR IGNORE INTO occurrence (file_id, start, end, symbol_id, role)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .and_then(|mut st| {
                st.execute(params![
                    file,
                    occurrence.start,
                    occurrence.end,
                    symbol,
                    occurrence.role
                ])
            })
            .map_err(storage)?;
        }
        let mut seen: HashSet<(&str, i64, i64)> = HashSet::new();
        for info in &d.symbols {
            let source = symbols.of_fact(&tx, file, &info.symbol)?;
            for (kind, targets) in [("override", &info.implements), ("alias", &info.references)] {
                for target in targets {
                    let target = symbols.of_fact(&tx, file, target)?;
                    if !seen.insert((kind, source, target)) {
                        continue;
                    }
                    tx.prepare_cached(
                        "INSERT INTO edge (kind, file_id, source_id, target_id) VALUES (?1, ?2, ?3, ?4)",
                    )
                    .and_then(|mut st| st.execute(params![kind, file, source, target]))
                    .map_err(storage)?;
                }
            }
        }
    }

    // References, the candidates of the ambiguous ones, and the import edges.
    // `located` maps a reference location to its row and its target.
    let mut located: HashMap<(i64, u32, u32), (i64, Option<i64>)> =
        HashMap::with_capacity(s.references.len());
    for r in &s.references {
        let file = *files.get(r.path.as_str()).ok_or_else(|| {
            ScippyError::Storage(format!(
                "a reference lies in {:?}, which is not a document",
                r.path
            ))
        })?;
        let (outcome, target, reason, limited) = match &r.outcome {
            ReferenceOutcome::Resolved(t) => (
                "resolved",
                Some(symbols.of_fact(&tx, file, t)?),
                None,
                r.limited.map(Bound::code),
            ),
            ReferenceOutcome::Ambiguous(_) => ("ambiguous", None, None, None),
            ReferenceOutcome::Unresolved(why) => ("unresolved", None, Some(why.as_str()), None),
            ReferenceOutcome::Bounded(bound) => ("bounded", None, Some(bound.as_str()), None),
        };
        tx.prepare_cached(
            "INSERT INTO reference (file_id, start, end, kind, outcome, target_id, reason, limited)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .and_then(|mut st| {
            st.execute(params![
                file,
                r.start,
                r.end,
                r.kind.map(ReferenceKind::code),
                outcome,
                target,
                reason,
                limited
            ])
        })
        .map_err(storage)?;
        let id = tx.last_insert_rowid();
        located.insert((file, r.start, r.end), (id, target));
        if let Some(target) = target {
            if imports.contains(&(file, r.start, r.end, target)) {
                tx.prepare_cached(
                    "INSERT INTO edge (kind, file_id, target_id, start, end, reference_id)
                     VALUES ('import', ?1, ?2, ?3, ?4, ?5)",
                )
                .and_then(|mut st| st.execute(params![file, target, r.start, r.end, id]))
                .map_err(storage)?;
            }
        }
        if let ReferenceOutcome::Ambiguous(candidates) = &r.outcome {
            let mut sorted: Vec<&String> = candidates.iter().collect();
            sorted.sort();
            sorted.dedup();
            for (ordinal, candidate) in sorted.into_iter().enumerate() {
                let symbol = symbols.of_fact(&tx, file, candidate)?;
                tx.prepare_cached(
                    "INSERT INTO reference_candidate (reference_id, ordinal, symbol_id) VALUES (?1, ?2, ?3)",
                )
                .and_then(|mut st| st.execute(params![id, ordinal as i64, symbol]))
                .map_err(storage)?;
            }
        }
    }

    // The edges that the run gave: inheritance, links, and embeds. An edge names its references
    // by location.
    for e in &s.edges {
        let file = *files.get(e.path.as_str()).ok_or_else(|| {
            ScippyError::Storage(format!("an edge lies in {:?}, which is not a file", e.path))
        })?;
        let find = |span: Option<(u32, u32)>| -> Result<Option<(i64, Option<i64>)>, ScippyError> {
            span.map(|(start, end)| {
                located.get(&(file, start, end)).copied().ok_or_else(|| {
                    ScippyError::Storage(format!(
                        "an edge in {:?} names a reference at {start}..{end}, which does not exist",
                        e.path
                    ))
                })
            })
            .transpose()
        };
        let reference = find(e.reference_span)?;
        let anchor = find(e.anchor_span)?;
        let source = e
            .source
            .as_deref()
            .map(|symbol| symbols.of_fact(&tx, file, symbol))
            .transpose()?;
        tx.prepare_cached(
            "INSERT INTO edge (kind, file_id, source_id, target_id, start, end, reference_id,
                               anchor_reference_id, link_kind, label, state, anchor_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .and_then(|mut st| {
            st.execute(params![
                e.kind,
                file,
                source,
                reference.and_then(|r| r.1),
                e.span.0,
                e.span.1,
                reference.map(|r| r.0),
                anchor.map(|a| a.0),
                e.link_kind,
                e.label,
                e.state,
                e.anchor_state,
            ])
        })
        .map_err(storage)?;
    }

    // Diagnostics.
    for d in &s.diagnostics {
        tx.prepare_cached(
            "INSERT INTO diagnostic (root_id, severity, stage, code, message, path, start, end)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .and_then(|mut st| {
            st.execute(params![
                d.severity.to_lowercase(),
                d.stage.to_lowercase(),
                d.code,
                d.message,
                d.file,
                d.range.map(|r| r.0),
                d.range.map(|r| r.1),
            ])
        })
        .map_err(storage)?;
    }

    // The relation. It follows from the facts in the same transaction.
    tx.execute(
        "INSERT INTO file_reference (file_id, target_id, kind, supports, limited)
         SELECT file_id, target_id, kind, count(*), count(limited) FROM support
         GROUP BY file_id, target_id, kind",
        [],
    )
    .map_err(storage)?;

    // The flags follow from the rows. A snapshot that disagrees is a bug in the caller.
    let codes = INCOMPLETE_CODES
        .iter()
        .map(|c| format!("'{c}'"))
        .collect::<Vec<_>>()
        .join(",");
    let input_complete: bool = tx
        .query_row(
            &format!(
                "SELECT NOT EXISTS (SELECT 1 FROM file WHERE status IN ('skipped','failed'))
                    AND NOT EXISTS (SELECT 1 FROM diagnostic
                                    WHERE severity = 'error' OR code IN ({codes}))"
            ),
            [],
            |r| r.get(0),
        )
        .map_err(storage)?;
    let analysis_exact: bool = tx
        .query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM reference WHERE outcome = 'bounded' OR limited IS NOT NULL)",
            [],
            |r| r.get(0),
        )
        .map_err(storage)?;
    if input_complete != s.input_complete || analysis_exact != s.analysis_exact {
        return Err(ScippyError::Storage(format!(
            "the snapshot flags disagree with its rows: input_complete {} and analysis_exact {} in the snapshot, {input_complete} and {analysis_exact} in the rows",
            s.input_complete, s.analysis_exact
        )));
    }

    // The revision row. Older revisions go.
    let scip_sha256 = sha256_hex(&s.scip);
    tx.execute(
        "INSERT INTO revision (id, root_id, parent, input_digest, tool_version, position_encoding,
                               input_complete, analysis_exact, strict, scip_sha256, scip_size)
         VALUES (?1, 1, ?2, ?3, ?4, 'utf8-byte', ?5, ?6, ?7, ?8, ?9)",
        params![
            revision,
            parent,
            input_digest(s, &key, &manifest),
            s.tool_version,
            input_complete,
            analysis_exact,
            o.strict,
            scip_sha256,
            s.scip.len() as i64,
        ],
    )
    .map_err(storage)?;
    tx.execute("DELETE FROM revision WHERE id < ?1", [revision])
        .map_err(storage)?;
    tx.commit().map_err(storage)?;
    Ok(Published {
        revision,
        scip_sha256,
    })
}
