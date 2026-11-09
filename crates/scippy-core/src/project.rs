//! Project discovery: the file walker and the document path type.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::ScippyError;
use crate::language::LanguagePack;
use crate::report::{Diagnostic, Severity, Stage};

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

/// The files that the walker routed to a language pack.
pub(crate) struct Discovery {
    /// The canonical project root.
    pub root: PathBuf,
    /// Each file with the index of its pack in the `languages` slice.
    pub files: Vec<(ProjectRelativePath, usize)>,
    /// Count of all regular files that the walker saw.
    pub files_discovered: usize,
    pub diagnostics: Vec<Diagnostic>,
}

/// Walk the project in a deterministic order and route each file to one pack.
///
/// The walker never follows symbolic links and never reads file content.
/// It skips the output file.
pub(crate) fn discover(
    root: &Path,
    output: &Path,
    languages: &[&dyn LanguagePack],
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
        output: absolute_output(output),
        found: Discovery {
            root: root.clone(),
            files: Vec::new(),
            files_discovered: 0,
            diagnostics: Vec::new(),
        },
    };
    walk.directory(&root, "");
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
    languages: &'a [&'a dyn LanguagePack],
    output: PathBuf,
    found: Discovery,
}

impl Walk<'_> {
    fn directory(&mut self, dir: &Path, prefix: &str) {
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

            if kind.is_dir() {
                if utf8.is_some_and(|n| IGNORED_DIRS.contains(&n)) {
                    continue;
                }
                match utf8 {
                    Some(n) => self.directory(&path, &format!("{prefix}{n}/")),
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
            if path == self.output || accepting.is_empty() {
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
            if accepting.len() > 1 {
                let ids: Vec<_> = accepting.iter().map(|&i| self.languages[i].id()).collect();
                self.warn(
                    Severity::Error,
                    "ambiguous-language",
                    Some(relative.as_str().to_string()),
                    format!("claimed by more than one language pack: {}", ids.join(", ")),
                );
                continue;
            }
            self.found.files.push((relative, accepting[0]));
        }
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
    impl LanguagePack for Ext {
        fn id(&self) -> &'static str {
            self.0
        }
        fn scip_language(&self) -> &'static str {
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
        let found = discover(dir.path(), &dir.path().join("index.scip"), &[&py]).unwrap();
        let paths: Vec<_> = found.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["a.py", "b.py", "pkg/y.py", "pkg/z.py"]);
        assert_eq!(found.files_discovered, 5);
        assert!(found.diagnostics.is_empty());
    }

    #[test]
    fn skips_the_output_file() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "a.scip");
        let scip = Ext("scip", "scip");
        let found = discover(dir.path(), &dir.path().join("a.scip"), &[&scip]).unwrap();
        assert!(found.files.is_empty());
    }

    #[test]
    fn reports_ambiguous_files_and_skips_them() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "a.py");
        let (one, two) = (Ext("one", "py"), Ext("two", "py"));
        let found = discover(dir.path(), &dir.path().join("o.scip"), &[&one, &two]).unwrap();
        assert!(found.files.is_empty());
        assert_eq!(found.diagnostics[0].code, "ambiguous-language");
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "real.py");
        std::os::unix::fs::symlink("real.py", dir.path().join("link.py")).unwrap();
        let py = Ext("python", "py");
        let found = discover(dir.path(), &dir.path().join("o.scip"), &[&py]).unwrap();
        assert_eq!(found.files.len(), 1);
        assert_eq!(found.diagnostics[0].code, "symlink-skipped");
    }

    #[test]
    fn rejects_a_bad_root() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("o.scip");
        assert!(discover(&dir.path().join("missing"), &out, &[]).is_err());
        touch(dir.path(), "file.txt");
        assert!(discover(&dir.path().join("file.txt"), &out, &[]).is_err());
    }
}
