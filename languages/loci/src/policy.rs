//! The visibility policy: the ignore list and the id templates. It copies `vault/policy.py` of
//! loci-core (D-051).
//!
//! The ignore list is a file with one `fnmatch` glob on each line. A path has one or more ids:
//! its vault path, and one id for each id template that matches it. A path is hidden when any of
//! its ids matches a pattern, or when a parent folder of that id matches. A fault in the policy
//! refuses the whole vault. It never means "ignore nothing".

use std::path::{Path, PathBuf};

use crate::config::{Glob, Settings};
use crate::text::{nfc, py_strip};

/// The most bytes that an ignore list or a roots file can hold.
pub(crate) const MAX_FILE_BYTES: usize = 1024 * 1024;

/// A template that maps a vault path to a second id.
///
/// The source is a path of literal segments and placeholders. `{name}` matches one segment.
/// `{name:path}` matches one or more segments and must be the last segment. The target is the
/// second id. It can use each placeholder of the source as `{name}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdTemplate {
    segments: Vec<Segment>,
    target: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    Literal(String),
    One(String),
    Rest(String),
}

/// The placeholders of a text: `(name, deep, start, end)`. Only `{[a-z][a-z0-9_]*(:path)?}` counts.
fn placeholders(text: &str) -> Vec<(String, bool, usize, usize)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let name_start = i + 1;
        let mut j = name_start;
        if j < bytes.len() && bytes[j].is_ascii_lowercase() {
            j += 1;
            while j < bytes.len()
                && (bytes[j].is_ascii_lowercase() || bytes[j].is_ascii_digit() || bytes[j] == b'_')
            {
                j += 1;
            }
            let name_end = j;
            let deep = text[j..].starts_with(":path");
            if deep {
                j += ":path".len();
            }
            if j < bytes.len() && bytes[j] == b'}' {
                found.push((text[name_start..name_end].to_string(), deep, i, j + 1));
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    found
}

impl IdTemplate {
    pub fn new(source: &str, target: &str) -> Result<Self, String> {
        let bad = |what: &str| format!("[policy] id_template: {what}");
        if source.is_empty() || target.is_empty() {
            return Err(bad("'from' and 'to' must not be empty"));
        }
        let parts: Vec<&str> = source.split('/').collect();
        if parts.iter().any(|p| p.is_empty()) {
            return Err(bad(&format!("'from' {source:?} has an empty path segment")));
        }
        let mut names: Vec<String> = Vec::new();
        let mut segments = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            let found = placeholders(part);
            let whole = found.len() == 1 && found[0].2 == 0 && found[0].3 == part.len();
            if !whole {
                if part.contains('{') || part.contains('}') {
                    return Err(bad(&format!("bad placeholder in 'from' {source:?}")));
                }
                segments.push(Segment::Literal(part.to_string()));
                continue;
            }
            let (name, deep, _, _) = &found[0];
            if names.contains(name) {
                return Err(bad(&format!(
                    "placeholder {name:?} repeats in 'from' {source:?}"
                )));
            }
            if *deep && index != parts.len() - 1 {
                return Err(bad(&format!(
                    "'{{{name}:path}}' must be the last segment of {source:?}"
                )));
            }
            names.push(name.clone());
            segments.push(if *deep {
                Segment::Rest(name.clone())
            } else {
                Segment::One(name.clone())
            });
        }
        let mut leftover = String::new();
        let mut last = 0;
        for (name, deep, start, end) in placeholders(target) {
            if deep {
                continue;
            }
            if !names.contains(&name) {
                return Err(bad(&format!(
                    "'to' {target:?} uses a placeholder that 'from' does not name"
                )));
            }
            leftover.push_str(&target[last..start]);
            last = end;
        }
        leftover.push_str(&target[last..]);
        if leftover.contains('{') || leftover.contains('}') {
            return Err(bad(&format!("bad placeholder in 'to' {target:?}")));
        }
        Ok(Self {
            segments,
            target: target.to_string(),
        })
    }

    /// The first literal segment of the source, when it has no placeholder.
    fn head(&self) -> Option<&str> {
        match self.segments.first() {
            Some(Segment::Literal(text)) => Some(text),
            _ => None,
        }
    }

    /// The second id of a path, or `None` when the template does not match.
    pub fn render(&self, path: &str) -> Option<String> {
        let parts: Vec<&str> = path.split('/').collect();
        let mut groups: Vec<(&str, String)> = Vec::new();
        let last = self.segments.len() - 1;
        for (index, segment) in self.segments.iter().enumerate() {
            match segment {
                Segment::Literal(text) => {
                    if parts.get(index) != Some(&text.as_str()) {
                        return None;
                    }
                }
                Segment::One(name) => {
                    let part = parts.get(index)?;
                    if part.is_empty() {
                        return None;
                    }
                    groups.push((name, part.to_string()));
                }
                Segment::Rest(name) => {
                    if index != last || parts.len() <= index {
                        return None;
                    }
                    let rest = parts[index..].join("/");
                    if rest.is_empty() {
                        return None;
                    }
                    groups.push((name, rest));
                    return Some(self.fill(&groups));
                }
            }
        }
        if parts.len() != self.segments.len() {
            return None;
        }
        Some(self.fill(&groups))
    }

    fn fill(&self, groups: &[(&str, String)]) -> String {
        let mut out = String::new();
        let mut last = 0;
        for (name, deep, start, end) in placeholders(&self.target) {
            if deep {
                continue;
            }
            out.push_str(&self.target[last..start]);
            if let Some((_, value)) = groups.iter().find(|(n, _)| *n == name) {
                out.push_str(value);
            }
            last = end;
        }
        out.push_str(&self.target[last..]);
        out
    }
}

/// The patterns of an ignore list, and the id templates that give a path more ids.
#[derive(Clone, Debug, Default)]
pub struct IgnoreList {
    patterns: Vec<Glob>,
    templates: Vec<IdTemplate>,
}

impl IgnoreList {
    pub fn new(patterns: Vec<String>, templates: Vec<IdTemplate>) -> Self {
        Self {
            patterns: patterns.iter().map(|p| Glob::new(p)).collect(),
            templates,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    fn matches(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        self.patterns.iter().any(|g| g.is_match(&chars))
    }

    /// A folder matches, or one of its parents does.
    fn folder_ignored(&self, folder: &str) -> bool {
        let mut at = folder;
        loop {
            if self.matches(at) {
                return true;
            }
            match at.rsplit_once('/') {
                Some((parent, _)) if !parent.is_empty() => at = parent,
                _ => return false,
            }
        }
    }

    fn id_ignored(&self, id: &str) -> bool {
        if let Some((folder, _)) = id.rsplit_once('/') {
            if !folder.is_empty() && self.folder_ignored(folder) {
                return true;
            }
        }
        self.matches(id)
    }

    /// True when a pattern matches any id of the path, or a parent folder of an id.
    pub fn ignored(&self, path: &str) -> bool {
        if self.patterns.is_empty() {
            return false;
        }
        let path = nfc(path);
        if self.id_ignored(&path) {
            return true;
        }
        let head = path.split('/').next().unwrap_or("");
        let mut seen: Vec<String> = Vec::new();
        for template in &self.templates {
            let applies = template.head().is_none_or(|h| h == head);
            if !applies {
                continue;
            }
            if let Some(second) = template.render(&path) {
                if second != path && !seen.contains(&second) {
                    if self.id_ignored(&second) {
                        return true;
                    }
                    seen.push(second);
                }
            }
        }
        false
    }
}

/// The patterns of an ignore list text: one for each line, without comments and trailing slashes.
pub fn parse_ignore_text(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| py_strip(line).trim_end_matches('/'))
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(nfc)
        .collect()
}

/// Expand a leading `~` with the home folder, like Python `Path.expanduser`.
pub(crate) fn expand_user(path: &str) -> Result<PathBuf, String> {
    if path == "~" || path.starts_with("~/") {
        let home = std::env::var_os("HOME").ok_or("cannot expand '~': no home folder")?;
        let mut out = PathBuf::from(home);
        if let Some(rest) = path.strip_prefix("~/") {
            out.push(rest);
        }
        return Ok(out);
    }
    if path.starts_with('~') {
        return Err("cannot expand '~user' in a path".to_string());
    }
    Ok(PathBuf::from(path))
}

/// Read the ignore list that `[policy] ignore_file` names. A relative path starts at the vault.
pub fn read_ignore_file(root: &Path, configured: &str) -> Result<Vec<String>, String> {
    let path = expand_user(configured)?;
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    let data = read_limited(&path).map_err(|e| format!("cannot read the ignore list: {e}"))?;
    if data.contains(&0) {
        return Err("the ignore list holds a NUL byte".to_string());
    }
    let text = String::from_utf8(data).map_err(|_| "the ignore list is not valid UTF-8")?;
    Ok(parse_ignore_text(&text))
}

/// Read a file up to one MiB. A larger file is a fault.
pub(crate) fn read_limited(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE_BYTES as u64 + 1).read_to_end(&mut data))
        .map_err(|e| format!("{:?}", e.kind()))?;
    if data.len() > MAX_FILE_BYTES {
        return Err("the file is too large".to_string());
    }
    Ok(data)
}

const POLICY_KEYS: &[&str] = &[
    "ignore_file",
    "note_max_bytes",
    "id_template",
    "write_allow",
    "property_allow",
];

/// Parse the `[policy]` table into `settings`. Any fault is an error. A wrong type or an
/// unknown key never keeps a default, because a typo would then ignore nothing without a sound.
pub fn parse_policy(value: &toml::Value, settings: &mut Settings) -> Result<(), String> {
    let table = value.as_table().ok_or("[policy] must be a table")?;
    let mut unknown: Vec<&str> = table
        .keys()
        .map(String::as_str)
        .filter(|k| !POLICY_KEYS.contains(k))
        .collect();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        return Err(format!("[policy] has unknown keys: {}", unknown.join(", ")));
    }
    if let Some(v) = table.get("ignore_file") {
        match v.as_str() {
            Some(f) if !f.trim().is_empty() => settings.ignore_file = Some(f.to_string()),
            _ => return Err("[policy] ignore_file must be a non-empty string".to_string()),
        }
    }
    if let Some(v) = table.get("note_max_bytes") {
        match v.as_integer() {
            Some(n) if n >= 1 => settings.note_max_bytes = n as u64,
            _ => return Err("[policy] note_max_bytes must be an integer of 1 or more".to_string()),
        }
    }
    if let Some(v) = table.get("id_template") {
        let items = v
            .as_array()
            .ok_or("[policy] id_template must be an array of tables")?;
        for item in items {
            let item = item
                .as_table()
                .filter(|t| t.len() == 2 && t.contains_key("from") && t.contains_key("to"))
                .ok_or("[policy] each id_template needs exactly the keys 'from' and 'to'")?;
            let (Some(from), Some(to)) = (item["from"].as_str(), item["to"].as_str()) else {
                return Err("[policy] id_template 'from' and 'to' must be strings".to_string());
            };
            settings.id_templates.push(IdTemplate::new(from, to)?);
        }
    }
    for key in ["write_allow", "property_allow"] {
        let Some(v) = table.get(key) else {
            continue;
        };
        let items = v
            .as_array()
            .and_then(|a| a.iter().map(|i| i.as_str()).collect::<Option<Vec<_>>>())
            .ok_or(format!("[policy] {key} must be an array of strings"))?;
        for raw in items {
            if key == "write_allow" {
                let pattern = py_strip(raw).trim_end_matches('/');
                if pattern.is_empty()
                    || pattern.starts_with('/')
                    || pattern.split('/').any(|s| s == "..")
                {
                    return Err(format!(
                        "[policy] {key} has a blank, rooted, or '..' pattern"
                    ));
                }
            } else if raw.is_empty()
                || !raw
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!("[policy] {key} holds a key outside [A-Za-z0-9_-]"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(patterns: &[&str], templates: &[(&str, &str)]) -> IgnoreList {
        IgnoreList::new(
            patterns.iter().map(|s| s.to_string()).collect(),
            templates
                .iter()
                .map(|(f, t)| IdTemplate::new(f, t).unwrap())
                .collect(),
        )
    }

    #[test]
    fn a_pattern_matches_the_path_or_a_parent_folder() {
        let l = list(&["secret", "*/.env*"], &[]);
        assert!(l.ignored("secret"));
        assert!(l.ignored("secret/a/b.md"));
        assert!(l.ignored("x/y/.env.local"));
        assert!(!l.ignored("a/secret/b.md"));
    }

    #[test]
    fn a_template_gives_a_path_a_second_id() {
        let l = list(
            &["proj/notes/private"],
            &[("1_Projects/{project}/{rest:path}", "{project}/notes/{rest}")],
        );
        assert!(l.ignored("1_Projects/proj/private/a.md"));
        assert!(!l.ignored("1_Projects/proj/open/a.md"));
        assert!(!l.ignored("1_Projects/proj"));
    }

    #[test]
    fn a_template_needs_valid_placeholders() {
        assert!(IdTemplate::new("a/{x:path}/b", "{x}").is_err());
        assert!(IdTemplate::new("a/{x}", "{y}").is_err());
        assert!(IdTemplate::new("a/{x}/{x}", "{x}").is_err());
        let t = IdTemplate::new("a/{x}", "{x}/n").unwrap();
        assert_eq!(t.render("a/b").as_deref(), Some("b/n"));
        assert_eq!(t.render("a/b/c"), None);
    }

    #[test]
    fn the_policy_refuses_a_typo() {
        let mut s = Settings::default();
        let table = |text: &str| toml::Value::Table(text.parse::<toml::Table>().unwrap());
        assert!(parse_policy(&table("ignorefile = \"x\""), &mut s).is_err());
        assert!(parse_policy(&table("write_allow = [\"/a\"]"), &mut s).is_err());
        assert!(parse_policy(&table("property_allow = [\"a b\"]"), &mut s).is_err());
        assert!(parse_policy(&table("note_max_bytes = 0"), &mut s).is_err());
        assert!(parse_policy(&table("note_max_bytes = 9"), &mut s).is_ok());
        assert_eq!(s.note_max_bytes, 9);
    }

    #[test]
    fn ignore_text_has_no_comments() {
        assert_eq!(
            parse_ignore_text("# c\n\n a/ \nb*\n"),
            vec!["a".to_string(), "b*".to_string()]
        );
    }
}
