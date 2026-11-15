//! The source store: one read of each input file in one run.

use std::collections::HashMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::project::FoundFile;
use crate::snapshot::sha256_hex;

/// A function that the run calls with the path of a project file after it reads the file.
/// It serves tests: a test counts the reads, or changes the file after its first read.
/// The run never calls it for a manifest or for a support file.
#[derive(Clone)]
pub struct ReadHook(Arc<dyn Fn(&Path) + Send + Sync>);

impl ReadHook {
    pub fn new(hook: impl Fn(&Path) + Send + Sync + 'static) -> Self {
        Self(Arc::new(hook))
    }
}

impl fmt::Debug for ReadHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReadHook")
    }
}

/// The bytes of a file, as the run read them once.
pub(crate) struct Stored {
    /// The text. Every stage that needs the file shares this one allocation.
    pub text: Arc<str>,
    /// The SHA-256 of the bytes, as lowercase hex. The store computes it at the read.
    pub sha256: String,
}

/// A diagnostic code and a message for a file that the run could not read.
pub(crate) type ReadFailure = (&'static str, String);

struct Entry {
    disk: PathBuf,
    read: OnceLock<Result<Stored, ReadFailure>>,
}

/// The content of every accepted file in one run.
///
/// The store reads a file from disk at its first use, and never again. A stage that needs the
/// file later gets the same text, even if the file changed on disk in between. Workers share
/// the store. `OnceLock` makes sure that only one worker reads a file.
pub(crate) struct SourceStore {
    max_bytes: u64,
    hook: Option<ReadHook>,
    entries: HashMap<String, Entry>,
}

impl SourceStore {
    pub fn new(root: &Path, files: &[FoundFile], max_bytes: u64, hook: Option<ReadHook>) -> Self {
        let entries = files
            .iter()
            .map(|f| {
                (
                    f.path.as_str().to_owned(),
                    Entry {
                        disk: f.disk_path(root),
                        read: OnceLock::new(),
                    },
                )
            })
            .collect();
        Self {
            max_bytes,
            hook,
            entries,
        }
    }

    /// The content of a file. The first call reads the file. Later calls, from any worker,
    /// return the same result, a failure too.
    pub fn get(&self, file: &FoundFile) -> &Result<Stored, ReadFailure> {
        let entry = self
            .entries
            .get(file.path.as_str())
            .expect("the store holds every accepted file");
        entry.read.get_or_init(|| {
            let result = read_source(&entry.disk, self.max_bytes);
            if let Some(hook) = &self.hook {
                (hook.0)(&entry.disk);
            }
            result
        })
    }

    /// The paths, sorted, of the files among `paths` whose content on disk now differs from the
    /// content that the run read. The check reads the disk once for each file. A file that the
    /// run did not read is not stale.
    pub fn stale<'a>(&self, paths: impl Iterator<Item = &'a str>) -> Vec<String> {
        let mut stale: Vec<String> = paths
            .filter(|path| {
                let Some(entry) = self.entries.get(*path) else {
                    return false;
                };
                let Some(Ok(stored)) = entry.read.get() else {
                    return false;
                };
                disk_hash(&entry.disk, self.max_bytes).as_deref() != Some(stored.sha256.as_str())
            })
            .map(str::to_owned)
            .collect();
        stale.sort();
        stale
    }
}

/// Read the bytes of a file, up to `max` bytes. `None` when the file is unreadable or larger.
fn read_limited(full: &Path, max: u64) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    std::fs::File::open(full).and_then(|f| f.take(max + 1).read_to_end(&mut bytes))?;
    Ok((bytes.len() as u64 <= max).then_some(bytes))
}

/// The SHA-256 of a file as it is now. `None` when the file is unreadable or over `max` bytes.
fn disk_hash(full: &Path, max: u64) -> Option<String> {
    read_limited(full, max)
        .ok()
        .flatten()
        .map(|bytes| sha256_hex(&bytes))
}

/// Read a file once. Return a diagnostic code and message on failure.
fn read_source(full: &Path, max: u64) -> Result<Stored, ReadFailure> {
    let bytes = match read_limited(full, max) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            // The file grew after the walker measured it.
            return Err((
                "file-too-large",
                format!("skipped: the file exceeds the limit of {max} bytes"),
            ));
        }
        Err(e) => return Err(("unreadable-file", format!("skipped: {e}"))),
    };
    let sha256 = sha256_hex(&bytes);
    match std::str::from_utf8(&bytes) {
        Ok(text) => Ok(Stored {
            text: Arc::from(text),
            sha256,
        }),
        Err(_) => Err(("invalid-utf8", "skipped: the file is not UTF-8".to_string())),
    }
}
