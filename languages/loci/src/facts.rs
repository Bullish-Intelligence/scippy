//! Source-bound note facts and incremental resolution. This interface does not scan Markdown.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use scippy_core::ProjectRelativePath;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Classified, Note, Record, Settings, Vault};

type Dependency = (String, String);

/// The versions that define the meaning of the supplied facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub format_version: u32,
    pub extractor_version: u32,
    pub pack_digests: BTreeMap<String, String>,
}

impl Profile {
    fn validate(&self) -> Result<(), String> {
        if self.format_version != 1 || self.extractor_version != 1 {
            return Err("Unsupported note fact version.".into());
        }
        for name in ["markdown", "markdown_inline", "yaml"] {
            let digest = self
                .pack_digests
                .get(name)
                .ok_or("A parser pack digest is missing.")?;
            if !digest.strip_prefix("sha256:").is_some_and(is_hash) {
                return Err("A parser pack digest is invalid.".into());
            }
        }
        Ok(())
    }
}

/// Name-resolution policy. A policy change requires a reset.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub case_sensitive: bool,
    pub alias_resolution: bool,
    pub path_links_only: bool,
    pub id_anchors: bool,
    pub roots: Vec<(String, bool)>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            alias_resolution: true,
            path_links_only: false,
            id_anchors: true,
            roots: Vec::new(),
        }
    }
}

/// Availability of a source. Unavailable sources retain their names but have no anchors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Indexed,
    Unavailable,
    Skipped,
}

/// Facts from a trusted language projection, bound to original source bytes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceFacts {
    pub path: String,
    pub sha256: String,
    pub text: Option<String>,
    pub note: Option<Note>,
    #[serde(default)]
    pub status: Status,
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn span(text: &str, range: (usize, usize)) -> Result<(), String> {
    if range.0 >= range.1 || text.get(range.0..range.1).is_none() {
        return Err("A fact has an invalid UTF-8 byte span.".into());
    }
    Ok(())
}

impl SourceFacts {
    /// Validate transport and source binding. The producer owns semantic correctness.
    pub fn validate(&self) -> Result<(), String> {
        ProjectRelativePath::new(&self.path).map_err(|e| e.to_string())?;
        if !is_hash(&self.sha256) {
            return Err("The source hash is invalid.".into());
        }
        if self.status != Status::Indexed {
            if self.note.is_some() || self.text.is_some() {
                return Err("An unavailable source must contain no facts or text.".into());
            }
            return Ok(());
        }
        let text = self.text.as_deref().ok_or("Source text is missing.")?;
        let note = self.note.as_ref().ok_or("Source facts are missing.")?;
        if text.len() > 2 * 1024 * 1024 {
            return Err("The note exceeds two mebibytes.".into());
        }
        if format!("{:x}", Sha256::digest(text.as_bytes())) != self.sha256 {
            return Err("The source hash does not match the text.".into());
        }
        if note.len != text.len() {
            return Err("The source length does not match the facts.".into());
        }
        for anchor in &note.anchors {
            span(text, anchor.span)?;
            if (anchor.kind == crate::AnchorKind::Heading && !(1..=6).contains(&anchor.level))
                || anchor.text.contains('\0')
            {
                return Err("An anchor has an invalid name or heading level.".into());
            }
        }
        for reference in &note.relations {
            span(text, reference.span)?;
            if let Some((target, anchor)) = reference.parts {
                for part in [target, anchor] {
                    if part.0 > part.1
                        || text.get(part.0..part.1).is_none()
                        || part.0 < reference.span.0
                        || part.1 > reference.span.1
                    {
                        return Err("A reference part lies outside its source span.".into());
                    }
                }
            }
        }
        for (_, start, end) in &note.aliases {
            span(text, (*start, *end))?;
        }
        for tag in &note.tags {
            span(text, tag.span)?;
        }
        Ok(())
    }
}

/// One source replacement or deletion, with an expected previous source hash.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    pub base_sha256: Option<String>,
    pub source: Option<SourceFacts>,
}

/// The records that changed in one atomic index generation.
#[derive(Debug)]
pub struct Update {
    pub generation: u64,
    pub changed: BTreeMap<String, Vec<Record>>,
    pub removed: Vec<String>,
}

/// A reusable resolver over trusted source facts. It owns no files or persistent database.
pub struct NotesIndex {
    profile: Profile,
    policy: Policy,
    sources: BTreeMap<String, SourceFacts>,
    attachments: Vec<String>,
    vault: Vault,
    records: BTreeMap<String, Vec<Record>>,
    dependencies: BTreeMap<String, BTreeSet<Dependency>>,
    generation: u64,
}

impl NotesIndex {
    pub fn new(
        profile: Profile,
        policy: Policy,
        sources: Vec<SourceFacts>,
        attachments: Vec<String>,
    ) -> Result<Self, String> {
        profile.validate()?;
        let mut paths = HashSet::new();
        let mut total = 0;
        for source in &sources {
            source.validate()?;
            total += source.text.as_ref().map_or(0, String::len);
            if !paths.insert(source.path.clone()) {
                return Err("A source path occurs twice.".into());
            }
        }
        if sources.len() > 50_000 || total > 256 * 1024 * 1024 {
            return Err("The source set exceeds the index limits.".into());
        }
        validate_attachments(&attachments)?;
        let mut roots = HashSet::new();
        for (root, _) in &policy.roots {
            ProjectRelativePath::new(root).map_err(|e| e.to_string())?;
            if root.contains('/') || !roots.insert(root) {
                return Err("A root name is invalid or repeated.".into());
            }
        }
        let sources: BTreeMap<_, _> = sources.into_iter().map(|s| (s.path.clone(), s)).collect();
        let vault = build_vault(&policy, &sources, &attachments);
        let mut index = Self {
            profile,
            policy,
            sources,
            attachments,
            vault,
            records: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            generation: 1,
        };
        index.resolve(index.sources.keys().cloned().collect());
        Ok(index)
    }

    /// Assign a later generation after a successful reset.
    pub fn set_generation(&mut self, generation: u64) {
        assert!(generation >= self.generation);
        self.generation = generation;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
    pub fn records(&self) -> &BTreeMap<String, Vec<Record>> {
        &self.records
    }

    /// Apply a validated batch. A refusal leaves the index unchanged.
    pub fn update(
        &mut self,
        base_generation: u64,
        profile: &Profile,
        changes: Vec<Change>,
        attachments: Option<Vec<String>>,
    ) -> Result<Update, String> {
        if base_generation != self.generation {
            return Err("The index generation changed.".into());
        }
        if profile != &self.profile {
            return Err("The source fact profile changed. Reset the index.".into());
        }
        let mut paths = HashSet::new();
        let mut total: usize = self
            .sources
            .values()
            .map(|s| s.text.as_ref().map_or(0, String::len))
            .sum();
        let mut count = self.sources.len();
        for change in &changes {
            ProjectRelativePath::new(&change.path).map_err(|e| e.to_string())?;
            if !paths.insert(change.path.clone()) {
                return Err("A change path occurs twice.".into());
            }
            let previous = self.sources.get(&change.path);
            if previous.map(|s| &s.sha256) != change.base_sha256.as_ref() {
                return Err("The source hash changed.".into());
            }
            if let Some(old) = previous {
                total -= old.text.as_ref().map_or(0, String::len);
                count -= 1;
            }
            if let Some(source) = &change.source {
                source.validate()?;
                if source.path != change.path {
                    return Err("The change names another source path.".into());
                }
                total += source.text.as_ref().map_or(0, String::len);
                count += 1;
            }
        }
        if total > 256 * 1024 * 1024 || count > 50_000 {
            return Err("The source set exceeds the index limits.".into());
        }
        if let Some(attachments) = &attachments {
            validate_attachments(attachments)?;
        }
        // Status and attachment transitions rebuild the lookup maps conservatively.
        let rebuild = attachments.is_some()
            || changes.iter().any(|c| {
                self.sources
                    .get(&c.path)
                    .is_some_and(|s| s.status != Status::Indexed)
                    || c.source
                        .as_ref()
                        .is_some_and(|s| s.status != Status::Indexed)
            });
        let mut buckets = BTreeSet::new();
        let mut removed = Vec::new();
        for change in changes {
            let old = self.sources.remove(&change.path);
            if !rebuild {
                buckets.extend(self.vault.update_note(
                    &change.path,
                    old.as_ref().and_then(|s| s.note.as_ref()),
                    change.source.as_ref().and_then(|s| s.note.as_ref()),
                ));
            }
            match change.source {
                Some(source) => {
                    self.sources.insert(change.path, source);
                }
                None => {
                    self.records.remove(&change.path);
                    self.dependencies.remove(&change.path);
                    removed.push(change.path);
                }
            }
        }
        if let Some(attachments) = attachments {
            self.attachments = attachments;
        }
        let dirty = if rebuild {
            self.vault = build_vault(&self.policy, &self.sources, &self.attachments);
            self.sources.keys().cloned().collect()
        } else {
            self.sources
                .keys()
                .filter(|path| {
                    paths.contains(*path)
                        || self
                            .dependencies
                            .get(*path)
                            .is_some_and(|d| !d.is_disjoint(&buckets))
                })
                .cloned()
                .collect()
        };
        let changed = self.resolve(dirty);
        self.generation += 1;
        Ok(Update {
            generation: self.generation,
            changed,
            removed,
        })
    }

    fn resolve(&mut self, dirty: BTreeSet<String>) -> BTreeMap<String, Vec<Record>> {
        let mut changed = BTreeMap::new();
        self.vault.take_dependencies();
        for path in dirty {
            let records = self.sources[&path]
                .note
                .as_ref()
                .map_or_else(Vec::new, |note| self.vault.records(&path, note));
            self.dependencies
                .insert(path.clone(), self.vault.take_dependencies());
            self.records.insert(path.clone(), records.clone());
            changed.insert(path, records);
        }
        changed
    }
}

fn validate_attachments(attachments: &[String]) -> Result<(), String> {
    if attachments.len() > 100_000 {
        return Err("The attachment set exceeds the index limit.".into());
    }
    let mut seen = HashSet::new();
    for path in attachments {
        ProjectRelativePath::new(path).map_err(|e| e.to_string())?;
        if !seen.insert(path) {
            return Err("An attachment path occurs twice.".into());
        }
    }
    Ok(())
}

fn build_vault(
    policy: &Policy,
    sources: &BTreeMap<String, SourceFacts>,
    attachments: &[String],
) -> Vault {
    let settings = Settings {
        case_sensitive: policy.case_sensitive,
        alias_resolution: policy.alias_resolution,
        path_links_only: policy.path_links_only,
        id_anchors: policy.id_anchors,
        ..Settings::default()
    };
    let classified = Classified {
        sources: sources
            .values()
            .filter(|s| s.status == Status::Indexed)
            .map(|s| s.path.clone())
            .collect(),
        skipped: sources
            .values()
            .filter(|s| s.status != Status::Indexed)
            .map(|s| (s.path.clone(), 0, 0))
            .collect(),
        link_targets: attachments.to_vec(),
        roots: policy.roots.clone(),
    };
    let notes = sources
        .values()
        .filter_map(|s| s.note.as_ref().map(|n| (s.path.clone(), n.clone())))
        .collect();
    Vault::new(&settings, &classified, &notes)
}
