//! Project discovery: the file walker and the document path type.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::ScippyError;
use crate::language::{ExtraFile, ProjectFile};
use crate::report::{Diagnostic, Severity, Stage};

/// What the walker needs from a language pack: the name and the file test.
pub(crate) trait Router {
    fn id(&self) -> &'static str;
    fn accepts(&self, path: &Path) -> bool;
    /// Say whether the pack wants to enter a directory that the walker ignores by default.
    fn enter_ignored(&self, relative_dir: &str) -> bool {
        let _ = relative_dir;
        false
    }
    /// Say whether the pack hides a directory or a file from every pack.
    fn hidden(&self, relative: &str) -> bool {
        let _ = relative;
        false
    }
}

/// Limits on the input of one indexing run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputLimits {
    /// A file larger than this is skipped with a warning.
    pub max_file_bytes: u64,
    /// The walker stops to accept files when the accepted files would exceed this total.
    pub max_total_bytes: u64,
}

impl InputLimits {
    /// One MiB for a file. Hand-written source is far smaller. The largest mypy file is 413 KB.
    pub const DEFAULT_MAX_FILE_BYTES: u64 = 1024 * 1024;
    /// 32 MiB for all files. The graph build took about 1.3 GB for each MB of Python source
    /// (6.2 GB for the 4.7 MB of mypy), so this total bounds the memory of the graph near 40 GB.
    pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
}

impl Default for InputLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: Self::DEFAULT_MAX_FILE_BYTES,
            max_total_bytes: Self::DEFAULT_MAX_TOTAL_BYTES,
        }
    }
}

/// Directories that the walker never enters.
/// They hold version-control data, dependencies, or build output.
const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".jj",
    ".devenv",
    ".direnv",
    ".venv",
    "venv",
    "__pycache__",
    "node_modules",
    "target",
];

/// A canonical document path: UTF-8, `/` separators, no empty, `.` or `..` segment.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectRelativePath(String);

impl ProjectRelativePath {
    pub fn new(path: &str) -> Result<Self, ScippyError> {
        let reason = if path.is_empty() {
            Some("path is empty")
        } else if path.starts_with('/') {
            Some("path is absolute")
        } else if path.contains('\\') {
            Some("path has a backslash")
        } else if path
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
        {
            Some("path has an empty, `.` or `..` segment")
        } else {
            None
        };
        match reason {
            Some(reason) => Err(ScippyError::InvalidPath {
                path: path.to_string(),
                reason,
            }),
            None => Ok(Self(path.to_string())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A file that the walker accepted.
pub(crate) struct FoundFile {
    pub path: ProjectRelativePath,
    /// The index of the pack in the `languages` slice.
    pub pack: usize,
    /// Where to read a file that lies outside the root. `None` for a file below the root.
    pub real: Option<PathBuf>,
}

impl FoundFile {
    /// The path to read the file from.
    pub fn disk_path(&self, root: &Path) -> PathBuf {
        self.real
            .clone()
            .unwrap_or_else(|| root.join(self.path.as_str()))
    }
}

/// The files that the walker routed to a language pack.
pub(crate) struct Discovery {
    /// The canonical project root.
    pub root: PathBuf,
    pub files: Vec<FoundFile>,
    /// Every regular file that the walker saw, whether a pack accepted it or not.
    pub all_files: Vec<ProjectFile>,
    /// Count of all regular files that the walker saw.
    pub files_discovered: usize,
    pub diagnostics: Vec<Diagnostic>,
}

/// Walk the project in a deterministic order and route each file to one pack.
///
/// The walker never follows symbolic links and never reads file content.
/// It skips the output file. It enforces the input limits from the sizes that the file system reports.
pub(crate) fn discover(
    root: &Path,
    output: &Path,
    languages: &[&dyn Router],
    limits: InputLimits,
    extra: &[ExtraFile],
) -> Result<Discovery, ScippyError> {
    let invalid = |reason: String| ScippyError::InvalidRoot {
        path: root.to_path_buf(),
        reason,
    };
    let root = fs::canonicalize(root).map_err(|e| invalid(e.to_string()))?;
    if !root.is_dir() {
        return Err(invalid("not a directory".to_string()));
    }
    fs::read_dir(&root).map_err(|e| invalid(e.to_string()))?;

    let mut walk = Walk {
        languages,
        limits,
        total_bytes: 0,
        limit_reached: false,
        output: absolute_output(output),
        found: Discovery {
            root: root.clone(),
            files: Vec::new(),
            all_files: Vec::new(),
            files_discovered: 0,
            diagnostics: Vec::new(),
        },
    };
    walk.directory(&root, "", None);
    for file in extra {
        let name = file.path.as_str().rsplit('/').next().unwrap_or_default();
        walk.found.files_discovered += 1;
        walk.found.all_files.push(ProjectFile {
            path: file.path.clone(),
            size: file.size,
        });
        walk.route(
            file.path.clone(),
            Path::new(name),
            file.size,
            None,
            Some(file.real.clone()),
        );
    }
    Ok(walk.found)
}

/// Resolve the output path even when the file does not exist yet.
fn absolute_output(output: &Path) -> PathBuf {
    if let Ok(path) = fs::canonicalize(output) {
        return path;
    }
    let name = output.file_name().map(PathBuf::from).unwrap_or_default();
    let parent = match output.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::canonicalize(parent)
        .map(|p| p.join(&name))
        .unwrap_or_else(|_| output.to_path_buf())
}

struct Walk<'a> {
    languages: &'a [&'a dyn Router],
    limits: InputLimits,
    /// The sum of the sizes of the accepted files.
    total_bytes: u64,
    limit_reached: bool,
    output: PathBuf,
    found: Discovery,
}

impl Walk<'_> {
    /// `only` names the one pack that owns the files of this directory. The walker entered the
    /// directory for that pack, although it ignores such a directory by default.
    fn directory(&mut self, dir: &Path, prefix: &str, only: Option<usize>) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                let file = (!prefix.is_empty()).then(|| prefix.trim_end_matches('/').to_string());
                self.warn(Severity::Error, "unreadable-directory", file, e.to_string());
                return;
            }
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let name = entry.file_name();
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let utf8 = name.to_str();

            if let Some(n) = utf8 {
                let relative = format!("{prefix}{n}");
                if self.languages.iter().any(|l| l.hidden(&relative)) {
                    continue;
                }
            }

            if kind.is_dir() {
                let mut owner = only;
                if let Some(n) = utf8.filter(|n| IGNORED_DIRS.contains(n)) {
                    let relative = format!("{prefix}{n}");
                    let wanting = (0..self.languages.len()).find(|&i| {
                        only.is_none_or(|o| o == i) && self.languages[i].enter_ignored(&relative)
                    });
                    if wanting.is_none() {
                        continue;
                    }
                    owner = wanting;
                }
                match utf8 {
                    Some(n) => self.directory(&path, &format!("{prefix}{n}/"), owner),
                    None => self.warn(
                        Severity::Warning,
                        "non-utf8-path",
                        None,
                        format!("skipped directory with a non-UTF-8 name: {name:?}"),
                    ),
                }
                continue;
            }

            let accepting: Vec<usize> = (0..self.languages.len())
                .filter(|&i| only.is_none_or(|o| o == i))
                .filter(|&i| self.languages[i].accepts(Path::new(&name)))
                .collect();

            if kind.is_symlink() {
                if let (Some(n), false) = (utf8, accepting.is_empty()) {
                    self.warn(
                        Severity::Warning,
                        "symlink-skipped",
                        Some(format!("{prefix}{n}")),
                        "symbolic links are not indexed".to_string(),
                    );
                }
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            self.found.files_discovered += 1;
            if path == self.output {
                continue;
            }
            if let Some(relative) =
                utf8.and_then(|n| ProjectRelativePath::new(&format!("{prefix}{n}")).ok())
            {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                self.found.all_files.push(ProjectFile {
                    path: relative,
                    size,
                });
            }
            if accepting.is_empty() {
                continue;
            }
            let Some(n) = utf8 else {
                self.warn(
                    Severity::Warning,
                    "non-utf8-path",
                    None,
                    format!("skipped file with a non-UTF-8 name: {name:?}"),
                );
                continue;
            };
            let relative = format!("{prefix}{n}");
            let relative = match ProjectRelativePath::new(&relative) {
                Ok(relative) => relative,
                Err(e) => {
                    self.warn(Severity::Warning, "invalid-path", None, e.to_string());
                    continue;
                }
            };
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            self.route(relative, Path::new(&name), size, only, None);
        }
    }

    /// Give a file that the walker saw to the pack that accepts it, within the input limits.
    /// `real` is where to read a file that lies outside the root.
    fn route(
        &mut self,
        relative: ProjectRelativePath,
        name: &Path,
        size: u64,
        only: Option<usize>,
        real: Option<PathBuf>,
    ) {
        let accepting: Vec<usize> = (0..self.languages.len())
            .filter(|&i| only.is_none_or(|o| o == i))
            .filter(|&i| self.languages[i].accepts(name))
            .collect();
        if accepting.is_empty() {
            return;
        }
        if accepting.len() > 1 {
            let ids: Vec<_> = accepting.iter().map(|&i| self.languages[i].id()).collect();
            self.warn(
                Severity::Error,
                "ambiguous-language",
                Some(relative.as_str().to_string()),
                format!("claimed by more than one language pack: {}", ids.join(", ")),
            );
            return;
        }
        if size > self.limits.max_file_bytes {
            self.warn(
                Severity::Warning,
                "file-too-large",
                Some(relative.as_str().to_string()),
                format!(
                    "skipped: {size} bytes exceed the limit of {} bytes",
                    self.limits.max_file_bytes
                ),
            );
            return;
        }
        if self.limit_reached {
            return;
        }
        if self.total_bytes + size > self.limits.max_total_bytes {
            self.limit_reached = true;
            self.warn(
                Severity::Error,
                "input-limit-reached",
                Some(relative.as_str().to_string()),
                format!(
                    "skipped this file and every later file: the total input would exceed {} bytes",
                    self.limits.max_total_bytes
                ),
            );
            return;
        }
        self.total_bytes += size;
        self.found.files.push(FoundFile {
            path: relative,
            pack: accepting[0],
            real,
        });
    }

    fn warn(
        &mut self,
        severity: Severity,
        code: &'static str,
        file: Option<String>,
        message: String,
    ) {
        self.found.diagnostics.push(Diagnostic {
            severity,
            stage: Stage::Discover,
            file,
            range: None,
            code,
            message,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ext(&'static str, &'static str);
    impl Router for Ext {
        fn id(&self) -> &'static str {
            self.0
        }
        fn accepts(&self, path: &Path) -> bool {
            path.extension().is_some_and(|e| e == self.1)
        }
    }

    fn touch(root: &Path, relative: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }

    #[test]
    fn path_rules() {
        for good in ["a.py", "src/a.py", "src/café/ü.py", "a b/c.py"] {
            assert!(ProjectRelativePath::new(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "/a.py",
            "a//b.py",
            "./a.py",
            "a/./b.py",
            "../a.py",
            "a/../b.py",
            "a/",
            "a\\b.py",
        ] {
            assert!(ProjectRelativePath::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn walks_in_order_and_skips_ignored_directories() {
        let dir = tempfile::tempdir().unwrap();
        for f in [
            "b.py",
            "a.py",
            "pkg/z.py",
            "pkg/y.py",
            "node_modules/x.py",
            ".git/h.py",
            "notes.txt",
        ] {
            touch(dir.path(), f);
        }
        let py = Ext("python", "py");
        let found = discover(
            dir.path(),
            &dir.path().join("index.scip"),
            &[&py],
            InputLimits::default(),
            &[],
        )
        .unwrap();
        let paths: Vec<_> = found.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.py", "b.py", "pkg/y.py", "pkg/z.py"]);
        assert_eq!(found.files_discovered, 5);
        assert!(found.diagnostics.is_empty());
    }

    #[test]
    fn skips_the_output_file() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "a.scip");
        let scip = Ext("scip", "scip");
        let found = discover(
            dir.path(),
            &dir.path().join("a.scip"),
            &[&scip],
            InputLimits::default(),
            &[],
        )
        .unwrap();
        assert!(found.files.is_empty());
    }

    #[test]
    fn reports_ambiguous_files_and_skips_them() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "a.py");
        let (one, two) = (Ext("one", "py"), Ext("two", "py"));
        let found = discover(
            dir.path(),
            &dir.path().join("o.scip"),
            &[&one, &two],
            InputLimits::default(),
            &[],
        )
        .unwrap();
        assert!(found.files.is_empty());
        assert_eq!(found.diagnostics[0].code, "ambiguous-language");
    }

    #[test]
    fn lists_every_regular_file_for_native_packs() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["a.py", "b.png", "sub/c.txt", "node_modules/x.js"] {
            touch(dir.path(), f);
        }
        let py = Ext("python", "py");
        let found = discover(
            dir.path(),
            &dir.path().join("o.scip"),
            &[&py],
            InputLimits::default(),
            &[],
        )
        .unwrap();
        let all: Vec<_> = found.all_files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(all, ["a.py", "b.png", "sub/c.txt"]);
        assert_eq!(found.files.len(), 1);
    }

    struct Enter(&'static str, &'static str, bool);
    impl Router for Enter {
        fn id(&self) -> &'static str {
            self.0
        }
        fn accepts(&self, path: &Path) -> bool {
            path.extension().is_some_and(|e| e == self.1)
        }
        fn enter_ignored(&self, relative_dir: &str) -> bool {
            self.2 && relative_dir.ends_with("target")
        }
    }

    #[test]
    fn a_pack_can_enter_an_ignored_folder_for_its_own_files() {
        let dir = tempfile::tempdir().unwrap();
        for f in [
            "target/n.md",
            "target/x.py",
            "a/target/m.md",
            "node_modules/q.md",
        ] {
            touch(dir.path(), f);
        }
        let (md, py) = (Enter("md", "md", true), Enter("py", "py", false));
        let found = discover(
            dir.path(),
            &dir.path().join("o.scip"),
            &[&md, &py],
            InputLimits::default(),
            &[],
        )
        .unwrap();
        let paths: Vec<_> = found.files.iter().map(|f| f.path.as_str()).collect();
        // The other pack gets no file below the folder. `node_modules` stays closed.
        assert_eq!(paths, ["a/target/m.md", "target/n.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "real.py");
        std::os::unix::fs::symlink("real.py", dir.path().join("link.py")).unwrap();
        let py = Ext("python", "py");
        let found = discover(
            dir.path(),
            &dir.path().join("o.scip"),
            &[&py],
            InputLimits::default(),
            &[],
        )
        .unwrap();
        assert_eq!(found.files.len(), 1);
        assert_eq!(found.diagnostics[0].code, "symlink-skipped");
    }

    #[test]
    fn rejects_a_bad_root() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("o.scip");
        assert!(
            discover(
                &dir.path().join("missing"),
                &out,
                &[],
                InputLimits::default(),
                &[]
            )
            .is_err()
        );
        touch(dir.path(), "file.txt");
        assert!(
            discover(
                &dir.path().join("file.txt"),
                &out,
                &[],
                InputLimits::default(),
                &[]
            )
            .is_err()
        );
    }

    fn write(root: &Path, relative: &str, bytes: usize) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn skips_a_file_over_the_file_limit() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "small.py", 10);
        write(dir.path(), "big.py", 11);
        let py = Ext("python", "py");
        let limits = InputLimits {
            max_file_bytes: 10,
            max_total_bytes: 1000,
        };
        let found = discover(dir.path(), &dir.path().join("o.scip"), &[&py], limits, &[]).unwrap();
        let paths: Vec<_> = found.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["small.py"]);
        assert_eq!(found.diagnostics.len(), 1);
        assert_eq!(found.diagnostics[0].code, "file-too-large");
        assert_eq!(found.diagnostics[0].file.as_deref(), Some("big.py"));
    }

    #[test]
    fn stops_at_the_total_limit_in_walk_order() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.py", "b.py", "c.py", "d.py"] {
            write(dir.path(), name, 10);
        }
        let py = Ext("python", "py");
        let limits = InputLimits {
            max_file_bytes: 10,
            max_total_bytes: 25,
        };
        let found = discover(dir.path(), &dir.path().join("o.scip"), &[&py], limits, &[]).unwrap();
        let paths: Vec<_> = found.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.py", "b.py"]);
        // One diagnostic names the first file that the walker dropped.
        assert_eq!(found.diagnostics.len(), 1);
        assert_eq!(found.diagnostics[0].code, "input-limit-reached");
        assert_eq!(found.diagnostics[0].severity, Severity::Error);
        assert_eq!(found.diagnostics[0].file.as_deref(), Some("c.py"));
    }
}
