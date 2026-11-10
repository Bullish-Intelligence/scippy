//! Extra roots: code repositories that appear as virtual top-level folders. It copies
//! `vault/roots.py` and `vault/root_listing.py` of loci-core (D-069, D-077).
//!
//! A vault has one primary root. A workspace adds more roots with `[[root]]` tables in
//! `vault.toml`, and with a generated file that `roots_file` names. The file `src/a.py` of the root
//! `repo` has the vault path `repo/src/a.py`. Like the ignore list, the roots config fails closed:
//! a bad entry, an unknown key, a name that clashes with a top-level entry of the vault, an
//! overlap of two folders, or an unreadable roots file refuses the vault. A folder that does not
//! exist is a warning, and the root lists no files.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::GlobSet;
use crate::policy::{IdTemplate, expand_user, read_limited};
use crate::text::{casefold, py_strip};

const DEFAULT_ID_TEMPLATE: &str = "{name}/code/{path}";
/// The code cap of semantic-views: a claimed file larger than this is not a document.
const DEFAULT_MAX_BYTES: u64 = 256 * 1024;
const DEFAULT_NOTE_MAX_BYTES: u64 = 1024 * 1024;
const DEFAULT_EXCLUDE_DIRS: &[&str] = &["vendor", "node_modules"];
/// Folders that a walk never enters. A git root needs no such list: git lists tracked files only.
const WALK_EXCLUDE: &[&str] = &[
    ".git/**",
    ".jj/**",
    ".hg/**",
    ".svn/**",
    ".venv/**",
    "venv/**",
    ".devenv/**",
    ".direnv/**",
    ".tox/**",
    ".ruff_cache/**",
    ".pytest_cache/**",
    ".mypy_cache/**",
    ".hypothesis/**",
    "__pycache__/**",
    "target/**",
    "dist/**",
    "build/**",
    "result",
    "result-*",
];
const LOCK_NAMES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
    "uv.lock",
    "poetry.lock",
    "pdm.lock",
    "cargo.lock",
    "gemfile.lock",
    "composer.lock",
    "flake.lock",
    "devenv.lock",
    "go.sum",
];
const BINARY_SNIFF_BYTES: usize = 8192;

const ROOT_KEYS: &[&str] = &[
    "name",
    "path",
    "lister",
    "writable",
    "stem_visible",
    "include",
    "exclude",
    "id_template",
    "max_bytes",
    "note_max_bytes",
    "binary_sniff",
    "drop_lock_files",
    "exclude_dirs",
    "untracked",
    "claims",
];

/// One extra root.
#[derive(Clone, Debug)]
pub struct RootSpec {
    pub name: String,
    pub path: String,
    /// The lister is `git` (tracked files) or `walk`.
    pub git: bool,
    pub stem_visible: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub id_template: String,
    pub max_bytes: u64,
    pub note_max_bytes: u64,
    pub binary_sniff: bool,
    pub drop_lock_files: bool,
    pub exclude_dirs: Vec<String>,
    pub untracked: bool,
    /// `(analyzer name, root-relative globs)`. A glob claims a file of this root only.
    pub claims: Vec<(String, Vec<String>)>,
}

impl RootSpec {
    /// The folder on disk.
    pub fn base(&self) -> PathBuf {
        expand_user(&self.path).unwrap_or_else(|_| PathBuf::from(&self.path))
    }

    /// The template that gives a file of this root its id in the ignore list.
    pub fn ignore_template(&self) -> IdTemplate {
        let target = self.id_template.replace("{name}", &self.name);
        IdTemplate::new(&format!("{}/{{path:path}}", self.name), &target)
            .expect("a checked root name and id template")
    }
}

fn fault(where_: &str, message: &str) -> String {
    format!("{where_}: {message}")
}

fn name_ok(name: &str) -> bool {
    let chars: Vec<char> = name.chars().collect();
    if chars.is_empty() || chars.len() > 100 {
        return false;
    }
    let first = |c: char| c.is_ascii_alphanumeric() || c == '_';
    first(chars[0]) && chars[1..].iter().all(|&c| first(c) || c == '.' || c == '-')
}

fn bool_key(entry: &toml::Table, key: &str, default: bool, where_: &str) -> Result<bool, String> {
    match entry.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| fault(where_, &format!("{key} must be true or false"))),
    }
}

fn int_key(entry: &toml::Table, key: &str, default: u64, where_: &str) -> Result<u64, String> {
    match entry.get(key) {
        None => Ok(default),
        Some(v) => match v.as_integer() {
            Some(n) if n >= 1 => Ok(n as u64),
            _ => Err(fault(
                where_,
                &format!("{key} must be an integer of 1 or more"),
            )),
        },
    }
}

fn globs(
    value: Option<&toml::Value>,
    key: &str,
    default: Vec<String>,
    where_: &str,
    allow_empty: bool,
) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(default);
    };
    let list = value
        .as_array()
        .and_then(|a| a.iter().map(|i| i.as_str()).collect::<Option<Vec<_>>>())
        .ok_or_else(|| fault(where_, &format!("{key} must be an array of strings")))?;
    if list.is_empty() && !allow_empty {
        return Err(fault(where_, &format!("{key} must not be empty")));
    }
    let mut out = Vec::new();
    for raw in list {
        let pattern = py_strip(raw).trim_end_matches('/');
        if pattern.is_empty() || pattern.starts_with('/') || pattern.split('/').any(|s| s == "..") {
            return Err(fault(
                where_,
                &format!("{key} has a blank, rooted, or '..' pattern"),
            ));
        }
        out.push(pattern.to_string());
    }
    Ok(out)
}

fn parse_root(entry: &toml::Value, index: usize, origin: &str) -> Result<RootSpec, String> {
    let mut where_ = format!("[[root]] #{} in {origin}", index + 1);
    let entry = entry
        .as_table()
        .ok_or_else(|| fault(&where_, "must be a table"))?;
    let mut unknown: Vec<&str> = entry
        .keys()
        .map(String::as_str)
        .filter(|k| !ROOT_KEYS.contains(k))
        .collect();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        return Err(fault(
            &where_,
            &format!("unknown keys: {}", unknown.join(", ")),
        ));
    }
    let name = match entry.get("name").and_then(|v| v.as_str()) {
        Some(n) if name_ok(n) => n.to_string(),
        _ => {
            return Err(fault(
                &where_,
                "name must match [A-Za-z0-9_][A-Za-z0-9._-]* (one path segment)",
            ));
        }
    };
    where_ = format!("[[root]] {name:?} in {origin}");
    let path = match entry.get("path").and_then(|v| v.as_str()) {
        Some(p) if !p.trim().is_empty() => p.to_string(),
        _ => return Err(fault(&where_, "path must be a non-empty string")),
    };
    if !(path.starts_with('~') || Path::new(&path).is_absolute()) {
        return Err(fault(&where_, "path must be absolute or start with '~'"));
    }
    if expand_user(&path).is_err() {
        return Err(fault(&where_, "cannot expand '~' in path"));
    }
    let git = match entry.get("lister") {
        None => true,
        Some(v) => match v.as_str() {
            Some("git") => true,
            Some("walk") => false,
            _ => return Err(fault(&where_, "lister must be 'git' or 'walk'")),
        },
    };
    let template = match entry.get("id_template") {
        None => DEFAULT_ID_TEMPLATE.to_string(),
        Some(v) => match v.as_str() {
            Some(t) if t.contains("{path}") => t.to_string(),
            _ => {
                return Err(fault(
                    &where_,
                    "id_template must be a string that holds {path}",
                ));
            }
        },
    };
    let stripped = template.replace("{name}", "").replace("{path}", "");
    if stripped.contains('{') || stripped.contains('}') {
        return Err(fault(&where_, "id_template may use only {name} and {path}"));
    }
    let default_exclude = if git {
        Vec::new()
    } else {
        WALK_EXCLUDE.iter().map(|s| s.to_string()).collect()
    };
    let writable = bool_key(entry, "writable", false, &where_)?;
    let exclude_dirs = match entry.get("exclude_dirs") {
        None => DEFAULT_EXCLUDE_DIRS.iter().map(|s| s.to_string()).collect(),
        Some(v) => {
            let list = v
                .as_array()
                .and_then(|a| a.iter().map(|i| i.as_str()).collect::<Option<Vec<_>>>())
                .filter(|l| l.iter().all(|n| !n.is_empty() && !n.contains('/')))
                .ok_or_else(|| {
                    fault(
                        &where_,
                        "exclude_dirs must be an array of folder names without '/'",
                    )
                })?;
            list.iter().map(|s| s.to_string()).collect()
        }
    };
    let claims = match entry.get("claims") {
        None => Vec::new(),
        Some(v) => {
            let table = v.as_table().ok_or_else(|| {
                fault(
                    &where_,
                    "claims must be a table of analyzer name to glob list",
                )
            })?;
            let mut out = Vec::new();
            for (name, value) in table {
                if name.trim().is_empty() {
                    return Err(fault(
                        &where_,
                        "claims must name an analyzer by a non-empty string",
                    ));
                }
                let list = globs(
                    Some(value),
                    "claims",
                    Vec::new(),
                    &format!("{where_} claims[{name:?}]"),
                    false,
                )?;
                out.push((name.clone(), list));
            }
            out
        }
    };
    Ok(RootSpec {
        name,
        path,
        git,
        stem_visible: bool_key(entry, "stem_visible", false, &where_)?,
        include: globs(
            entry.get("include"),
            "include",
            vec!["**/*".to_string()],
            &where_,
            false,
        )?,
        exclude: globs(
            entry.get("exclude"),
            "exclude",
            default_exclude,
            &where_,
            true,
        )?,
        id_template: template,
        max_bytes: int_key(entry, "max_bytes", DEFAULT_MAX_BYTES, &where_)?,
        note_max_bytes: int_key(entry, "note_max_bytes", DEFAULT_NOTE_MAX_BYTES, &where_)?,
        binary_sniff: bool_key(entry, "binary_sniff", true, &where_)?,
        drop_lock_files: bool_key(entry, "drop_lock_files", true, &where_)?,
        exclude_dirs,
        untracked: bool_key(entry, "untracked", writable, &where_)?,
        claims,
    })
}

/// Parse the value of the `root` key: an array of tables.
pub fn parse_roots_array(value: &toml::Value, origin: &str) -> Result<Vec<RootSpec>, String> {
    let list = value
        .as_array()
        .ok_or_else(|| format!("root in {origin} must be an array of tables ([[root]])"))?;
    list.iter()
        .enumerate()
        .map(|(i, entry)| parse_root(entry, i, origin))
        .collect()
}

/// Read the roots file that `roots_file` names. A relative path starts at the vault.
pub fn read_roots_file(root: &Path, configured: &str) -> Result<Vec<RootSpec>, String> {
    let path = expand_user(configured).map_err(|_| "cannot expand '~' in roots_file")?;
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    let data = read_limited(&path).map_err(|e| format!("cannot read the roots file: {e}"))?;
    let table: toml::Table = String::from_utf8(data)
        .ok()
        .and_then(|t| t.parse().ok())
        .ok_or("the roots file is not valid UTF-8 TOML")?;
    let mut unknown: Vec<&str> = table
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "root")
        .collect();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        return Err(format!(
            "the roots file has unknown top-level keys: {}",
            unknown.join(", ")
        ));
    }
    let origin = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("roots file");
    match table.get("root") {
        None => Ok(Vec::new()),
        Some(v) => parse_roots_array(v, origin),
    }
}

fn safe_resolve(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Check the set of roots: unique names, no clash with a top-level entry of the vault, and no
/// folder that equals, holds, or lies inside another root or the vault.
pub fn check_roots(vault: &Path, roots: &[RootSpec]) -> Result<(), String> {
    if roots.is_empty() {
        return Ok(());
    }
    let mut seen: Vec<(String, &str)> = Vec::new();
    for spec in roots {
        let key = casefold(&spec.name);
        if let Some((_, first)) = seen.iter().find(|(k, _)| *k == key) {
            return Err(format!("root name {:?} repeats {first:?}", spec.name));
        }
        seen.push((key, &spec.name));
    }
    let top: BTreeSet<String> = std::fs::read_dir(vault)
        .map_err(|e| format!("cannot read the vault folder: {:?}", e.kind()))?
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(casefold))
        .collect();
    for spec in roots {
        if top.contains(&casefold(&spec.name)) {
            return Err(format!(
                "root name {:?} clashes with a top-level entry of the vault: rename the root",
                spec.name
            ));
        }
    }
    let vault_resolved = safe_resolve(vault);
    let resolved: Vec<(&RootSpec, PathBuf)> =
        roots.iter().map(|r| (r, safe_resolve(&r.base()))).collect();
    for (index, (spec, base)) in resolved.iter().enumerate() {
        if base.starts_with(&vault_resolved) || vault_resolved.starts_with(base) {
            return Err(format!("root {:?} overlaps the vault folder", spec.name));
        }
        for (other, other_base) in &resolved[..index] {
            if base == other_base {
                return Err(format!(
                    "roots {:?} and {:?} name the same folder",
                    other.name, spec.name
                ));
            }
        }
    }
    Ok(())
}

/// The files of one root, in the two tiers of loci-core.
#[derive(Debug, Default)]
pub struct RootListing {
    /// Tier 1: notes and claimed files. `(vault path, size)`.
    pub sources: Vec<(String, u64)>,
    /// Tier 2: every other file. `(vault path, size)`.
    pub link_targets: Vec<(String, u64)>,
    /// A fault that does not stop the run: `root_unavailable`, `root_list_failed`.
    pub diagnostic: Option<&'static str>,
}

/// The tracked paths of a git root, sorted, or `None` when `git` fails.
fn git_names(base: &Path, untracked: bool) -> Option<Vec<String>> {
    let mut command = Command::new("git");
    command
        .args(["-c", "core.fsmonitor=false", "-C"])
        .arg(base)
        .args(["ls-files", "-z"]);
    if untracked {
        command.args(["--cached", "--others", "--exclude-standard"]);
    }
    let output = command.env("GIT_OPTIONAL_LOCKS", "0").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let names: BTreeSet<String> = output
        .stdout
        .split(|&b| b == 0)
        .filter(|n| !n.is_empty())
        .filter_map(|n| std::str::from_utf8(n).ok().map(str::to_string))
        .collect();
    Some(names.into_iter().collect())
}

/// The paths of a walk root. A folder link is not entered. A file link is listed and dropped later.
fn walk_names(spec: &RootSpec, base: &Path) -> Vec<String> {
    let exclude = GlobSet::new(&spec.exclude);
    let mut names = Vec::new();
    let mut stack = vec![(base.to_path_buf(), String::new())];
    while let Some((dir, key)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut dirs: Vec<(String, PathBuf)> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let is_dir = std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir());
            if is_dir {
                let is_link = entry.file_type().is_ok_and(|t| t.is_symlink());
                let rel = if key.is_empty() {
                    name.clone()
                } else {
                    format!("{key}/{name}")
                };
                if !spec.exclude_dirs.contains(&name) && !exclude.is_match(&rel) && !is_link {
                    dirs.push((rel, entry.path()));
                }
            } else {
                files.push(name);
            }
        }
        files.sort();
        for name in files {
            names.push(if key.is_empty() {
                name
            } else {
                format!("{key}/{name}")
            });
        }
        dirs.sort();
        for (rel, path) in dirs.into_iter().rev() {
            stack.push((path, rel));
        }
    }
    names
}

fn is_lock_or_vendor(spec: &RootSpec, rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    if spec.drop_lock_files {
        let base = parts[parts.len() - 1].to_lowercase();
        if base.ends_with(".lock") || LOCK_NAMES.contains(&base.as_str()) {
            return true;
        }
    }
    !spec.exclude_dirs.is_empty()
        && parts[..parts.len() - 1]
            .iter()
            .any(|p| spec.exclude_dirs.iter().any(|d| d == p))
}

/// True when the first bytes of a file hold a NUL byte, or the file cannot be read.
fn proves_binary(path: &Path) -> Option<bool> {
    use std::io::Read;
    let mut buffer = vec![0u8; BINARY_SNIFF_BYTES];
    let mut file = std::fs::File::open(path).ok()?;
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => return None,
        }
    }
    Some(buffer[..filled].contains(&0))
}

/// List one root. It never fails for a missing folder or a failed `git`.
///
/// `ignore` hides a path. `vault_claims` are the globs of the vault claims, on the vault path.
pub fn list_root(
    spec: &RootSpec,
    ignore: &crate::policy::IgnoreList,
    vault_claims: &GlobSet,
) -> RootListing {
    let base = spec.base();
    let mut out = RootListing::default();
    if !base.is_dir() {
        out.diagnostic = Some("root_unavailable");
        return out;
    }
    let names = if spec.git {
        git_names(&base, spec.untracked)
    } else {
        Some(walk_names(spec, &base))
    };
    let Some(names) = names else {
        out.diagnostic = Some("root_list_failed");
        return out;
    };
    let include = GlobSet::new(&spec.include);
    let exclude = GlobSet::new(&spec.exclude);
    let root_claims: Vec<String> = spec
        .claims
        .iter()
        .flat_map(|(_, globs)| globs.iter().cloned())
        .collect();
    let root_claims = GlobSet::new(&root_claims);
    let has_root_claims = !root_claims.is_empty();
    for rel in names {
        if rel.contains('\\') || rel.contains('\n') {
            continue;
        }
        let vault_path = format!("{}/{rel}", spec.name);
        if vault_path.split('/').any(|p| p.is_empty() || p == "..") {
            continue;
        }
        if !include.is_match(&rel) || exclude.is_match(&rel) {
            continue;
        }
        if ignore.ignored(&vault_path) {
            continue;
        }
        if is_lock_or_vendor(spec, &rel) {
            continue;
        }
        let full = base.join(&rel);
        let Ok(meta) = std::fs::symlink_metadata(&full) else {
            continue;
        };
        if !meta.file_type().is_file() {
            continue;
        }
        let is_note = rel.to_lowercase().ends_with(".md");
        let vault_claimed = !vault_claims.is_empty() && vault_claims.is_match(&vault_path);
        let root_claimed = has_root_claims && root_claims.is_match(&rel);
        if !(is_note || vault_claimed || root_claimed) {
            out.link_targets.push((vault_path, meta.len()));
            continue;
        }
        let cap = if is_note {
            spec.note_max_bytes
        } else {
            spec.max_bytes
        };
        if meta.len() > cap {
            // A claim of the root takes code. A big file stays an attachment.
            if !(is_note || vault_claimed) {
                out.link_targets.push((vault_path, meta.len()));
            }
            continue;
        }
        // The read of the file: a binary file moves to tier 2. Any other fault drops the file.
        if spec.binary_sniff {
            match proves_binary(&full) {
                Some(true) => {
                    out.link_targets.push((vault_path, meta.len()));
                    continue;
                }
                Some(false) => {}
                None => continue,
            }
        }
        out.sources.push((vault_path, meta.len()));
    }
    out
}

/// The files of every extra root.
#[derive(Debug, Default)]
pub struct Listing {
    pub sources: Vec<(String, u64)>,
    pub link_targets: Vec<(String, u64)>,
    /// `(root name, fault code)` for each root that could not be listed.
    pub diagnostics: Vec<(String, &'static str)>,
}

impl Listing {
    /// List each root of the settings.
    pub fn of(settings: &crate::config::Settings) -> Self {
        let mut all = Listing::default();
        let vault_claims = GlobSet::new(&settings.claims);
        for spec in &settings.roots {
            let one = list_root(spec, &settings.ignore, &vault_claims);
            all.sources.extend(one.sources);
            all.link_targets.extend(one.link_targets);
            if let Some(code) = one.diagnostic {
                all.diagnostics.push((spec.name.clone(), code));
            }
        }
        all
    }

    /// Where a file of a root lies on disk, from its vault path.
    pub fn real_path(settings: &crate::config::Settings, vault_path: &str) -> Option<PathBuf> {
        let (head, rest) = vault_path.split_once('/')?;
        let spec = settings.roots.iter().find(|r| r.name == head)?;
        Some(spec.base().join(rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Vec<RootSpec>, String> {
        let t: toml::Table = text.parse().unwrap();
        parse_roots_array(&t["root"], "vault.toml")
    }

    #[test]
    fn a_root_needs_a_name_and_an_absolute_path() {
        assert!(parse("[[root]]\nname = \"a\"\npath = \"rel\"").is_err());
        assert!(parse("[[root]]\nname = \"a b\"\npath = \"/x\"").is_err());
        assert!(parse("[[root]]\nname = \"a\"\npath = \"/x\"\nbogus = 1").is_err());
        let ok = parse("[[root]]\nname = \"a\"\npath = \"/x\"").unwrap();
        assert!(ok[0].git && !ok[0].stem_visible && ok[0].exclude.is_empty());
        let walk = parse("[[root]]\nname = \"a\"\npath = \"/x\"\nlister = \"walk\"").unwrap();
        assert!(walk[0].exclude.contains(&".git/**".to_string()));
    }

    #[test]
    fn the_untracked_default_follows_writable() {
        let r = parse("[[root]]\nname = \"a\"\npath = \"/x\"\nwritable = true").unwrap();
        assert!(r[0].untracked);
    }

    #[test]
    fn a_claim_needs_globs() {
        assert!(parse("[[root]]\nname = \"a\"\npath = \"/x\"\nclaims = { c = [] }").is_err());
        let r =
            parse("[[root]]\nname = \"a\"\npath = \"/x\"\nclaims = { c = [\"**/*\"] }").unwrap();
        assert_eq!(r[0].claims[0].0, "c");
    }
}
