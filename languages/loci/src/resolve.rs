//! The resolution of links. It copies `compiler/relations.py` of loci-core.
//!
//! Resolution is a pure function of the file lists and the anchors. A tie is `ambiguous`, with
//! sorted candidates. Nothing picks a winner.

use std::collections::{BTreeMap, HashMap};

use crate::config::{GlobSet, Settings};
use crate::note::Note;
use crate::scan::{AnchorKind, Kind, Relation, TagOrigin};
use crate::text::{casefold, github_slug, loose_heading, py_strip};

/// `[[#Heading]]` and `[[#^id]]` name the note that holds the link, as `[x](#Heading)` does.
/// loci-core 0.5.0 resolves them to `missing`. Version 0.6.0 resolves them to the same note.
pub const SAME_NOTE_WIKILINKS: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Resolved,
    Missing,
    Ambiguous,
    /// A tag has no resolution.
    None,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Resolved => "resolved",
            State::Missing => "missing",
            State::Ambiguous => "ambiguous",
            State::None => "none",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub state: State,
    pub path: Option<String>,
    pub is_document: bool,
    pub candidates: Vec<String>,
}

impl Resolution {
    fn resolved(path: &str, is_document: bool) -> Self {
        Self {
            state: State::Resolved,
            path: Some(path.to_string()),
            is_document,
            candidates: Vec::new(),
        }
    }

    fn ambiguous(mut candidates: Vec<String>, is_document: bool) -> Self {
        candidates.sort();
        Self {
            state: State::Ambiguous,
            path: None,
            is_document,
            candidates,
        }
    }

    fn missing() -> Self {
        Self {
            state: State::Missing,
            path: None,
            is_document: true,
            candidates: Vec::new(),
        }
    }
}

/// One relation of the export, in the shape of `graph/relations` of loci-core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub source: String,
    pub kind: Kind,
    pub embed: bool,
    pub span: (usize, usize),
    pub target: String,
    pub label: Option<String>,
    pub heading: Option<String>,
    pub block: Option<String>,
    pub property: Option<String>,
    pub origin: Option<&'static str>,
    pub state: State,
    pub target_path: Option<String>,
    pub target_is_document: Option<bool>,
    pub candidates: Vec<String>,
    pub anchor_state: &'static str,
    pub anchor_kind: Option<&'static str>,
    pub anchor_span: Option<(usize, usize)>,
    /// The spans of the target part and of the anchor part of the link. See [`Relation::parts`].
    pub parts: Option<((usize, usize), (usize, usize))>,
}

/// The anchors of one note, ready for lookup.
#[derive(Default)]
struct AnchorIndex {
    /// `(text, level, span)` in document order.
    headings: Vec<(String, u8, (usize, usize))>,
    /// The first span of each heading text, folded or exact as the vault asks.
    heading_first: HashMap<String, (usize, usize)>,
    block_first: HashMap<String, (usize, usize)>,
    /// Footnote ids and HTML ids, by key.
    footnotes: HashMap<String, (usize, usize)>,
    html: HashMap<String, (usize, usize)>,
    /// The note is a captured web page.
    page: bool,
}

/// The files of a vault, classified, with the maps that resolution needs.
pub struct Vault {
    pub settings: Settings,
    case_sensitive: bool,
    by_path: HashMap<String, Vec<String>>,
    by_stem: HashMap<String, Vec<String>>,
    by_alias: HashMap<String, Vec<String>>,
    link_targets: HashMap<String, Vec<String>>,
    link_by_basename: HashMap<String, Vec<String>>,
    anchors: HashMap<String, AnchorIndex>,
    /// The extra roots: name to `stem_visible`.
    roots: HashMap<String, bool>,
    /// The sources and the attachments of every extra root, by (root, key), for a link that is
    /// written inside that root.
    by_root_stem: HashMap<(String, String), Vec<String>>,
    by_root_basename: HashMap<(String, String), Vec<String>>,
}

/// The files that count in a vault.
#[derive(Debug, Default)]
pub struct Classified {
    /// The documents, sorted by path: the notes, and the files that an analyzer claims.
    pub sources: Vec<String>,
    /// The files that are not documents and not excluded, sorted by path.
    pub link_targets: Vec<String>,
    /// The extra roots: name and `stem_visible`.
    pub roots: Vec<(String, bool)>,
}

/// Say whether a file or one of its folders matches an exclude glob.
pub struct Excluder {
    exclude: GlobSet,
    folders: HashMap<String, bool>,
}

impl Excluder {
    pub fn new(settings: &Settings) -> Self {
        Self {
            exclude: GlobSet::new(&settings.exclude),
            folders: HashMap::new(),
        }
    }

    /// True when the folder, or one of its parents, matches.
    pub fn folder(&mut self, folder: &str) -> bool {
        if folder.is_empty() {
            return false;
        }
        if let Some(&hit) = self.folders.get(folder) {
            return hit;
        }
        let parent = folder.rsplit_once('/').map_or("", |(p, _)| p);
        let hit = self.exclude.is_match(folder) || self.folder(parent);
        self.folders.insert(folder.to_string(), hit);
        hit
    }

    pub fn file(&mut self, path: &str) -> bool {
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        self.exclude.is_match(path) || self.folder(parent)
    }
}

/// Sort the files of a vault into documents and link targets.
///
/// `files` lists the files of the vault root with their sizes. A file of an extra root is in
/// `extra`, already sorted into its tiers by [`crate::roots::list_root`].
pub fn classify(
    files: &[(String, u64)],
    settings: &Settings,
    extra: &crate::roots::Listing,
) -> Classified {
    let include = GlobSet::new(&settings.include);
    let claims = GlobSet::new(&settings.claims);
    let mut excluder = Excluder::new(settings);
    let mut out = Classified::default();
    for (path, size) in files {
        if excluder.file(path) || settings.ignore.ignored(path) {
            continue;
        }
        let is_md = path.to_lowercase().ends_with(".md");
        let claimed = !claims.is_empty() && claims.is_match(path);
        if (is_md && include.is_match(path)) || (claimed && !is_manifest(path)) {
            // A note, or a file that an analyzer claims, over the size cap is in neither tier.
            if *size > settings.note_max_bytes {
                continue;
            }
            out.sources.push(path.clone());
        } else if !is_manifest(path) {
            out.link_targets.push(path.clone());
        }
    }
    out.sources
        .extend(extra.sources.iter().map(|(p, _)| p.clone()));
    out.link_targets
        .extend(extra.link_targets.iter().map(|(p, _)| p.clone()));
    out.sources.sort();
    out.link_targets.sort();
    out.roots = settings
        .roots
        .iter()
        .map(|r| (r.name.clone(), r.stem_visible))
        .collect();
    out
}

/// The manifests of a vault are sources of their own kind and never link targets.
fn is_manifest(path: &str) -> bool {
    path == ".loci/vault.toml"
        || path
            .strip_prefix(".loci/workspaces/")
            .is_some_and(|rest| !rest.contains('/') && rest.ends_with(".yaml"))
}

fn stem_of(target: &str) -> &str {
    let name = target.rsplit('/').next().unwrap_or(target);
    if name.to_lowercase().ends_with(".md") {
        &name[..name.len() - 3]
    } else {
        name
    }
}

impl Vault {
    fn norm(&self, s: &str) -> String {
        if self.case_sensitive {
            s.to_string()
        } else {
            casefold(s)
        }
    }

    pub fn new(
        settings: &Settings,
        classified: &Classified,
        notes: &BTreeMap<String, Note>,
    ) -> Self {
        let case_sensitive = settings.case_sensitive;
        let mut vault = Self {
            settings: settings.clone(),
            case_sensitive,
            by_path: HashMap::new(),
            by_stem: HashMap::new(),
            by_alias: HashMap::new(),
            link_targets: HashMap::new(),
            link_by_basename: HashMap::new(),
            anchors: HashMap::new(),
            roots: classified.roots.iter().cloned().collect(),
            by_root_stem: HashMap::new(),
            by_root_basename: HashMap::new(),
        };
        for path in &classified.sources {
            let npath = vault.norm(path);
            let stem = vault.norm(stem_of(path));
            vault.by_path.entry(npath).or_default().push(path.clone());
            let visible = match vault.root_of(path) {
                Some(root) => {
                    vault
                        .by_root_stem
                        .entry((root.to_string(), stem.clone()))
                        .or_default()
                        .push(path.clone());
                    vault.roots[root]
                }
                None => true,
            };
            if visible {
                vault.by_stem.entry(stem).or_default().push(path.clone());
            }
            let Some(note) = notes.get(path) else {
                continue;
            };
            let mut seen: Vec<&str> = Vec::new();
            // A root with `stem_visible = false` keeps its aliases out of bare lookups too.
            let alias_visible = vault.root_of(path).is_none_or(|r| vault.roots[r]);
            for (alias, _, _) in &note.aliases {
                if !alias_visible {
                    break;
                }
                if seen.contains(&alias.as_str()) {
                    continue;
                }
                seen.push(alias);
                let key = vault.norm(alias);
                vault.by_alias.entry(key).or_default().push(path.clone());
            }
            let mut index = AnchorIndex::default();
            for anchor in &note.anchors {
                match anchor.kind {
                    AnchorKind::Heading => {
                        index
                            .headings
                            .push((anchor.text.clone(), anchor.level, anchor.span));
                        let key = vault.norm(&anchor.text);
                        index.heading_first.entry(key).or_insert(anchor.span);
                    }
                    AnchorKind::Block => {
                        let key = vault.norm(&anchor.text);
                        index.block_first.entry(key).or_insert(anchor.span);
                    }
                    AnchorKind::Footnote => {
                        let key = vault.norm(&anchor.text);
                        index.footnotes.entry(key).or_insert(anchor.span);
                    }
                    AnchorKind::Html => {
                        let key = vault.norm(&anchor.text);
                        index.html.entry(key).or_insert(anchor.span);
                    }
                }
            }
            index.page = note.page;
            vault.anchors.insert(path.clone(), index);
        }
        for path in &classified.link_targets {
            let npath = vault.norm(path);
            let stem = vault.norm(stem_of(path));
            vault
                .link_targets
                .entry(npath)
                .or_default()
                .push(path.clone());
            let visible = match vault.root_of(path) {
                Some(root) => {
                    vault
                        .by_root_basename
                        .entry((root.to_string(), stem.clone()))
                        .or_default()
                        .push(path.clone());
                    vault.roots[root]
                }
                None => true,
            };
            if visible {
                vault
                    .link_by_basename
                    .entry(stem)
                    .or_default()
                    .push(path.clone());
            }
        }
        vault
    }

    /// The extra root that holds a path: the first segment names it, and a folder follows.
    fn root_of<'a>(&self, path: &'a str) -> Option<&'a str> {
        let (head, _) = path.split_once('/')?;
        self.roots.contains_key(head).then_some(head)
    }

    /// The explicit path of an already normalized target: a document, then an attachment.
    fn path_hit(&self, t: &str) -> Option<Resolution> {
        let alternative = if t.to_lowercase().ends_with(".md") {
            t[..t.len() - 3].to_string()
        } else {
            format!("{t}.md")
        };
        for candidate in [t, alternative.as_str()] {
            if let Some(found) = self.by_path.get(candidate) {
                return Some(match found.len() {
                    1 => Resolution::resolved(&found[0], true),
                    _ => Resolution::ambiguous(found.clone(), true),
                });
            }
        }
        let found = self.link_targets.get(t)?;
        Some(match found.len() {
            1 => Resolution::resolved(&found[0], false),
            _ => Resolution::ambiguous(found.clone(), false),
        })
    }

    fn from_list(found: Option<&Vec<String>>, is_document: bool) -> Option<Resolution> {
        let found = found?;
        match found.len() {
            0 => None,
            1 => Some(Resolution::resolved(&found[0], is_document)),
            _ => Some(Resolution::ambiguous(found.clone(), is_document)),
        }
    }

    /// Resolve the raw target of a wikilink, an embed, or a front matter link.
    ///
    /// A link in a file of an extra root resolves root-first: the path below that root, then a
    /// name inside that root, and only then the rules of the vault.
    pub fn resolve_one(&self, target: &str, source: &str) -> Resolution {
        let t = self.norm(target);
        let root = self.root_of(source);
        if let Some(root) = root {
            if let Some(hit) = self.path_hit(&self.norm(&format!("{root}/{target}"))) {
                return hit;
            }
        }
        if let Some(hit) = self.path_hit(&t) {
            return hit;
        }
        if !self.settings.path_links_only {
            let stem = self.norm(stem_of(target));
            if let Some(root) = root {
                let key = (root.to_string(), stem.clone());
                if let Some(hit) = Self::from_list(self.by_root_stem.get(&key), true) {
                    return hit;
                }
            }
            if let Some(hit) = Self::from_list(self.by_stem.get(&stem), true) {
                return hit;
            }
        }
        if self.settings.alias_resolution {
            if let Some(hit) = Self::from_list(self.by_alias.get(&t), true) {
                return hit;
            }
        }
        let key = self.norm(stem_of(target));
        if let Some(root) = root {
            let scoped = (root.to_string(), key.clone());
            if let Some(hit) = Self::from_list(self.by_root_basename.get(&scoped), false) {
                return hit;
            }
        }
        if let Some(hit) = Self::from_list(self.link_by_basename.get(&key), false) {
            return hit;
        }
        Resolution::missing()
    }

    /// Resolve the target of a Markdown link: a path, next to the source and then from the root.
    pub fn resolve_markdown(&self, target: &str, source: &str) -> Resolution {
        if target.is_empty() {
            return self
                .path_hit(&self.norm(source))
                .unwrap_or_else(Resolution::missing);
        }
        let base = source.rsplit_once('/').map_or("", |(b, _)| b);
        let mut named = markdown_candidates(target, base);
        // A link in an extra root also names a path below that root.
        if let Some(root) = self.root_of(source) {
            if !target.starts_with('/') {
                if let Some(scoped) = join_normalized(root, target) {
                    if !scoped.is_empty() && !named.contains(&scoped) {
                        named.insert(1.min(named.len()), scoped);
                    }
                }
            }
        }
        for candidate in named {
            if let Some(hit) = self.path_hit(&self.norm(&candidate)) {
                return hit;
            }
        }
        Resolution::missing()
    }

    /// `(anchor_state, anchor_kind, span)`. It is separate from the state of the link.
    fn resolve_anchor(
        &self,
        resolution: &Resolution,
        heading: Option<&str>,
        block: Option<&str>,
        markdown: bool,
    ) -> (&'static str, Option<&'static str>, Option<(usize, usize)>) {
        if heading.is_none() && block.is_none() {
            return ("none", None, None);
        }
        let (State::Resolved, Some(path)) = (resolution.state, resolution.path.as_deref()) else {
            return ("none", None, None);
        };
        let index = self.anchors.get(path);
        if let Some(block) = block {
            let wanted = block.strip_prefix('^').unwrap_or(block);
            let key = self.norm(wanted);
            if let Some(span) = index.and_then(|i| i.block_first.get(&key)) {
                return ("resolved", Some("block"), Some(*span));
            }
            return self.id_anchor(index, None, Some(block), markdown);
        }
        let wanted = heading.unwrap_or("");
        let key = self.norm(wanted);
        if let Some(index) = index {
            if let Some(span) = index.heading_first.get(&key) {
                return ("resolved", Some("heading"), Some(*span));
            }
            let segments: Vec<&str> = wanted
                .split('#')
                .map(py_strip)
                .filter(|s| !s.is_empty())
                .collect();
            if segments.len() > 1 {
                if let Some(span) = self.heading_chain(&index.headings, &segments) {
                    return ("resolved", Some("heading"), Some(span));
                }
            }
            let loose = loose_heading(wanted, self.case_sensitive);
            if !loose.is_empty() {
                for (text, _, span) in &index.headings {
                    if loose_heading(text, self.case_sensitive) == loose {
                        return ("resolved", Some("heading"), Some(*span));
                    }
                }
            }
            if markdown {
                let slug = github_slug(wanted);
                if !slug.is_empty() {
                    if let Some(span) = heading_slugs(&index.headings).get(&slug) {
                        return ("resolved", Some("heading"), Some(*span));
                    }
                }
            }
        }
        self.id_anchor(index, Some(wanted), None, markdown)
    }

    /// The `id` rules, for a link whose heading or block matched nothing.
    ///
    /// 1. An HTML `id` of the note, or an `<a name>`, equals the fragment.
    /// 2. A footnote definition matches the fragment, with or without a known prefix.
    ///    A block form `^x` names the footnote `x`.
    /// 3. A Markdown link in a captured web page: `unchecked`.
    fn id_anchor(
        &self,
        index: Option<&AnchorIndex>,
        heading: Option<&str>,
        block: Option<&str>,
        markdown: bool,
    ) -> (&'static str, Option<&'static str>, Option<(usize, usize)>) {
        const MISSING: (&str, Option<&str>, Option<(usize, usize)>) = ("missing", None, None);
        let Some(index) = index.filter(|_| self.settings.id_anchors) else {
            return MISSING;
        };
        if let Some(block) = block {
            let wanted = block.strip_prefix('^').unwrap_or(block);
            return match index.footnotes.get(&self.norm(wanted)) {
                Some(span) => ("resolved", Some("id"), Some(*span)),
                None => MISSING,
            };
        }
        let key = self.norm(heading.unwrap_or(""));
        if let Some(span) = index.html.get(&key) {
            return ("resolved", Some("id"), Some(*span));
        }
        const PREFIXES: [&str; 11] = [
            "",
            "user-content-fnref-",
            "user-content-fn-",
            "fnref:",
            "fnref-",
            "fnref_",
            "fnref",
            "fn:",
            "fn-",
            "fn_",
            "fn",
        ];
        for prefix in PREFIXES {
            let rest = if prefix.is_empty() {
                key.as_str()
            } else {
                let prefix = if self.case_sensitive {
                    prefix.to_string()
                } else {
                    casefold(prefix)
                };
                match key.strip_prefix(prefix.as_str()) {
                    Some(rest) => rest,
                    None => continue,
                }
            };
            if !rest.is_empty() {
                if let Some(span) = index.footnotes.get(rest) {
                    return ("resolved", Some("id"), Some(*span));
                }
            }
        }
        if markdown && index.page {
            return ("unchecked", Some("id"), None);
        }
        MISSING
    }

    /// The span of the last heading of a chain `A#B#C`: each is a descendant of the one before.
    fn heading_chain(
        &self,
        headings: &[(String, u8, (usize, usize))],
        segments: &[&str],
    ) -> Option<(usize, usize)> {
        let ends: Vec<usize> = (0..headings.len())
            .map(|i| {
                let level = headings[i].1;
                let mut j = i + 1;
                while j < headings.len() && headings[j].1 > level {
                    j += 1;
                }
                j
            })
            .collect();
        let same = |text: &str, wanted: &str| {
            let fold = |s: &str| self.norm(s);
            fold(text) == fold(wanted)
                || loose_heading(text, self.case_sensitive)
                    == loose_heading(wanted, self.case_sensitive)
        };
        fn find(
            headings: &[(String, u8, (usize, usize))],
            ends: &[usize],
            segments: &[&str],
            same: &dyn Fn(&str, &str) -> bool,
            lo: usize,
            hi: usize,
            k: usize,
        ) -> Option<(usize, usize)> {
            for i in lo..hi {
                if same(&headings[i].0, segments[k]) {
                    if k == segments.len() - 1 {
                        return Some(headings[i].2);
                    }
                    if let Some(inner) = find(headings, ends, segments, same, i + 1, ends[i], k + 1)
                    {
                        return Some(inner);
                    }
                }
            }
            None
        }
        find(headings, &ends, segments, &same, 0, headings.len(), 0)
    }

    /// Resolve one relation of a note.
    pub fn resolve_relation(&self, source: &str, r: &Relation) -> Record {
        let markdown = r.kind.is_markdown();
        let same_note = SAME_NOTE_WIKILINKS
            && !markdown
            && r.target.is_empty()
            && (r.heading.is_some() || r.block.is_some());
        let resolution = if markdown || same_note {
            self.resolve_markdown(&r.target, source)
        } else {
            self.resolve_one(&r.target, source)
        };
        let (anchor_state, anchor_kind, anchor_span) = self.resolve_anchor(
            &resolution,
            r.heading.as_deref(),
            r.block.as_deref(),
            markdown,
        );
        let resolved = resolution.state == State::Resolved;
        Record {
            source: source.to_string(),
            kind: r.kind,
            embed: matches!(r.kind, Kind::Embed | Kind::MarkdownEmbed) || r.embed,
            span: r.span,
            target: r.target.clone(),
            label: r.label.clone(),
            heading: r.heading.clone(),
            block: r
                .block
                .as_ref()
                .map(|b| b.strip_prefix('^').unwrap_or(b).to_string()),
            property: r.prop.clone(),
            origin: None,
            state: resolution.state,
            target_path: resolution.path.clone(),
            target_is_document: resolved.then_some(resolution.is_document),
            candidates: resolution.candidates,
            anchor_state,
            anchor_kind,
            anchor_span,
            parts: r.parts,
        }
    }

    /// The records of one note: its links, then its tags, sorted by span.
    pub fn records(&self, path: &str, note: &Note) -> Vec<Record> {
        let mut out: Vec<Record> = note
            .relations
            .iter()
            .map(|r| self.resolve_relation(path, r))
            .collect();
        for tag in &note.tags {
            out.push(Record {
                source: path.to_string(),
                kind: Kind::Tag,
                embed: false,
                span: tag.span,
                target: tag.tag.clone(),
                label: None,
                heading: None,
                block: None,
                property: tag.prop.clone(),
                origin: Some(match tag.origin {
                    TagOrigin::Body => "body",
                    TagOrigin::Frontmatter => "frontmatter",
                }),
                state: State::None,
                target_path: None,
                target_is_document: None,
                candidates: Vec::new(),
                anchor_state: "none",
                anchor_kind: None,
                anchor_span: None,
                parts: None,
            });
        }
        out.sort_by_key(|r| r.span);
        out
    }
}

/// Slug -> span of the first heading that has it. A repeated slug gets `-1`, `-2`, as on GitHub.
fn heading_slugs(headings: &[(String, u8, (usize, usize))]) -> HashMap<String, (usize, usize)> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut found = HashMap::new();
    for (text, _, span) in headings {
        let base = github_slug(text);
        let count = *seen.get(&base).unwrap_or(&0);
        seen.insert(base.clone(), count + 1);
        found.entry(base.clone()).or_insert(*span);
        if count > 0 {
            found.entry(format!("{base}-{count}")).or_insert(*span);
        }
    }
    found
}

/// Join a Markdown link path to a folder and remove `.` and `..`. A leading `/` is the root.
/// Return `None` when the path climbs above the root.
fn join_normalized(base: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = if target.starts_with('/') || base.is_empty() {
        Vec::new()
    } else {
        base.split('/').collect()
    };
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// The paths that a Markdown link can name, in order: next to the source, then from the root.
fn markdown_candidates(target: &str, base: &str) -> Vec<String> {
    let mut found: Vec<Option<String>> = Vec::new();
    if !target.starts_with('/') {
        found.push(join_normalized(base, target));
    }
    found.push(join_normalized("", target));
    let mut out: Vec<String> = Vec::new();
    for c in found.into_iter().flatten() {
        if !c.is_empty() && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_handles_dots() {
        assert_eq!(join_normalized("a/b", "../c.md").as_deref(), Some("a/c.md"));
        assert_eq!(join_normalized("", "../c.md"), None);
        assert_eq!(join_normalized("a", "/c.md").as_deref(), Some("c.md"));
    }

    #[test]
    fn candidates_in_order() {
        assert_eq!(markdown_candidates("x.md", "a"), ["a/x.md", "x.md"]);
        assert_eq!(markdown_candidates("/x.md", "a"), ["x.md"]);
    }
}
