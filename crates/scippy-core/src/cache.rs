//! A disposable cache of file graphs and their partial paths.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
pub(crate) struct CachedGraph {
    pub graph: stack_graphs::serde::StackGraph,
    pub paths: Vec<stack_graphs::serde::PartialPath>,
}

/// The run-wide state of the cache. All shards share it. A failure turns the cache off for the
/// rest of the run, and the run goes on without it.
pub(crate) struct CacheGate {
    path: Option<PathBuf>,
    timeout: Duration,
    off: AtomicBool,
    prepared: OnceLock<()>,
}

impl CacheGate {
    pub fn new(path: Option<&Path>, timeout: Duration) -> Self {
        Self {
            path: path.map(Path::to_path_buf),
            timeout,
            off: AtomicBool::new(false),
            prepared: OnceLock::new(),
        }
    }

    pub fn is_on(&self) -> bool {
        self.path.is_some() && !self.off.load(Ordering::SeqCst)
    }

    /// Turn the cache off and print one warning for the whole run.
    pub fn fail(&self, error: &str) {
        if !self.off.swap(true, Ordering::SeqCst) {
            let path = self.path.as_deref().unwrap_or(Path::new(""));
            eprintln!(
                "warning: the graph cache {} is off for this run: {error}",
                path.display()
            );
        }
    }

    /// Open a connection for one shard. Return `None` when the cache is off.
    pub fn open(&self) -> Option<GraphCache> {
        let path = self.path.as_deref()?;
        self.prepared.get_or_init(|| self.prepare(path));
        if !self.is_on() {
            return None;
        }
        let exe = match executable_hash() {
            Ok(exe) => exe,
            Err(error) => {
                self.fail(&error);
                return None;
            }
        };
        let opened = Connection::open(path).and_then(|db| {
            db.busy_timeout(self.timeout)?;
            Ok(db)
        });
        match opened {
            Ok(db) => Some(GraphCache { db, exe }),
            Err(error) => {
                self.fail(&error.to_string());
                None
            }
        }
    }

    /// Run once for the whole run: hash the executable, create the table, and replace a
    /// damaged file.
    fn prepare(&self, path: &Path) {
        if let Err(error) = executable_hash() {
            return self.fail(&error);
        }
        let mut created = create(path, self.timeout);
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
            created = create(path, self.timeout);
        }
        if let Err(error) = created {
            self.fail(&error.to_string());
        }
    }
}

fn create(path: &Path, timeout: Duration) -> rusqlite::Result<()> {
    let db = Connection::open(path)?;
    db.busy_timeout(timeout)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS file_graph_v2 \
         (key TEXT PRIMARY KEY, value BLOB NOT NULL, sum TEXT NOT NULL)",
    )
}

pub(crate) struct GraphCache {
    db: Connection,
    exe: &'static str,
}

/// The checksum covers the key and the value. A row that moves to another key fails it.
fn checksum(key: &str, value: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update((key.len() as u64).to_le_bytes());
    hash.update(key.as_bytes());
    hash.update(value);
    format!("{:x}", hash.finalize())
}

impl GraphCache {
    /// Return `Ok(None)` for a missing row and for a row that fails its checksum. The failed
    /// row is deleted.
    pub fn get(&self, key: &str) -> Result<Option<CachedGraph>, String> {
        let row: Option<(Value, Value)> = self
            .db
            .query_row(
                "SELECT value, sum FROM file_graph_v2 WHERE key = ?1",
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
                serde_json::from_slice(&bytes)
                    .map(Some)
                    .map_err(|e| e.to_string())
            }
            _ => {
                self.db
                    .execute("DELETE FROM file_graph_v2 WHERE key = ?1", [key])
                    .map_err(|e| e.to_string())?;
                Ok(None)
            }
        }
    }

    pub fn put(&self, key: &str, value: &CachedGraph) -> Result<(), String> {
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        let sum = checksum(key, &bytes);
        self.db
            .execute(
                "INSERT OR REPLACE INTO file_graph_v2 VALUES (?1, ?2, ?3)",
                params![key, bytes, sum],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

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
    ) -> Result<String, String> {
        let input = serde_json::to_vec(&(
            "file-graph-v2",
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
        Ok(format!("{:x}", Sha256::digest(input)))
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
