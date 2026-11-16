//! A disposable cache of file graphs and their partial paths.
//!
//! A row holds a graph in the `bincode` format, compressed with `zstd`. The cache keeps rows for
//! the running executable only, and it keeps its file under a byte cap. A row has an owner: a
//! file of a project. A new row of an owner replaces the older row of that owner.

use std::cell::RefCell;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use sha2::{Digest, Sha256};

/// The default byte cap of the cache file. A cache of the measured projects holds about 9 times
/// the source size, so 1 GiB fits about 100 MB of source code.
pub const DEFAULT_MAX_BYTES: u64 = 1 << 30;

/// The `zstd` level of a stored row. A low level is fast and still shrinks the row a lot.
const ZSTD_LEVEL: i32 = 1;

/// A shard writes its new rows in one transaction when they reach this many bytes, and when
/// it ends.
const FLUSH_BYTES: usize = 32 << 20;

#[derive(bincode::Encode, bincode::Decode)]
pub(crate) struct CachedGraph {
    pub graph: stack_graphs::serde::StackGraph,
    pub paths: Vec<stack_graphs::serde::PartialPath>,
}

/// The run-wide state of the cache. All shards share it. A failure turns the cache off for the
/// rest of the run, and the run goes on without it. When the last owner drops the gate, the
/// run ends: the gate then keeps the cache file under its byte cap.
pub(crate) struct CacheGate {
    path: Option<PathBuf>,
    /// The canonical project root. Projects can share one cache file, so it is part of each owner.
    root: String,
    timeout: Duration,
    /// The byte cap of the cache file. Zero means no cap.
    max_bytes: u64,
    off: AtomicBool,
    /// The number of failures that the gate saw. A lock timeout is one failure, and it costs
    /// one wait of at most `timeout`.
    failures: AtomicUsize,
    /// The number of this run. It is zero until the cache file is ready.
    run: AtomicI64,
    prepared: OnceLock<()>,
    #[cfg(test)]
    exe: Option<&'static str>,
}

impl CacheGate {
    pub fn new(path: Option<&Path>, root: &Path, timeout: Duration, max_bytes: u64) -> Self {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        Self {
            path: path.map(Path::to_path_buf),
            root: root.to_string_lossy().into_owned(),
            timeout,
            max_bytes,
            off: AtomicBool::new(false),
            failures: AtomicUsize::new(0),
            run: AtomicI64::new(0),
            prepared: OnceLock::new(),
            #[cfg(test)]
            exe: None,
        }
    }

    /// Act as another executable. Tests use it to check the pruning.
    #[cfg(test)]
    fn with_exe(mut self, exe: &'static str) -> Self {
        self.exe = Some(exe);
        self
    }

    fn exe(&self) -> Result<&'static str, String> {
        #[cfg(test)]
        if let Some(exe) = self.exe {
            return Ok(exe);
        }
        executable_hash()
    }

    /// The number of failures so far. Each failure ends one wait for the cache file.
    pub fn failures(&self) -> usize {
        self.failures.load(Ordering::SeqCst)
    }

    pub fn is_on(&self) -> bool {
        self.path.is_some() && !self.off.load(Ordering::SeqCst)
    }

    /// Turn the cache off and print one warning for the whole run.
    pub fn fail(&self, error: &str) {
        self.failures.fetch_add(1, Ordering::SeqCst);
        if !self.off.swap(true, Ordering::SeqCst) {
            let path = self.path.as_deref().unwrap_or(Path::new(""));
            eprintln!(
                "warning: the graph cache {} is off for this run: {error}",
                path.display()
            );
        }
    }

    /// Open a connection for one shard. Return `None` when the cache is off.
    pub fn open(self: &Arc<Self>) -> Option<GraphCache> {
        let path = self.path.as_deref()?;
        self.prepared.get_or_init(|| self.prepare(path));
        if !self.is_on() {
            return None;
        }
        let exe = match self.exe() {
            Ok(exe) => exe,
            Err(error) => {
                self.fail(&error);
                return None;
            }
        };
        let opened = Connection::open(path).and_then(|db| {
            db.busy_timeout(self.timeout)?;
            // The cache can be rebuilt. A power loss can lose the newest rows, and it cannot
            // damage the file in WAL mode. So NORMAL is safe enough, and it avoids a sync
            // for each commit.
            db.pragma_update(None, "synchronous", "NORMAL")?;
            Ok(db)
        });
        match opened {
            Ok(db) => Some(GraphCache {
                db,
                exe,
                run: self.run.load(Ordering::SeqCst),
                gate: Arc::clone(self),
                pending: RefCell::default(),
            }),
            Err(error) => {
                self.fail(&error.to_string());
                None
            }
        }
    }

    /// Run once for the whole run: hash the executable, create the tables, drop the rows of
    /// other executables, and replace a damaged file.
    fn prepare(&self, path: &Path) {
        let exe = match self.exe() {
            Ok(exe) => exe,
            Err(error) => return self.fail(&error),
        };
        let mut created = create(path, self.timeout, exe);
        if let Err(rusqlite::Error::SqliteFailure(failure, _)) = &created
            && matches!(
                failure.code,
                ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt
            )
        {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            if let Err(error) = std::fs::rename(path, &bad) {
                return self.fail(&format!("it is damaged and the rename fails: {error}"));
            }
            eprintln!(
                "warning: the graph cache {} is damaged: moved it to {}",
                path.display(),
                Path::new(&bad).display()
            );
            created = create(path, self.timeout, exe);
        }
        match created {
            Ok(run) => self.run.store(run, Ordering::SeqCst),
            Err(error) => self.fail(&error.to_string()),
        }
    }
}

impl Drop for CacheGate {
    fn drop(&mut self) {
        let Some(path) = &self.path else { return };
        if self.run.load(Ordering::SeqCst) == 0 || !self.is_on() {
            return;
        }
        if let Err(error) = shrink(path, self.timeout, self.max_bytes) {
            eprintln!(
                "warning: the graph cache {} could not shrink: {error}",
                path.display()
            );
        }
    }
}

/// Create the tables and return the number of this run.
///
/// The cache keeps the rows of one executable. This function deletes the rows of the others,
/// and the old `file_graph_v2` and `file_graph_v3` tables.
fn create(path: &Path, timeout: Duration, exe: &str) -> rusqlite::Result<i64> {
    let db = Connection::open(path)?;
    db.busy_timeout(timeout)?;
    // WAL mode lets the shards read while one shard writes. The mode stays in the file.
    db.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0))?;
    db.execute_batch(
        "DROP TABLE IF EXISTS file_graph_v2; \
         DROP TABLE IF EXISTS file_graph_v3; \
         DROP TABLE IF EXISTS file_graph_use_v3; \
         CREATE TABLE IF NOT EXISTS file_graph_v4 \
         (key TEXT PRIMARY KEY, value BLOB NOT NULL, sum TEXT NOT NULL); \
         CREATE TABLE IF NOT EXISTS file_graph_use_v4 \
         (key TEXT PRIMARY KEY, exe TEXT NOT NULL, last_used INTEGER NOT NULL, \
          bytes INTEGER NOT NULL, owner TEXT NOT NULL); \
         CREATE INDEX IF NOT EXISTS file_graph_use_v4_last_used \
         ON file_graph_use_v4 (last_used); \
         CREATE INDEX IF NOT EXISTS file_graph_use_v4_owner \
         ON file_graph_use_v4 (owner)",
    )?;
    // An incremental vacuum returns the pages of deleted rows to the file system without a
    // rewrite of the file. A plain `VACUUM` copies the whole file and needs as much free disk
    // space as the file. The mode takes effect after one `VACUUM`, which is cheap here: the old
    // table is gone, and a new file is empty.
    let mode: i64 = db.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
    if mode != 2 {
        db.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM")?;
    }
    db.execute("DELETE FROM file_graph_use_v4 WHERE exe <> ?1", [exe])?;
    db.execute(
        "DELETE FROM file_graph_v4 WHERE key NOT IN (SELECT key FROM file_graph_use_v4)",
        [],
    )?;
    db.query_row(
        "SELECT COALESCE(MAX(last_used), 0) + 1 FROM file_graph_use_v4",
        [],
        |r| r.get(0),
    )
}

/// Delete the least recently used rows while the live bytes of the file exceed `max_bytes`.
/// Then return the freed pages to the file system.
fn shrink(path: &Path, timeout: Duration, max_bytes: u64) -> rusqlite::Result<()> {
    let db = Connection::open(path)?;
    db.busy_timeout(timeout)?;
    let pragma = |name: &str| db.query_row(&format!("PRAGMA {name}"), [], |r| r.get::<_, i64>(0));
    let page_size = pragma("page_size")?;
    let cap = i64::try_from(max_bytes).unwrap_or(i64::MAX);
    // Whole pages of other rows stay partly empty, so one pass can free less than it should.
    for _ in 0..4 {
        let live = (pragma("page_count")? - pragma("freelist_count")?) * page_size;
        if cap == 0 || live <= cap {
            break;
        }
        delete_oldest(&db, live - cap)?;
    }
    // A small free list costs less to keep than to move pages.
    let pages = pragma("page_count")?;
    if pragma("freelist_count")? * 10 > pages {
        // SQLite frees one page for each step of the statement.
        let mut vacuum = db.prepare("PRAGMA incremental_vacuum")?;
        let mut steps = vacuum.query([])?;
        while steps.next()?.is_some() {}
    }
    // Move the WAL into the file. Only then does the file get its smaller size.
    db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
}

/// Delete the oldest rows until they hold at least `excess` bytes.
fn delete_oldest(db: &Connection, excess: i64) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    let mut keys = Vec::new();
    {
        let mut rows =
            tx.prepare("SELECT key, bytes FROM file_graph_use_v4 ORDER BY last_used, key")?;
        let mut rows = rows.query([])?;
        let mut freed = 0;
        while freed < excess
            && let Some(row) = rows.next()?
        {
            keys.push(row.get::<_, String>(0)?);
            freed += row.get::<_, i64>(1)?;
        }
    }
    for key in &keys {
        tx.execute("DELETE FROM file_graph_v4 WHERE key = ?1", [key])?;
        tx.execute("DELETE FROM file_graph_use_v4 WHERE key = ?1", [key])?;
    }
    tx.commit()
}

/// The rows and the hits that a shard has not written yet.
#[derive(Default)]
struct Pending {
    /// The key, the owner, the stored value, and the checksum of each new row.
    rows: Vec<(String, String, Vec<u8>, String)>,
    bytes: usize,
    /// The keys of the rows that this shard read.
    touched: Vec<String>,
}

pub(crate) struct GraphCache {
    db: Connection,
    exe: &'static str,
    run: i64,
    gate: Arc<CacheGate>,
    pending: RefCell<Pending>,
}

impl Drop for GraphCache {
    /// Write the last rows. A failure turns the cache off.
    fn drop(&mut self) {
        if self.gate.is_on()
            && let Err(error) = self.flush()
        {
            self.gate.fail(&error);
        }
    }
}

/// The checksum covers the key and the value. A row that moves to another key fails it.
fn checksum(key: &str, value: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update((key.len() as u64).to_le_bytes());
    hash.update(key.as_bytes());
    hash.update(value);
    format!("{:x}", hash.finalize())
}

/// Encode and compress in one pass. The raw encoding never sits in memory as a whole.
pub(crate) fn encode(value: &CachedGraph) -> Result<Vec<u8>, String> {
    let mut writer =
        zstd::stream::Encoder::new(Vec::new(), ZSTD_LEVEL).map_err(|e| e.to_string())?;
    bincode::encode_into_std_write(value, &mut writer, bincode::config::standard())
        .map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

pub(crate) fn decode(bytes: &[u8]) -> Result<CachedGraph, String> {
    let mut reader = zstd::stream::Decoder::new(bytes).map_err(|e| e.to_string())?;
    bincode::decode_from_std_read(&mut reader, bincode::config::standard())
        .map_err(|e| e.to_string())
}

impl GraphCache {
    /// Return `Ok(None)` for a missing row and for a row that fails its checksum. The failed
    /// row is deleted.
    pub fn get(&self, key: &str) -> Result<Option<CachedGraph>, String> {
        let row: Option<(Value, Value)> = self
            .db
            .query_row(
                "SELECT value, sum FROM file_graph_v4 WHERE key = ?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((value, sum)) = row else {
            return Ok(None);
        };
        match (value, sum) {
            (Value::Blob(bytes), Value::Text(sum)) if sum == checksum(key, &bytes) => {
                let graph = decode(&bytes)?;
                self.pending.borrow_mut().touched.push(key.to_owned());
                Ok(Some(graph))
            }
            _ => {
                for table in ["file_graph_v4", "file_graph_use_v4"] {
                    self.db
                        .execute(&format!("DELETE FROM {table} WHERE key = ?1"), [key])
                        .map_err(|e| e.to_string())?;
                }
                Ok(None)
            }
        }
    }

    /// Keep the row in memory. The cache writes the rows of a shard in few transactions. The
    /// write deletes the older row of the same owner.
    pub fn put(&self, key: &str, owner: &str, value: &CachedGraph) -> Result<(), String> {
        let bytes = encode(value)?;
        let sum = checksum(key, &bytes);
        let full = {
            let mut pending = self.pending.borrow_mut();
            pending.bytes += bytes.len();
            pending
                .rows
                .push((key.to_owned(), owner.to_owned(), bytes, sum));
            pending.bytes >= FLUSH_BYTES
        };
        if full { self.flush() } else { Ok(()) }
    }

    /// Write the new rows and the use of the read rows in one transaction.
    fn flush(&self) -> Result<(), String> {
        let pending = std::mem::take(&mut *self.pending.borrow_mut());
        if pending.rows.is_empty() && pending.touched.is_empty() {
            return Ok(());
        }
        let write = || -> rusqlite::Result<()> {
            let tx = self.db.unchecked_transaction()?;
            for (key, owner, bytes, sum) in &pending.rows {
                // Delete the superseded row first. It belongs to the same owner and executable.
                tx.prepare_cached(
                    "DELETE FROM file_graph_v4 WHERE key IN (SELECT key FROM file_graph_use_v4 \
                     WHERE owner = ?1 AND exe = ?2 AND key <> ?3)",
                )?
                .execute(params![owner, self.exe, key])?;
                tx.prepare_cached(
                    "DELETE FROM file_graph_use_v4 WHERE owner = ?1 AND exe = ?2 AND key <> ?3",
                )?
                .execute(params![owner, self.exe, key])?;
                tx.prepare_cached("INSERT OR REPLACE INTO file_graph_v4 VALUES (?1, ?2, ?3)")?
                    .execute(params![key, bytes, sum])?;
                tx.prepare_cached(
                    "INSERT OR REPLACE INTO file_graph_use_v4 VALUES (?1, ?2, ?3, ?4, ?5)",
                )?
                .execute(params![
                    key,
                    self.exe,
                    self.run,
                    bytes.len() as i64,
                    owner
                ])?;
            }
            for key in &pending.touched {
                tx.prepare_cached("UPDATE file_graph_use_v4 SET last_used = ?1 WHERE key = ?2")?
                    .execute(params![self.run, key])?;
            }
            tx.commit()
        };
        write().map_err(|e| e.to_string())
    }

    /// Return the key of a row and its owner.
    #[allow(clippy::too_many_arguments)]
    pub fn key(
        &self,
        pack: &str,
        pack_key: &str,
        path: &str,
        source: &str,
        globals: &[(String, String)],
        dialect: usize,
        shadow_aware: bool,
    ) -> Result<(String, String), String> {
        let input = serde_json::to_vec(&(
            "file-graph-v4",
            self.exe,
            pack,
            pack_key,
            path,
            source,
            globals,
            dialect,
            shadow_aware,
        ))
        .map_err(|e| e.to_string())?;
        // The owner has no source and no globals. It is the same for every edit of a file.
        let owner = serde_json::to_vec(&("owner", &self.gate.root, pack, path, dialect))
            .map_err(|e| e.to_string())?;
        Ok((
            format!("{:x}", Sha256::digest(input)),
            format!("{:x}", Sha256::digest(owner)),
        ))
    }
}

/// Include the whole executable because pack rules and grammars are compiled into it.
pub(crate) fn executable_hash() -> Result<&'static str, String> {
    static HASH: OnceLock<Result<String, String>> = OnceLock::new();
    HASH.get_or_init(|| {
        let path = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buf = [0; 65536];
        loop {
            let n = file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
        }
        Ok(format!("{:x}", hash.finalize()))
    })
    .as_deref()
    .map_err(Clone::clone)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(5);

    fn gate(path: &Path, exe: &'static str, max_bytes: u64) -> Arc<CacheGate> {
        project_gate(path, Path::new("/project-a"), exe, max_bytes)
    }

    fn project_gate(path: &Path, root: &Path, exe: &'static str, max_bytes: u64) -> Arc<CacheGate> {
        Arc::new(CacheGate::new(Some(path), root, TIMEOUT, max_bytes).with_exe(exe))
    }

    /// A graph with one file and no nodes. The name tells the rows apart.
    fn graph(name: &str) -> CachedGraph {
        let mut graph = stack_graphs::graph::StackGraph::new();
        graph.get_or_create_file(name);
        CachedGraph {
            graph: stack_graphs::serde::StackGraph::from_graph(&graph),
            paths: Vec::new(),
        }
    }

    fn count(path: &Path, table: &str) -> i64 {
        Connection::open(path)
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// Add a row of `size` bytes that no `get` can read. The tests use it to fill the file.
    fn fill(path: &Path, exe: &str, key: &str, run: i64, size: usize) {
        let db = Connection::open(path).unwrap();
        let bytes: Vec<u8> = (0..size).map(|n| (n * 31 + run as usize) as u8).collect();
        db.execute(
            "INSERT INTO file_graph_v4 VALUES (?1, ?2, ?3)",
            params![key, bytes, checksum(key, &bytes)],
        )
        .unwrap();
        db.execute(
            "INSERT INTO file_graph_use_v4 VALUES (?1, ?2, ?3, ?4, ?5)",
            params![key, exe, run, size as i64, key],
        )
        .unwrap();
    }

    #[test]
    fn a_row_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graphs.sqlite");
        let gate = gate(&path, "exe-a", 0);
        {
            let cache = gate.open().unwrap();
            assert!(cache.get("k").unwrap().is_none());
            cache.put("k", "k", &graph("a.py")).unwrap();
        }
        let cache = gate.open().unwrap();
        let got = cache.get("k").unwrap().unwrap();
        assert_eq!(got.graph.files.data, ["a.py"]);
        assert!(gate.is_on());
    }

    #[test]
    fn open_deletes_the_rows_of_other_executables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graphs.sqlite");
        {
            let old = gate(&path, "exe-a", 0);
            let cache = old.open().unwrap();
            cache.put("one", "one", &graph("a.py")).unwrap();
            cache.put("two", "two", &graph("b.py")).unwrap();
        }
        assert_eq!(count(&path, "file_graph_v4"), 2);
        let new = gate(&path, "exe-b", 0);
        let cache = new.open().unwrap();
        assert_eq!(count(&path, "file_graph_v4"), 0);
        assert_eq!(count(&path, "file_graph_use_v4"), 0);
        assert!(cache.get("one").unwrap().is_none());
    }

    #[test]
    fn a_new_run_keeps_the_rows_of_the_same_executable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graphs.sqlite");
        for _ in 0..2 {
            let gate = gate(&path, "exe-a", 0);
            let cache = gate.open().unwrap();
            cache.put("one", "one", &graph("a.py")).unwrap();
        }
        assert_eq!(count(&path, "file_graph_v4"), 1);
    }

    #[test]
    fn the_cap_deletes_the_least_recently_used_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graphs.sqlite");
        // The first gate makes the file. Its last drop shrinks the file to the cap.
        let cap = 1 << 20;
        let first = gate(&path, "exe-a", cap);
        let cache = first.open().unwrap();
        for run in 1..=100 {
            fill(&path, "exe-a", &format!("row{run:03}"), run, 20_000);
        }
        drop(cache);
        drop(first);
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size <= cap, "the file has {size} bytes");
        let db = Connection::open(&path).unwrap();
        let keys: Vec<String> = db
            .prepare("SELECT key FROM file_graph_use_v4 ORDER BY key")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        // The rows of the oldest runs are gone. The newest row stays.
        assert!(!keys.is_empty() && keys.len() < 100);
        assert_eq!(keys.last().unwrap(), "row100");
        let oldest = keys.len();
        assert_eq!(keys[0], format!("row{:03}", 100 - oldest + 1));
        assert_eq!(count(&path, "file_graph_v4"), keys.len() as i64);
    }

    #[test]
    fn a_new_key_replaces_the_row_of_the_same_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graphs.sqlite");
        let gate = gate(&path, "exe-a", 0);
        {
            let cache = gate.open().unwrap();
            cache.put("edit1", "file-a", &graph("a.py")).unwrap();
            cache.put("other", "file-b", &graph("b.py")).unwrap();
        }
        {
            // A later run edits the file twice. Each put replaces the row before it.
            let cache = gate.open().unwrap();
            cache.put("edit2", "file-a", &graph("a.py")).unwrap();
        }
        {
            let cache = gate.open().unwrap();
            cache.put("edit3", "file-a", &graph("a.py")).unwrap();
            assert!(cache.get("edit2").unwrap().is_some());
        }
        assert_eq!(count(&path, "file_graph_v4"), 2);
        assert_eq!(count(&path, "file_graph_use_v4"), 2);
        let cache = gate.open().unwrap();
        assert!(cache.get("edit1").unwrap().is_none());
        assert!(cache.get("edit2").unwrap().is_none());
        assert!(cache.get("edit3").unwrap().is_some());
        assert!(cache.get("other").unwrap().is_some());
    }

    #[test]
    fn projects_that_share_a_cache_keep_their_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graphs.sqlite");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let key = |gate: &Arc<CacheGate>, source: &str| {
            gate.open()
                .unwrap()
                .key("python", "pack", "m.py", source, &[], 0, false)
                .unwrap()
        };
        let (gate_a, gate_b) = (
            project_gate(&path, &a, "exe-a", 0),
            project_gate(&path, &b, "exe-a", 0),
        );
        let (key_a1, owner_a) = key(&gate_a, "x = 1");
        let (key_b1, owner_b) = key(&gate_b, "x = 2");
        assert_ne!(owner_a, owner_b);
        for (gate, key, owner) in [(&gate_a, &key_a1, &owner_a), (&gate_b, &key_b1, &owner_b)] {
            gate.open()
                .unwrap()
                .put(key, owner, &graph("m.py"))
                .unwrap();
        }
        // Project A edits the same relative path.
        let (key_a2, owner_a2) = key(&gate_a, "x = 3");
        assert_eq!(owner_a, owner_a2);
        gate_a
            .open()
            .unwrap()
            .put(&key_a2, &owner_a2, &graph("m.py"))
            .unwrap();
        assert_eq!(count(&path, "file_graph_v4"), 2);
        let cache = gate_b.open().unwrap();
        assert!(cache.get(&key_a1).unwrap().is_none());
        assert!(cache.get(&key_a2).unwrap().is_some());
        assert!(cache.get(&key_b1).unwrap().is_some());
    }

    #[test]
    fn a_read_row_counts_as_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graphs.sqlite");
        {
            let gate = gate(&path, "exe-a", 0);
            let cache = gate.open().unwrap();
            cache.put("old", "old", &graph("a.py")).unwrap();
            cache.put("other", "other", &graph("b.py")).unwrap();
        }
        let last_used = |key: &str| -> i64 {
            Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT last_used FROM file_graph_use_v4 WHERE key = ?1",
                    [key],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(last_used("old"), 1);
        {
            let gate = gate(&path, "exe-a", 0);
            let cache = gate.open().unwrap();
            assert!(cache.get("old").unwrap().is_some());
        }
        assert_eq!(last_used("old"), 2);
        assert_eq!(last_used("other"), 1);
    }

    #[test]
    fn a_new_file_has_incremental_vacuum_and_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graphs.sqlite");
        let _cache = gate(&path, "exe-a", 0).open().unwrap();
        let db = Connection::open(&path).unwrap();
        let mode: i64 = db
            .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
            .unwrap();
        let journal: String = db
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!((mode, journal.as_str()), (2, "wal"));
    }
}
