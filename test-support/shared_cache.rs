//! Share the product of a slow, read-only step between test processes.
//!
//! Nextest runs each test in its own process, so a `OnceLock` shares nothing. A test that
//! indexes a fixture pays for the index again in each process. This module keeps the product
//! in a directory under the Cargo target directory. The first process builds a product under a
//! lock, and the other processes read it.
//!
//! The directory name holds a hash of the test binary and of the input files. The test binary
//! holds the code under test, so a changed build or a changed fixture gives a new directory.
//! A process never reads a stale product. This pattern comes from the `query_golden` harness of
//! `scippy-cli` (commit 98fc446).
//!
//! Share a product only when every test reads it and none changes it.
//! A test that edits files or reindexes keeps its own copy.
//!
//! The minimum supported Rust version is 1.85, and `File::lock` needs 1.89. The lock is a
//! directory: `create_dir` fails when the directory exists.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The shared products of one test binary.
pub struct Shared {
    dir: PathBuf,
}

impl Shared {
    /// Open the shared directory for this test binary and these input folders.
    ///
    /// `base` is a folder under the Cargo target directory that only this test binary uses.
    /// The call removes the directories of earlier input sets from `base`.
    pub fn new(base: &Path, inputs: &[&Path]) -> Shared {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        stamp(&std::env::current_exe().unwrap(), &mut hasher);
        for input in inputs {
            walk(input, &mut hasher);
        }
        let name = format!("shared-{:016x}", hasher.finish());
        let dir = base.join(&name);
        if !dir.exists() {
            fs::create_dir_all(base).unwrap();
            for entry in fs::read_dir(base).unwrap() {
                let path = entry.unwrap().path();
                if path.file_name().is_some_and(|n| n != name.as_str()) {
                    let _ = fs::remove_dir_all(&path);
                }
            }
            fs::create_dir_all(&dir).unwrap();
        }
        Shared { dir }
    }

    /// The path of the product `name`. `build` writes the product at the path that it gets.
    ///
    /// The first process that asks for `name` runs `build`. Other processes wait for it, and
    /// then reuse the file.
    pub fn file(&self, name: &str, build: impl FnOnce(&Path)) -> PathBuf {
        let path = self.dir.join(name);
        let done = self.dir.join(format!("{name}.done"));
        if done.exists() {
            return path;
        }
        let _lock = DirLock::acquire(self.dir.join(format!("{name}.lock.d")));
        if !done.exists() {
            build(&path);
            fs::write(&done, "").unwrap();
        }
        path
    }
}

fn stamp(path: &Path, hasher: &mut impl Hasher) {
    let meta = fs::metadata(path).unwrap();
    path.hash(hasher);
    meta.len().hash(hasher);
    meta.modified().unwrap().hash(hasher);
}

fn walk(dir: &Path, hasher: &mut impl Hasher) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, hasher);
        } else {
            stamp(&path, hasher);
        }
    }
}

/// A lock between test processes: the directory exists while its owner holds the lock.
struct DirLock(PathBuf);

impl DirLock {
    fn acquire(path: PathBuf) -> DirLock {
        let stale = Duration::from_secs(1800);
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return DirLock(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // An owner that died leaves the directory. Remove it after 30 minutes.
                    let age = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok());
                    if age.is_some_and(|age| age > stale) {
                        let _ = fs::remove_dir(&path);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("cannot take the shared lock: {e}"),
            }
        }
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}
