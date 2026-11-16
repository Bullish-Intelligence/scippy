//! The identity store: a second SQLite file with entities, locators, and annotations.
//!
//! The facts store drops its tables when its version changes. This store never drops a table.
//! The design is in `METALAYER_IDENTITY.md`, sections 3 and 4. This file holds the tables, the
//! open rules, the record types, and the low-level helpers. The rebinding engine and the
//! annotation API (`identity/ops.rs`) call the helpers.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use scippy_core::ScippyError;

use crate::schema::storage;

/// Marks a database file as a Scippy identity store. The bytes spell "SCID".
pub(crate) const IDENTITY_APPLICATION_ID: i32 = 0x5343_4944;
/// The identity store format version that this code reads and writes.
pub const IDENTITY_VERSION: i32 = 1;
/// A locator history holds the newest entries only. Seq 0 is the current locator.
pub(crate) const LOCATOR_HISTORY: usize = 16;
/// The separator of the display names in `Locator::container`.
pub(crate) const CONTAINER_SEPARATOR: char = '\u{1f}';

/// The tables. The design is in `METALAYER_IDENTITY.md`, section 4.
pub(crate) const IDENTITY_DDL: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
CREATE TABLE entity (
  id TEXT PRIMARY KEY,
  family TEXT NOT NULL CHECK (family IN ('code','note')),
  kind TEXT NOT NULL,
  parent_id TEXT,
  created_at TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE locator (
  entity_id TEXT NOT NULL REFERENCES entity(id), seq INTEGER NOT NULL,
  key TEXT NOT NULL,
  symbol TEXT, path TEXT NOT NULL, kind TEXT NOT NULL, name TEXT NOT NULL,
  container TEXT NOT NULL,
  sig_digest TEXT, body_digest TEXT, body_tokens INTEGER,
  input_digest TEXT NOT NULL,
  PRIMARY KEY (entity_id, seq)
) WITHOUT ROWID;
CREATE INDEX locator_key ON locator(key);
CREATE TABLE outcome (
  entity_id TEXT PRIMARY KEY REFERENCES entity(id),
  outcome TEXT NOT NULL CHECK (outcome IN ('moved','ambiguous','orphaned')),
  tier TEXT, candidates TEXT,
  since_digest TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE veto (entity_id TEXT NOT NULL, key TEXT NOT NULL, PRIMARY KEY (entity_id, key)) WITHOUT ROWID;
CREATE TABLE annotation (
  id TEXT PRIMARY KEY,
  entity_id TEXT NOT NULL REFERENCES entity(id),
  created_at TEXT NOT NULL, author TEXT, tag TEXT, body TEXT NOT NULL,
  snapshot TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX annotation_entity ON annotation(entity_id);
";

/// The `meta` key of the root set that the store belongs to.
const META_ROOT_KEY: &str = "root_key";
/// The `meta` key of the facts input of the last rebinding.
const META_BOUND_INPUT_DIGEST: &str = "bound_input_digest";

/// One forward migration. It runs in the transaction of the open and must not commit.
pub(crate) type Migration = fn(&Transaction<'_>) -> Result<(), ScippyError>;

/// The forward migrations. Entry `n` changes version `n` to version `n + 1`.
/// The list is empty: no format older than version 1 exists.
pub(crate) const MIGRATIONS: &[Migration] = &[];

/// The default path of the identity store: `<db dir>/<db stem>.identity.db`.
pub fn default_identity_path(db: &Path) -> PathBuf {
    let stem = db
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "metalayer".into());
    db.with_file_name(format!("{stem}.identity.db"))
}

// ---- Ids and the clock ----

/// The source of entity ids, annotation ids, and timestamps. Tests use a fixed source.
pub trait IdSource {
    /// A new id as UUIDv7 text.
    fn new_id(&self) -> String;
    /// The current time as `YYYY-MM-DDTHH:MM:SSZ`.
    fn now(&self) -> String;
}

/// The real source: UUIDv7 ids and the system clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemIds;

impl IdSource for SystemIds {
    fn new_id(&self) -> String {
        uuid::Uuid::now_v7().to_string()
    }

    fn now(&self) -> String {
        format_utc(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        )
    }
}

/// A fixed source for tests. Ids count up from 1. The clock never moves.
#[derive(Debug)]
pub struct FixedIds {
    next: AtomicU64,
    time: String,
}

impl FixedIds {
    /// A source whose clock always reads `time`.
    pub fn new(time: &str) -> Self {
        Self::starting_at(1, time)
    }

    /// A source whose ids count up from `start`. Use it when several runs share one store.
    pub fn starting_at(start: u64, time: &str) -> Self {
        Self {
            next: AtomicU64::new(start),
            time: time.into(),
        }
    }
}

impl Default for FixedIds {
    fn default() -> Self {
        Self::new("2026-01-01T00:00:00Z")
    }
}

impl IdSource for FixedIds {
    fn new_id(&self) -> String {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        format!("00000000-0000-7000-8000-{n:012x}")
    }

    fn now(&self) -> String {
        self.time.clone()
    }
}

/// Format Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // The civil date of a day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

// ---- Records ----

/// The family of an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Family {
    Code,
    Note,
}

impl Family {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Note => "note",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "code" => Some(Self::Code),
            "note" => Some(Self::Note),
            _ => None,
        }
    }
}

/// The kind of an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EntityKind {
    Module,
    Class,
    Function,
    Method,
    Variable,
    Parameter,
    Note,
    Heading,
    Block,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Class => "class",
            Self::Function => "function",
            Self::Method => "method",
            Self::Variable => "variable",
            Self::Parameter => "parameter",
            Self::Note => "note",
            Self::Heading => "heading",
            Self::Block => "block",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "module" => Self::Module,
            "class" => Self::Class,
            "function" => Self::Function,
            "method" => Self::Method,
            "variable" => Self::Variable,
            "parameter" => Self::Parameter,
            "note" => Self::Note,
            "heading" => Self::Heading,
            "block" => Self::Block,
            _ => return None,
        })
    }
}

/// The rebinding tier that bound an entity. `METALAYER_IDENTITY.md`, sections 6 and 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Tier {
    T1,
    T1H,
    T2F,
    T2,
    T3,
    T4,
    T5,
    M2,
    M3,
    M4,
    M5,
    Param,
}

impl Tier {
    /// The text in the store, in the CLI, and in the design.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::T1 => "T1",
            Self::T1H => "T1h",
            Self::T2F => "T2f",
            Self::T2 => "T2",
            Self::T3 => "T3",
            Self::T4 => "T4",
            Self::T5 => "T5",
            Self::M2 => "M2",
            Self::M3 => "M3",
            Self::M4 => "M4",
            Self::M5 => "M5",
            Self::Param => "Param",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "T1" => Self::T1,
            "T1h" => Self::T1H,
            "T2f" => Self::T2F,
            "T2" => Self::T2,
            "T3" => Self::T3,
            "T4" => Self::T4,
            "T5" => Self::T5,
            "M2" => Self::M2,
            "M3" => Self::M3,
            "M4" => Self::M4,
            "M5" => Self::M5,
            "Param" => Self::Param,
            _ => return None,
        })
    }
}

/// The binding state of an entity. It is not the outcome of a reference search: see
/// [`StoredOutcome`](crate::StoredOutcome). The store holds a row for each state except `Matched`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BindOutcome {
    Matched,
    Moved,
    Ambiguous,
    Orphaned,
}

impl BindOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::Moved => "moved",
            Self::Ambiguous => "ambiguous",
            Self::Orphaned => "orphaned",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "matched" => Self::Matched,
            "moved" => Self::Moved,
            "ambiguous" => Self::Ambiguous,
            "orphaned" => Self::Orphaned,
            _ => return None,
        })
    }
}

/// One entity: a declaration, a parameter, a note, a heading, or a block.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entity {
    /// UUIDv7 text. Minted once.
    pub id: String,
    pub family: Family,
    pub kind: EntityKind,
    /// The note of a heading or block, the function of a parameter.
    pub parent_id: Option<String>,
    pub created_at: String,
}

/// Where an entity was at one publish.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Locator {
    pub entity_id: String,
    /// 0 is current. A higher number is older. `push_locator` sets it.
    pub seq: i64,
    /// Code: the symbol without the package version. Parameter: the parent id and the name.
    pub key: String,
    pub symbol: Option<String>,
    pub path: String,
    pub kind: String,
    pub name: String,
    /// The display names of the containers, outer to inner, joined by [`CONTAINER_SEPARATOR`].
    pub container: String,
    pub sig_digest: Option<String>,
    pub body_digest: Option<String>,
    pub body_tokens: Option<i64>,
    /// The `input_digest` of the facts that the locator was read from.
    pub input_digest: String,
}

impl Locator {
    /// The display names of the containers, outer to inner.
    pub fn containers(&self) -> Vec<&str> {
        split_container(&self.container)
    }
}

/// Join display names with [`CONTAINER_SEPARATOR`].
pub(crate) fn join_container<S: AsRef<str>>(names: &[S]) -> String {
    let mut out = String::new();
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            out.push(CONTAINER_SEPARATOR);
        }
        out.push_str(name.as_ref());
    }
    out
}

/// Split a container text into display names. An empty text gives no names.
pub(crate) fn split_container(container: &str) -> Vec<&str> {
    if container.is_empty() {
        Vec::new()
    } else {
        container.split(CONTAINER_SEPARATOR).collect()
    }
}

/// The stored state of an entity that is not `matched`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutcomeRow {
    pub entity_id: String,
    /// `Moved`, `Ambiguous`, or `Orphaned`. `set_outcome` refuses `Matched`.
    pub outcome: BindOutcome,
    pub tier: Option<Tier>,
    /// The keys of the candidates.
    pub candidates: Vec<String>,
    /// The `input_digest` of the publish that set the state.
    pub since_digest: String,
}

/// One user annotation on an entity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Annotation {
    pub id: String,
    pub entity_id: String,
    pub created_at: String,
    pub author: Option<String>,
    pub tag: Option<String>,
    pub body: String,
    /// JSON of the locator at creation.
    pub snapshot: String,
}

mod ops;

pub use ops::{
    AcceptAction, AcceptResult, AnnotationEntry, AnnotationFilter, EntityDetail, IdentityError,
    IdentityProbe, LocatorMatch, StatusEntry,
};

// ---- The store ----

/// Options of [`IdentityStore::open`].
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct IdentityOptions {
    /// The root set key of the run. The store binds to the first key. `None` skips the guard.
    pub root_key: Option<String>,
    /// Bind the store to `root_key` when it holds another key. The data stays.
    pub replace_root_set: bool,
}

impl IdentityOptions {
    /// Options that guard the store with `root_key`.
    pub fn with_root_key(root_key: impl Into<String>) -> Self {
        Self {
            root_key: Some(root_key.into()),
            replace_root_set: false,
        }
    }
}

/// The identity store. It holds entities, locators, outcomes, vetoes, and annotations.
pub struct IdentityStore {
    /// The connection. The rebinding engine runs its own transaction on it.
    pub(crate) db: Connection,
    path: PathBuf,
    version: i32,
}

impl fmt::Debug for IdentityStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityStore")
            .field("path", &self.path)
            .field("version", &self.version)
            .finish()
    }
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

fn not_identity() -> ScippyError {
    ScippyError::Storage("the file is not a Scippy identity store".into())
}

/// Refuse a file that is not empty and not an identity store. The check reads only: it must run
/// before any write, so that a foreign file stays as it was.
fn refuse_foreign(found: &Header) -> Result<(), ScippyError> {
    if found.application_id == 0 && found.version == 0 {
        if found.has_tables {
            return Err(ScippyError::Storage(
                "the file is not a Scippy identity store: it has tables and no application id"
                    .into(),
            ));
        }
        Ok(())
    } else if found.application_id != IDENTITY_APPLICATION_ID {
        Err(not_identity())
    } else {
        Ok(())
    }
}

fn newer(found: i32, reads: i32) -> ScippyError {
    ScippyError::Storage(format!(
        "the identity store has format version {found}, but this build reads version {reads}; use a newer scippy"
    ))
}

/// The path of the backup that a migration from `version` writes.
pub(crate) fn backup_path(path: &Path, version: i32) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".bak-v{version}"));
    path.with_file_name(name)
}

impl IdentityStore {
    /// Open or create the identity store.
    ///
    /// A new file gets the tables. A newer version is refused. An older version is copied to
    /// `<name>.bak-v<N>`, then migrated forward in one transaction. The function never drops a
    /// table. A store that belongs to another root set is refused, unless
    /// `options.replace_root_set` is set. Then the store binds to the new root set and keeps its data.
    pub fn open(path: impl AsRef<Path>, options: &IdentityOptions) -> Result<Self, ScippyError> {
        Self::open_with(path.as_ref(), options, IDENTITY_VERSION, MIGRATIONS)
    }

    /// Open the store for reading. It never writes, creates, or migrates the file.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, ScippyError> {
        let path = path.as_ref();
        let db =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(storage)?;
        db.busy_timeout(Duration::from_secs(30)).map_err(storage)?;
        let found = header(&db)?;
        if found.application_id != IDENTITY_APPLICATION_ID {
            return Err(not_identity());
        }
        if found.version > IDENTITY_VERSION {
            return Err(newer(found.version, IDENTITY_VERSION));
        }
        if found.version < IDENTITY_VERSION {
            return Err(ScippyError::Storage(format!(
                "the identity store has format version {}, but this build reads version {IDENTITY_VERSION}; run a command that writes the store to migrate it",
                found.version
            )));
        }
        Ok(Self {
            db,
            path: path.into(),
            version: found.version,
        })
    }

    /// The open with a chosen target version and migration list. Tests use fake lists.
    pub(crate) fn open_with(
        path: &Path,
        options: &IdentityOptions,
        target: i32,
        steps: &[Migration],
    ) -> Result<Self, ScippyError> {
        let mut db = Connection::open(path).map_err(storage)?;
        db.busy_timeout(Duration::from_secs(30)).map_err(storage)?;
        // Check the application id first. The WAL pragma writes the header of the file.
        refuse_foreign(&header(&db)?)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=OFF;")
            .map_err(storage)?;

        // Back up before the write lock: copy a complete file.
        let seen = header(&db)?;
        if seen.application_id == IDENTITY_APPLICATION_ID && seen.version < target {
            db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .map_err(storage)?;
            let backup = backup_path(path, seen.version);
            std::fs::copy(path, &backup).map_err(|source| ScippyError::Io {
                path: backup,
                source,
            })?;
        }

        // An immediate transaction makes concurrent first opens take turns.
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let found = header(&tx)?;
        let mut version = found.version;
        // The file can change between the first check and the lock. Check again.
        refuse_foreign(&found)?;
        if found.application_id == 0 && found.version == 0 {
            tx.execute_batch(IDENTITY_DDL).map_err(storage)?;
            tx.execute_batch(&format!(
                "PRAGMA application_id={IDENTITY_APPLICATION_ID}; PRAGMA user_version={target};"
            ))
            .map_err(storage)?;
            version = target;
        } else if found.version > target {
            return Err(newer(found.version, target));
        } else {
            while version < target {
                let step = usize::try_from(version)
                    .ok()
                    .and_then(|n| steps.get(n))
                    .ok_or_else(|| {
                        ScippyError::Storage(format!(
                            "no migration from identity store version {version}"
                        ))
                    })?;
                step(&tx)?;
                version += 1;
            }
            if version != found.version {
                tx.execute_batch(&format!("PRAGMA user_version={version};"))
                    .map_err(storage)?;
            }
        }
        if let Some(key) = &options.root_key {
            guard_root_set(&tx, key, options.replace_root_set)?;
        }
        tx.commit().map_err(storage)?;
        db.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(storage)?;
        Ok(Self {
            db,
            path: path.into(),
            version,
        })
    }

    /// The path of the store file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The format version of the open store.
    pub fn version(&self) -> i32 {
        self.version
    }
}

/// Bind a store to a root set, or refuse a store that holds another one.
fn guard_root_set(tx: &Transaction<'_>, key: &str, replace: bool) -> Result<(), ScippyError> {
    let stored: Option<String> = tx
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [META_ROOT_KEY],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    match stored {
        Some(stored) if stored == key => Ok(()),
        Some(stored) if !replace => Err(ScippyError::RootSetMismatch {
            stored,
            requested: key.into(),
        }),
        _ => {
            tx.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![META_ROOT_KEY, key],
            )
            .map_err(storage)?;
            Ok(())
        }
    }
}

// ---- Helpers for the rebinding and annotation lanes ----

fn entity_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entity> {
    let family: String = r.get(1)?;
    let kind: String = r.get(2)?;
    let bad = |what: &str, text: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown {what} {text:?}").into(),
        )
    };
    Ok(Entity {
        id: r.get(0)?,
        family: Family::parse(&family).ok_or_else(|| bad("family", &family))?,
        kind: EntityKind::parse(&kind).ok_or_else(|| bad("kind", &kind))?,
        parent_id: r.get(3)?,
        created_at: r.get(4)?,
    })
}

const LOCATOR_COLUMNS: &str = "entity_id, seq, key, symbol, path, kind, name, container, \
                               sig_digest, body_digest, body_tokens, input_digest";

fn locator_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Locator> {
    Ok(Locator {
        entity_id: r.get(0)?,
        seq: r.get(1)?,
        key: r.get(2)?,
        symbol: r.get(3)?,
        path: r.get(4)?,
        kind: r.get(5)?,
        name: r.get(6)?,
        container: r.get(7)?,
        sig_digest: r.get(8)?,
        body_digest: r.get(9)?,
        body_tokens: r.get(10)?,
        input_digest: r.get(11)?,
    })
}

fn annotation_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Annotation> {
    Ok(Annotation {
        id: r.get(0)?,
        entity_id: r.get(1)?,
        created_at: r.get(2)?,
        author: r.get(3)?,
        tag: r.get(4)?,
        body: r.get(5)?,
        snapshot: r.get(6)?,
    })
}

/// The body of [`IdentityStore::push_locator`], for a caller that holds a transaction.
pub(crate) fn push_locator_in(tx: &Transaction<'_>, locator: &Locator) -> Result<(), ScippyError> {
    // Two steps keep the primary key free of clashes while the seq numbers move.
    tx.execute(
        "UPDATE locator SET seq = -(seq + 1) WHERE entity_id = ?1",
        [&locator.entity_id],
    )
    .map_err(storage)?;
    tx.execute(
        "UPDATE locator SET seq = -seq WHERE entity_id = ?1",
        [&locator.entity_id],
    )
    .map_err(storage)?;
    tx.execute(
        "DELETE FROM locator WHERE entity_id = ?1 AND seq >= ?2",
        params![locator.entity_id, LOCATOR_HISTORY as i64],
    )
    .map_err(storage)?;
    tx.execute(
        &format!(
            "INSERT INTO locator ({LOCATOR_COLUMNS})
             VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
        ),
        params![
            locator.entity_id,
            locator.key,
            locator.symbol,
            locator.path,
            locator.kind,
            locator.name,
            locator.container,
            locator.sig_digest,
            locator.body_digest,
            locator.body_tokens,
            locator.input_digest
        ],
    )
    .map_err(storage)?;
    Ok(())
}

// The annotation API calls most of these helpers. The rebinding code calls the rest.
impl IdentityStore {
    /// Read one `meta` value.
    pub(crate) fn meta(&self, key: &str) -> Result<Option<String>, ScippyError> {
        self.db
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(storage)
    }

    #[allow(dead_code)]
    /// Write one `meta` value.
    pub(crate) fn set_meta(&mut self, key: &str, value: &str) -> Result<(), ScippyError> {
        self.db
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(storage)?;
        Ok(())
    }

    #[allow(dead_code)]
    /// The root set key that binds the store.
    pub(crate) fn root_key(&self) -> Result<Option<String>, ScippyError> {
        self.meta(META_ROOT_KEY)
    }

    /// The facts `input_digest` of the last rebinding. `None` before the first rebinding.
    pub fn bound_input_digest(&self) -> Result<Option<String>, ScippyError> {
        self.meta(META_BOUND_INPUT_DIGEST)
    }

    #[allow(dead_code)]
    /// Record the facts `input_digest` of the last rebinding.
    pub(crate) fn set_bound_input_digest(&mut self, digest: &str) -> Result<(), ScippyError> {
        self.set_meta(META_BOUND_INPUT_DIGEST, digest)
    }

    #[allow(dead_code)]
    /// Add an entity. An existing id is an error: an id is minted once.
    pub(crate) fn insert_entity(&mut self, entity: &Entity) -> Result<(), ScippyError> {
        self.db
            .execute(
                "INSERT INTO entity (id, family, kind, parent_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    entity.id,
                    entity.family.as_str(),
                    entity.kind.as_str(),
                    entity.parent_id,
                    entity.created_at
                ],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// Read one entity.
    pub(crate) fn get_entity(&self, id: &str) -> Result<Option<Entity>, ScippyError> {
        self.db
            .query_row(
                "SELECT id, family, kind, parent_id, created_at FROM entity WHERE id = ?1",
                [id],
                entity_row,
            )
            .optional()
            .map_err(storage)
    }

    /// Make `locator` the current locator of its entity (seq 0). Older locators move up by
    /// one. The store keeps the newest [`LOCATOR_HISTORY`]. The `seq` of the argument is ignored.
    #[allow(dead_code)]
    pub(crate) fn push_locator(&mut self, locator: &Locator) -> Result<(), ScippyError> {
        let tx = self.db.transaction().map_err(storage)?;
        push_locator_in(&tx, locator)?;
        tx.commit().map_err(storage)
    }

    /// The locators of an entity, current first.
    pub(crate) fn locators(&self, entity_id: &str) -> Result<Vec<Locator>, ScippyError> {
        let mut statement = self
            .db
            .prepare(&format!(
                "SELECT {LOCATOR_COLUMNS} FROM locator WHERE entity_id = ?1 ORDER BY seq"
            ))
            .map_err(storage)?;
        let rows = statement
            .query_map([entity_id], locator_row)
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// The current locator of an entity (seq 0).
    pub(crate) fn current_locator(&self, entity_id: &str) -> Result<Option<Locator>, ScippyError> {
        self.db
            .query_row(
                &format!("SELECT {LOCATOR_COLUMNS} FROM locator WHERE entity_id = ?1 AND seq = 0"),
                [entity_id],
                locator_row,
            )
            .optional()
            .map_err(storage)
    }

    #[allow(dead_code)]
    /// Set the outcome of an entity. An existing row is replaced. `Matched` has no row:
    /// use [`IdentityStore::clear_outcome`].
    pub(crate) fn set_outcome(&mut self, row: &OutcomeRow) -> Result<(), ScippyError> {
        if row.outcome == BindOutcome::Matched {
            return Err(ScippyError::Storage(
                "a matched entity has no outcome row".into(),
            ));
        }
        let candidates = if row.candidates.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&row.candidates)
                    .map_err(|e| ScippyError::Serialize(e.to_string()))?,
            )
        };
        self.db
            .execute(
                "INSERT INTO outcome (entity_id, outcome, tier, candidates, since_digest)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(entity_id) DO UPDATE SET outcome = excluded.outcome,
                   tier = excluded.tier, candidates = excluded.candidates,
                   since_digest = excluded.since_digest",
                params![
                    row.entity_id,
                    row.outcome.as_str(),
                    row.tier.map(Tier::as_str),
                    candidates,
                    row.since_digest
                ],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// Remove the outcome row of an entity: the entity is `matched` again.
    pub(crate) fn clear_outcome(&mut self, entity_id: &str) -> Result<(), ScippyError> {
        self.db
            .execute("DELETE FROM outcome WHERE entity_id = ?1", [entity_id])
            .map_err(storage)?;
        Ok(())
    }

    /// The outcome row of an entity. `None` means `matched`.
    pub(crate) fn get_outcome(&self, entity_id: &str) -> Result<Option<OutcomeRow>, ScippyError> {
        let row = self
            .db
            .query_row(
                "SELECT entity_id, outcome, tier, candidates, since_digest
                 FROM outcome WHERE entity_id = ?1",
                [entity_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(storage)?;
        let Some((entity_id, outcome, tier, candidates, since_digest)) = row else {
            return Ok(None);
        };
        let outcome = BindOutcome::parse(&outcome)
            .ok_or_else(|| ScippyError::Storage(format!("unknown outcome {outcome:?}")))?;
        let tier = match tier {
            Some(text) => Some(
                Tier::parse(&text)
                    .ok_or_else(|| ScippyError::Storage(format!("unknown tier {text:?}")))?,
            ),
            None => None,
        };
        let candidates = match candidates {
            Some(text) => serde_json::from_str(&text)
                .map_err(|e| ScippyError::Storage(format!("bad candidates: {e}")))?,
            None => Vec::new(),
        };
        Ok(Some(OutcomeRow {
            entity_id,
            outcome,
            tier,
            candidates,
            since_digest,
        }))
    }

    #[allow(dead_code)]
    /// Forbid a binding of an entity to a key. Repeating a veto does nothing.
    pub(crate) fn add_veto(&mut self, entity_id: &str, key: &str) -> Result<(), ScippyError> {
        self.db
            .execute(
                "INSERT OR IGNORE INTO veto (entity_id, key) VALUES (?1, ?2)",
                params![entity_id, key],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// The keys that an entity must not bind to, sorted.
    pub(crate) fn vetoes(&self, entity_id: &str) -> Result<Vec<String>, ScippyError> {
        let mut statement = self
            .db
            .prepare("SELECT key FROM veto WHERE entity_id = ?1 ORDER BY key")
            .map_err(storage)?;
        let rows = statement
            .query_map([entity_id], |r| r.get(0))
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// Add an annotation. The entity must exist.
    pub(crate) fn insert_annotation(&mut self, annotation: &Annotation) -> Result<(), ScippyError> {
        self.db
            .execute(
                "INSERT INTO annotation (id, entity_id, created_at, author, tag, body, snapshot)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    annotation.id,
                    annotation.entity_id,
                    annotation.created_at,
                    annotation.author,
                    annotation.tag,
                    annotation.body,
                    annotation.snapshot
                ],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// Read one annotation.
    pub(crate) fn get_annotation(&self, id: &str) -> Result<Option<Annotation>, ScippyError> {
        self.db
            .query_row(
                "SELECT id, entity_id, created_at, author, tag, body, snapshot
                 FROM annotation WHERE id = ?1",
                [id],
                annotation_row,
            )
            .optional()
            .map_err(storage)
    }

    /// The annotations of an entity, oldest first (`created_at`, then `id`).
    pub(crate) fn list_annotations(&self, entity_id: &str) -> Result<Vec<Annotation>, ScippyError> {
        let mut statement = self
            .db
            .prepare(
                "SELECT id, entity_id, created_at, author, tag, body, snapshot
                 FROM annotation WHERE entity_id = ?1 ORDER BY created_at, id",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map([entity_id], annotation_row)
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// Delete one annotation. The result is `false` when the id does not exist.
    pub(crate) fn delete_annotation(&mut self, id: &str) -> Result<bool, ScippyError> {
        let n = self
            .db
            .execute("DELETE FROM annotation WHERE id = ?1", [id])
            .map_err(storage)?;
        Ok(n > 0)
    }
}
