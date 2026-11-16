//! The tables, the views, and the version of the database.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use scippy_core::ScippyError;

/// Marks a database file as a Scippy read model. The bytes spell "SCPY".
pub(crate) const APPLICATION_ID: i32 = 0x5343_5059;
/// The storage format version that this code reads and writes.
pub const SCHEMA_VERSION: i32 = 10;
/// The SHA-256 of `DDL` for each version that shipped, oldest first. The last entry is the
/// current version. A change of `DDL` needs a new entry with a higher `SCHEMA_VERSION`: the
/// test `the_ddl_hash_follows_the_version` fails otherwise. Never edit an old entry.
#[cfg(test)]
pub(crate) const DDL_HISTORY: &[(i32, &str)] = &[
    (
        7,
        "628c3a98261c0f3fe65869d75dd9133ab882663d9660614bc742c3f196a0386a",
    ),
    (
        8,
        "9700c08e00268552fa6c38b14c4f632567ae9d7ebd7d8acc1ab6cbb26d7497e7",
    ),
    (
        9,
        "aef21da97b62fe2d79dc80a297ca1148025db9e4c88f587c44aecb6bd4a57705",
    ),
    (
        10,
        "bca1e138e06926cd1f83f0359033a03616975662ec71d81baeae7e7b61d0f4fe",
    ),
];

/// The tables and views. The design is in `METALAYER_SCHEMA.md`, section 3.
pub(crate) const DDL: &str = "
CREATE TABLE root_set (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  key TEXT NOT NULL,
  project_root TEXT NOT NULL
);
CREATE TABLE root_member (
  path TEXT NOT NULL PRIMARY KEY
) WITHOUT ROWID;

CREATE TABLE revision (
  id INTEGER PRIMARY KEY,
  input_digest TEXT NOT NULL,
  tool_version TEXT NOT NULL,
  position_encoding TEXT NOT NULL CHECK (position_encoding = 'utf8-byte'),
  input_complete INTEGER NOT NULL CHECK (input_complete IN (0,1)),
  analysis_exact INTEGER NOT NULL CHECK (analysis_exact IN (0,1)),
  strict INTEGER NOT NULL CHECK (strict IN (0,1)),
  scip_sha256 TEXT NOT NULL, scip_size INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- A support file that the run read, such as `pyproject.toml`. It is not a source file.
-- `sha256` is NULL when the file did not exist at the start of the run.
CREATE TABLE support_input (
  path TEXT PRIMARY KEY,
  real_path TEXT,
  sha256 TEXT
) WITHOUT ROWID;

CREATE TABLE file (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL,
  pack TEXT, language TEXT,
  status TEXT NOT NULL CHECK (status IN ('indexed','skipped','failed','shadowed')),
  reason TEXT,
  sha256 TEXT, size INTEGER,
  real_path TEXT,
  stale INTEGER NOT NULL DEFAULT 0 CHECK (stale IN (0,1)),
  UNIQUE (path),
  CHECK ((status = 'indexed') = (reason IS NULL))
);

CREATE TABLE symbol (
  id INTEGER PRIMARY KEY,
  symbol TEXT NOT NULL,
  local INTEGER NOT NULL CHECK (local IN (0,1)),
  file_id INTEGER REFERENCES file(id),
  kind TEXT NOT NULL, display_name TEXT NOT NULL,
  CHECK (local = 0 OR file_id IS NOT NULL)
);
CREATE UNIQUE INDEX symbol_global ON symbol(symbol) WHERE local = 0;
CREATE UNIQUE INDEX symbol_local  ON symbol(file_id, symbol) WHERE local = 1;
CREATE INDEX symbol_file ON symbol(file_id);

CREATE TABLE occurrence (
  file_id INTEGER NOT NULL REFERENCES file(id),
  start INTEGER NOT NULL, end INTEGER NOT NULL,
  symbol_id INTEGER NOT NULL REFERENCES symbol(id),
  role TEXT NOT NULL CHECK (role IN ('definition','reference','import','read')),
  PRIMARY KEY (file_id, start, end, symbol_id, role)
) WITHOUT ROWID;
CREATE INDEX occurrence_symbol ON occurrence(symbol_id, file_id);

CREATE TABLE reference (
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES file(id),
  start INTEGER NOT NULL, end INTEGER NOT NULL,
  kind TEXT CHECK (kind IN ('plain','member','keyword')),
  outcome TEXT NOT NULL CHECK (outcome IN ('resolved','ambiguous','unresolved','bounded')),
  target_id INTEGER REFERENCES symbol(id),
  reason TEXT,
  limited TEXT CHECK (limited IN ('work','queue-length','arena-cells','failed')),
  UNIQUE (file_id, start, end),
  CHECK ((outcome = 'resolved') = (target_id IS NOT NULL)),
  CHECK ((outcome IN ('unresolved','bounded')) = (reason IS NOT NULL)),
  CHECK (limited IS NULL OR outcome = 'resolved')
);
CREATE INDEX reference_target ON reference(target_id) WHERE target_id IS NOT NULL;
CREATE INDEX reference_open ON reference(outcome, reason) WHERE outcome <> 'resolved';

CREATE TABLE reference_candidate (
  reference_id INTEGER NOT NULL REFERENCES reference(id),
  ordinal INTEGER NOT NULL,
  symbol_id INTEGER NOT NULL REFERENCES symbol(id),
  PRIMARY KEY (reference_id, ordinal)
) WITHOUT ROWID;

CREATE TABLE edge (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('import','inheritance','override','alias','link','embed')),
  file_id INTEGER NOT NULL REFERENCES file(id),
  source_id INTEGER REFERENCES symbol(id),
  target_id INTEGER REFERENCES symbol(id),
  start INTEGER, end INTEGER,
  reference_id INTEGER REFERENCES reference(id),
  anchor_reference_id INTEGER REFERENCES reference(id),
  link_kind TEXT,
  label TEXT,
  state TEXT CHECK (state IN ('resolved','missing','ambiguous','skipped-target','none')),
  anchor_state TEXT
);
CREATE INDEX edge_file ON edge(file_id, kind);
CREATE INDEX edge_target ON edge(target_id, kind);
CREATE INDEX edge_source ON edge(source_id, kind);
CREATE INDEX edge_reference ON edge(reference_id);
CREATE INDEX edge_anchor ON edge(anchor_reference_id);

CREATE TABLE diagnostic (
  id INTEGER PRIMARY KEY,
  severity TEXT NOT NULL CHECK (severity IN ('error','warning','info')),
  stage TEXT NOT NULL, code TEXT NOT NULL, message TEXT NOT NULL,
  path TEXT,
  start INTEGER, end INTEGER,
  CHECK ((start IS NULL) = (end IS NULL))
);
CREATE INDEX diagnostic_path ON diagnostic(path);
CREATE INDEX diagnostic_code ON diagnostic(code);

CREATE VIEW support AS
SELECT r.id AS reference_id, r.file_id, r.start, r.end, r.target_id, r.limited,
  CASE WHEN EXISTS (SELECT 1 FROM edge e WHERE e.kind = 'link'  AND (e.reference_id = r.id OR e.anchor_reference_id = r.id)) THEN 'link'
       WHEN EXISTS (SELECT 1 FROM edge e WHERE e.kind = 'embed' AND (e.reference_id = r.id OR e.anchor_reference_id = r.id)) THEN 'embed'
       WHEN EXISTS (SELECT 1 FROM occurrence o WHERE o.file_id = r.file_id AND o.start = r.start
                    AND o.end = r.end AND o.symbol_id = r.target_id AND o.role = 'import') THEN 'import'
       ELSE 'use' END AS kind
FROM reference r JOIN symbol t ON t.id = r.target_id
WHERE r.outcome = 'resolved' AND t.local = 0
  AND NOT EXISTS (SELECT 1 FROM occurrence o WHERE o.file_id = r.file_id AND o.start = r.start
                  AND o.end = r.end AND o.symbol_id = r.target_id AND o.role = 'definition');

CREATE TABLE file_reference (
  file_id INTEGER NOT NULL REFERENCES file(id),
  target_id INTEGER NOT NULL REFERENCES symbol(id),
  kind TEXT NOT NULL CHECK (kind IN ('import','use','link','embed')),
  supports INTEGER NOT NULL,
  limited INTEGER NOT NULL,
  PRIMARY KEY (file_id, target_id, kind)
) WITHOUT ROWID;
CREATE INDEX file_reference_target ON file_reference(target_id, kind);
";

pub(crate) fn storage(error: rusqlite::Error) -> ScippyError {
    ScippyError::Storage(error.to_string())
}

/// The identity and the version of a database file.
struct Header {
    application_id: i32,
    version: i32,
    has_tables: bool,
}

fn header(db: &Connection) -> Result<Header, ScippyError> {
    let number = |pragma: &str| -> Result<i32, ScippyError> {
        db.query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0))
            .map_err(storage)
    };
    Ok(Header {
        application_id: number("application_id")?,
        version: number("user_version")?,
        has_tables: db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table')",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?,
    })
}

/// Refuse a file that is not empty and not a Scippy database. The check reads only: it must
/// run before any write, so that a foreign file stays as it was.
fn refuse_foreign(found: &Header) -> Result<(), ScippyError> {
    if found.application_id == 0 && found.version == 0 {
        if found.has_tables {
            return Err(ScippyError::Storage(
                "the file is not a Scippy metalayer database: it has tables and no application id"
                    .into(),
            ));
        }
    } else if found.application_id != APPLICATION_ID {
        return Err(ScippyError::Storage(
            "the file is not a Scippy metalayer database".into(),
        ));
    }
    Ok(())
}

/// Open the database for writing. Create the tables, or rebuild a database with an older version.
pub(crate) fn open(path: &Path) -> Result<Connection, ScippyError> {
    let mut db = Connection::open(path).map_err(storage)?;
    db.busy_timeout(Duration::from_secs(30)).map_err(storage)?;
    // Check the application id first. The WAL pragma writes the header of the file.
    refuse_foreign(&header(&db)?)?;
    // A rebuild drops tables in one step. The foreign keys must wait until it ends.
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=OFF;")
        .map_err(storage)?;
    // An immediate transaction makes concurrent first opens take turns.
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let found = header(&tx)?;
    let mut rebuilt = None;
    // The file can change between the first check and the lock. Check again.
    refuse_foreign(&found)?;
    if found.application_id == 0 && found.version == 0 {
        // A new file.
    } else if found.version > SCHEMA_VERSION {
        return Err(ScippyError::Storage(format!(
            "the metalayer database has format version {}, but this build reads version {SCHEMA_VERSION}; use a newer scippy, or delete the database",
            found.version
        )));
    } else if found.version < SCHEMA_VERSION {
        drop_all(&tx)?;
        rebuilt = Some(found.version);
    }
    if found.version != SCHEMA_VERSION || rebuilt.is_some() {
        tx.execute_batch(DDL).map_err(storage)?;
        tx.execute_batch(&format!(
            "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version={SCHEMA_VERSION};"
        ))
        .map_err(storage)?;
    }
    tx.commit().map_err(storage)?;
    db.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(storage)?;
    if let Some(old) = rebuilt {
        eprintln!(
            "scippy: rebuilt the metalayer database: format version {old} became {SCHEMA_VERSION}"
        );
    }
    Ok(db)
}

/// Open the database for reading. Never write, create, or rebuild it.
pub(crate) fn open_read_only(path: &Path) -> Result<Connection, ScippyError> {
    let db =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(storage)?;
    db.busy_timeout(Duration::from_secs(30)).map_err(storage)?;
    let found = header(&db)?;
    if found.application_id != APPLICATION_ID {
        return Err(ScippyError::Storage(
            "the file is not a Scippy metalayer database; run `scippy index` to create one".into(),
        ));
    }
    if found.version != SCHEMA_VERSION {
        return Err(ScippyError::Storage(format!(
            "the metalayer database has format version {}, but this build reads version {SCHEMA_VERSION}; run `scippy index` to rebuild it",
            found.version
        )));
    }
    Ok(db)
}

/// Drop every view and table, and the indexes with them.
fn drop_all(db: &Connection) -> Result<(), ScippyError> {
    let names = |kind: &str| -> Result<Vec<String>, ScippyError> {
        let mut statement = db
            .prepare(&format!(
                "SELECT name FROM sqlite_master WHERE type='{kind}' AND name NOT LIKE 'sqlite_%'"
            ))
            .map_err(storage)?;
        let rows = statement.query_map([], |r| r.get(0)).map_err(storage)?;
        rows.collect::<Result<Vec<String>, _>>().map_err(storage)
    };
    for view in names("view")? {
        db.execute_batch(&format!("DROP VIEW \"{view}\""))
            .map_err(storage)?;
    }
    for table in names("table")? {
        db.execute_batch(&format!("DROP TABLE \"{table}\""))
            .map_err(storage)?;
    }
    Ok(())
}
