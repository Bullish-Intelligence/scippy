//! A disposable cache of file graphs and their partial paths.

use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
pub(crate) struct CachedGraph {
    pub graph: stack_graphs::serde::StackGraph,
    pub paths: Vec<stack_graphs::serde::PartialPath>,
}

pub(crate) struct GraphCache(Connection);

impl GraphCache {
    pub fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        db.busy_timeout(Duration::from_secs(30))
            .map_err(|e| e.to_string())?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS file_graph (key TEXT PRIMARY KEY, value BLOB NOT NULL)",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self(db))
    }

    pub fn get(&self, key: &str) -> Result<Option<CachedGraph>, String> {
        let bytes: Option<Vec<u8>> = self
            .0
            .query_row("SELECT value FROM file_graph WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        bytes
            .map(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
            .transpose()
    }

    pub fn put(&self, key: &str, value: &CachedGraph) -> Result<(), String> {
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        self.0
            .execute(
                "INSERT OR REPLACE INTO file_graph VALUES (?1, ?2)",
                params![key, bytes],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
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

pub(crate) fn key(
    pack: &str,
    pack_key: &str,
    path: &str,
    source: &str,
    globals: &[(String, String)],
    dialect: usize,
    shadow_aware: bool,
) -> Result<String, String> {
    let input = serde_json::to_vec(&(
        "file-graph-v1",
        executable_hash()?,
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
