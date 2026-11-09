//! The vault settings: `.loci/vault.toml`, read with the rules of loci-core.

use std::path::Path;

/// The default `discovery.exclude` of a vault.
const DEFAULT_EXCLUDE: &[&str] = &[
    ".obsidian/**",
    ".loci/**",
    ".git/**",
    ".trash/**",
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
    "node_modules/**",
    "target/**",
    "dist/**",
    "build/**",
    "result",
    "result-*",
];

/// The default size limit of a note in bytes.
const DEFAULT_NOTE_MAX_BYTES: u64 = 1024 * 1024;

/// The settings that decide which files count and how links resolve.
#[derive(Clone, Debug)]
pub struct Settings {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// `discovery.link_targets`. loci-core 0.5.0 reads it and never applies it.
    /// Every file that is not excluded and not a note is a link target.
    pub link_targets: Vec<String>,
    pub case_sensitive: bool,
    pub alias_resolution: bool,
    pub path_links_only: bool,
    pub markdown_links: bool,
    pub frontmatter_links: bool,
    pub inline_tags: bool,
    /// `[diagnostics] id_anchors`: a fragment can name a footnote or an HTML id.
    pub id_anchors: bool,
    pub note_max_bytes: u64,
    /// Things in the file that this reader does not support. Each one is a warning.
    pub unsupported: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            include: vec!["**/*.md".to_string()],
            exclude: DEFAULT_EXCLUDE.iter().map(|s| s.to_string()).collect(),
            link_targets: vec!["**/*".to_string()],
            case_sensitive: false,
            alias_resolution: true,
            path_links_only: false,
            markdown_links: true,
            frontmatter_links: true,
            inline_tags: true,
            id_anchors: true,
            note_max_bytes: DEFAULT_NOTE_MAX_BYTES,
            unsupported: Vec::new(),
        }
    }
}

impl Settings {
    /// Read `.loci/vault.toml` below `root`. Return `None` when the vault has no manifest.
    pub fn read(root: &Path) -> Result<Option<Self>, String> {
        let path = root.join(".loci").join("vault.toml");
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read vault.toml: {e}"))?;
        let text = String::from_utf8(bytes).map_err(|_| "vault.toml is not UTF-8".to_string())?;
        Self::parse(&text).map(Some)
    }

    /// Parse the manifest. A wrong type keeps the default, as in loci-core.
    pub fn parse(text: &str) -> Result<Self, String> {
        let data: toml::Table = text
            .parse()
            .map_err(|e| format!("vault.toml does not parse: {e}"))?;
        if data.get("schema").and_then(|v| v.as_integer()) != Some(1) {
            return Err("vault.toml has an unsupported schema".to_string());
        }
        if !data.get("id").is_some_and(|v| v.is_str()) {
            return Err("vault.toml has no id".to_string());
        }
        let mut s = Self::default();
        let section = |name: &str| data.get(name).and_then(|v| v.as_table());
        let strings = |table: &toml::Table, key: &str| -> Option<Vec<String>> {
            let list = table.get(key)?.as_array()?;
            list.iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
        };
        let flag = |table: &toml::Table, key: &str| table.get(key).and_then(|v| v.as_bool());
        if let Some(t) = section("discovery") {
            if let Some(v) = strings(t, "include") {
                s.include = v;
            }
            if let Some(v) = strings(t, "exclude") {
                s.exclude = v;
            }
            if let Some(v) = strings(t, "link_targets") {
                s.link_targets = v;
            }
        }
        if let Some(t) = section("resolution") {
            s.case_sensitive = flag(t, "case_sensitive").unwrap_or(s.case_sensitive);
            s.alias_resolution = flag(t, "alias_resolution").unwrap_or(s.alias_resolution);
            s.path_links_only = flag(t, "path_links_only").unwrap_or(s.path_links_only);
        }
        if let Some(t) = section("extract") {
            s.markdown_links = flag(t, "markdown_links").unwrap_or(s.markdown_links);
            s.frontmatter_links = flag(t, "frontmatter_links").unwrap_or(s.frontmatter_links);
            s.inline_tags = flag(t, "inline_tags").unwrap_or(s.inline_tags);
        }
        if let Some(t) = section("diagnostics") {
            s.id_anchors = flag(t, "id_anchors").unwrap_or(s.id_anchors);
        }
        if let Some(t) = data.get("policy") {
            let Some(t) = t.as_table() else {
                return Err("[policy] must be a table".to_string());
            };
            if let Some(v) = t.get("note_max_bytes") {
                match v.as_integer() {
                    Some(n) if n >= 1 => s.note_max_bytes = n as u64,
                    _ => return Err("[policy] note_max_bytes must be 1 or more".to_string()),
                }
            }
            if t.contains_key("ignore_file") {
                s.unsupported.push("[policy] ignore_file".to_string());
            }
        }
        for key in ["root", "roots_file"] {
            if data.contains_key(key) {
                s.unsupported.push(format!("{key} (extra roots)"));
            }
        }
        Ok(s)
    }
}

/// One compiled glob in the style of Python `fnmatch`.
#[derive(Clone, Debug)]
enum Token {
    Star,
    Any,
    Literal(char),
    Class {
        negate: bool,
        items: Vec<(char, char)>,
    },
}

#[derive(Clone, Debug)]
struct Glob(Vec<Token>);

impl Glob {
    /// Translate like `fnmatch.translate`: `*` crosses `/`.
    fn new(pattern: &str) -> Self {
        let chars: Vec<char> = pattern.chars().collect();
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            i += 1;
            match c {
                '*' => {
                    if !matches!(tokens.last(), Some(Token::Star)) {
                        tokens.push(Token::Star);
                    }
                }
                '?' => tokens.push(Token::Any),
                '[' => {
                    let mut j = i;
                    if j < chars.len() && chars[j] == '!' {
                        j += 1;
                    }
                    if j < chars.len() && chars[j] == ']' {
                        j += 1;
                    }
                    while j < chars.len() && chars[j] != ']' {
                        j += 1;
                    }
                    if j >= chars.len() {
                        tokens.push(Token::Literal('['));
                    } else {
                        let mut stuff: Vec<char> = chars[i..j].to_vec();
                        let negate = stuff.first() == Some(&'!');
                        if negate {
                            stuff.remove(0);
                        }
                        let mut items = Vec::new();
                        let mut k = 0;
                        while k < stuff.len() {
                            if k + 2 < stuff.len() && stuff[k + 1] == '-' {
                                items.push((stuff[k], stuff[k + 2]));
                                k += 3;
                            } else {
                                items.push((stuff[k], stuff[k]));
                                k += 1;
                            }
                        }
                        tokens.push(Token::Class { negate, items });
                        i = j + 1;
                    }
                }
                other => tokens.push(Token::Literal(other)),
            }
        }
        Self(tokens)
    }

    fn is_match(&self, text: &[char]) -> bool {
        let tokens = &self.0;
        let (mut t, mut p) = (0usize, 0usize);
        let mut backtrack: Option<(usize, usize)> = None;
        let step = |token: &Token, c: char| match token {
            Token::Any => true,
            Token::Literal(l) => *l == c,
            Token::Class { negate, items } => {
                items.iter().any(|&(lo, hi)| lo <= c && c <= hi) != *negate
            }
            Token::Star => false,
        };
        while t < text.len() {
            if p < tokens.len() && matches!(tokens[p], Token::Star) {
                backtrack = Some((p, t));
                p += 1;
            } else if p < tokens.len() && step(&tokens[p], text[t]) {
                p += 1;
                t += 1;
            } else if let Some((bp, bt)) = backtrack {
                backtrack = Some((bp, bt + 1));
                p = bp + 1;
                t = bt + 1;
            } else {
                return false;
            }
        }
        while p < tokens.len() && matches!(tokens[p], Token::Star) {
            p += 1;
        }
        p == tokens.len()
    }
}

/// A list of globs with the variants of loci-core: `**/x` also matches `x`, and `x/**` also `x`.
#[derive(Clone, Debug, Default)]
pub struct GlobSet(Vec<Glob>);

impl GlobSet {
    pub fn new(patterns: &[String]) -> Self {
        let mut globs = Vec::new();
        for pattern in patterns {
            globs.push(Glob::new(pattern));
            if let Some(rest) = pattern.strip_prefix("**/") {
                globs.push(Glob::new(rest));
            }
            if let Some(rest) = pattern.strip_suffix("/**") {
                globs.push(Glob::new(rest));
            }
        }
        Self(globs)
    }

    pub fn is_match(&self, path: &str) -> bool {
        let chars: Vec<char> = path.chars().collect();
        self.0.iter().any(|g| g.is_match(&chars))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(p: &[&str]) -> GlobSet {
        GlobSet::new(&p.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn star_crosses_slash_and_double_star_matches_the_root() {
        let md = set(&["**/*.md"]);
        assert!(md.is_match("a.md"));
        assert!(md.is_match("a/b/c.md"));
        assert!(!md.is_match("a.txt"));
        let ex = set(&["target/**", "result", "result-*"]);
        assert!(ex.is_match("target"));
        assert!(ex.is_match("target/x/y"));
        assert!(!ex.is_match("a/target"));
        assert!(ex.is_match("result-1"));
    }

    #[test]
    fn classes_work() {
        assert!(set(&["a[bc]d"]).is_match("acd"));
        assert!(!set(&["a[!bc]d"]).is_match("acd"));
        assert!(set(&["a?d"]).is_match("axd"));
    }

    #[test]
    fn defaults_and_types() {
        let s = Settings::parse("schema = 1\nid = \"x\"\n[resolution]\ncase_sensitive = \"no\"\n")
            .unwrap();
        assert!(!s.case_sensitive);
        assert!(s.exclude.contains(&".loci/**".to_string()));
        assert!(Settings::parse("schema = 2\nid = \"x\"").is_err());
    }
}
