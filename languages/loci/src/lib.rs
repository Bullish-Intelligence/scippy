//! The Loci language pack: notes with wikilinks, Markdown links, tags, and front matter links.
//!
//! The pack follows `loci-core` exactly. The scanner is a port of the walk of loci-core, and the
//! resolver copies its order and its states. A tie is `ambiguous`, and nothing picks a winner.
//! The pack is native: it reads its files and resolves its references itself, and the core
//! writes the index.
//!
//! # Selection
//!
//! The pack takes part in a run only when the project root has `.loci/vault.toml`.
//! It owns the files that end in `.md`. A project without that file gives its `.md` files to
//! no pack, as before. The other packs keep their files in a mixed project.
//! The pack reads `discovery.include`, `discovery.exclude`, `[resolution]`, `[extract]`,
//! `[diagnostics] id_anchors`, and `[policy] note_max_bytes`. `discovery.link_targets` is read and never applied, as in
//! loci-core. `[[root]]`, `roots_file`, and `[policy] ignore_file` are not supported.
//! Each one raises a warning.
//!
//! # Symbols
//!
//! The package is `loci <vault> .`. `<vault>` is the name of the project folder.
//! A descriptor path starts with the path of the note. Each folder and the file are a namespace.
//!
//! | Thing | Descriptor | Kind |
//! |---|---|---|
//! | note | `dir/Note.md/` | file |
//! | heading | `dir/Note.md/Heading text.` | namespace |
//! | block id | ``dir/Note.md/`^id`.`` | key |
//! | footnote | ``dir/Note.md/`fn:x`.`` | key |
//! | HTML id | ``dir/Note.md/`id:x`.`` | key |
//! | alias | ``dir/Note.md/`alias:Name`.`` | key |
//! | attachment | `dir/pic.png/` | file |
//! | tag | `a/b/` in the package `loci tags .` | key |
//!
//! A heading is keyed by its text. A footnote definition and an HTML id are anchors like a block id. The second heading with the same text is `text~2`, and so on.
//! A tag is keyed by its case-folded text, and `/` nests it.
//!
//! # Occurrences
//!
//! The definition of a note is the empty range at 0:0. The definition of a heading is the heading
//! line. The definition of a block id is the caret and the id. The definition of an alias is the
//! front matter item. An attachment is a stub document with a definition at 0:0.
//!
//! A link is one reference over the whole link. It names the most exact target: the heading or
//! the block when the anchor resolves, and the note otherwise. A missing anchor still names the
//! note. An embed has the read-access role. A link that is `missing` or `ambiguous` has no
//! occurrence. A tag is a reference to a symbol of the tag package.

mod config;
mod front;
mod note;
mod resolve;
mod scan;
mod text;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use scippy_core::{
    DeclKind, Descriptor, Diagnostic, Engine, ExternalIdentity, Identity, LanguagePack,
    NativeAnalysis, NativeDeclaration, NativeDocument, NativeInput, NativeOutput, NativeReference,
    NativeTarget, Package, ProjectFile, ProjectModel, ProjectRelativePath, Reach, ReferenceRole,
    Scope, Severity, Stage, SymbolKind, VirtualSymbol, WalkRules,
};

pub use config::{GlobSet, Settings};
pub use note::{Note, analyze_note};
pub use resolve::{Classified, Record, Resolution, State, Vault, classify};
pub use scan::{AnchorKind, Kind, TagOrigin};

use resolve::Excluder;
use text::casefold;

/// The Loci language pack.
pub struct LociPack;

impl LanguagePack for LociPack {
    fn id(&self) -> &'static str {
        "loci"
    }

    fn scip_language(&self) -> &'static str {
        "Markdown"
    }

    fn accepts(&self, path: &Path) -> bool {
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
    }

    fn engine(&self) -> Option<Engine<'_>> {
        Some(Engine::Native(self))
    }

    fn enabled(&self, root: &Path) -> bool {
        root.join(".loci").join("vault.toml").is_file()
    }

    fn walk_rules(&self, root: &Path) -> WalkRules {
        // The walker ignores some folders by default. The vault decides for its own files.
        let settings = Settings::read(root).ok().flatten().unwrap_or_default();
        let excluder = Mutex::new(Excluder::new(&settings));
        WalkRules {
            enter_ignored: Some(Box::new(move |dir| {
                !excluder.lock().expect("lock").folder(dir)
            })),
        }
    }
}

const TAG_PACKAGE: &str = "tags";

/// The identity policy of the pack.
struct LociModel {
    package: Package,
}

fn path_descriptors(path: &ProjectRelativePath) -> Vec<Descriptor> {
    path.as_str()
        .split('/')
        .map(|s| Descriptor::Namespace(s.to_string()))
        .collect()
}

impl ProjectModel for LociModel {
    fn package(&self) -> Option<Package> {
        Some(self.package.clone())
    }

    fn resolver_globals(&self, _path: &ProjectRelativePath) -> Vec<(String, String)> {
        Vec::new()
    }

    fn module_identity(&self, path: &ProjectRelativePath) -> Option<Identity> {
        let name = path.as_str().rsplit('/').next().unwrap_or(path.as_str());
        Some(Identity {
            reach: Reach::Global,
            descriptors: path_descriptors(path),
            kind: SymbolKind::File,
            display_name: name.to_string(),
        })
    }

    fn identity(
        &self,
        path: &ProjectRelativePath,
        _scopes: &[Scope],
        declaration: &Scope,
        _siblings: &[Scope],
    ) -> Identity {
        let mut descriptors = path_descriptors(path);
        descriptors.push(Descriptor::Term(declaration.name.clone()));
        let kind = match declaration.kind {
            DeclKind::Section => SymbolKind::Namespace,
            _ => SymbolKind::Key,
        };
        Identity {
            reach: Reach::Global,
            descriptors,
            kind,
            display_name: declaration.name.clone(),
        }
    }
}

impl NativeAnalysis for LociPack {
    fn default_package(&self) -> Package {
        Package {
            manager: "loci".to_string(),
            name: "vault".to_string(),
            version: String::new(),
        }
    }

    fn project_model(&self, root: &Path) -> Box<dyn ProjectModel> {
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("vault")
            .to_string();
        Box::new(LociModel {
            package: Package {
                manager: "loci".to_string(),
                name,
                version: String::new(),
            },
        })
    }

    fn analyze(&self, input: &NativeInput) -> NativeOutput {
        let mut output = NativeOutput::default();
        let warn = |code, message: String| Diagnostic {
            severity: Severity::Warning,
            stage: Stage::Discover,
            file: None,
            range: None,
            code,
            message,
        };
        let settings = match Settings::read(input.root) {
            Ok(Some(settings)) => settings,
            Ok(None) => return output,
            Err(message) => {
                output.diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    ..warn("vault-manifest-invalid", message)
                });
                return output;
            }
        };
        for what in &settings.unsupported {
            output.diagnostics.push(warn(
                "vault-setting-unsupported",
                format!("loci-core reads {what}. This pack does not, so its effect is missing"),
            ));
        }
        let files: Vec<(String, u64)> = input
            .files
            .iter()
            .map(|f: &ProjectFile| (f.path.as_str().to_string(), f.size))
            .collect();
        let classified = classify(&files, &settings);
        let wanted: HashSet<&str> = classified.sources.iter().map(String::as_str).collect();
        let mut notes: BTreeMap<String, Note> = BTreeMap::new();
        for source in input.sources {
            if wanted.contains(source.path.as_str()) {
                notes.insert(
                    source.path.as_str().to_string(),
                    analyze_note(source.text.as_bytes(), &settings),
                );
            }
        }
        // A note that the core could not read is still a note, with no facts.
        for path in &classified.sources {
            notes.entry(path.clone()).or_default();
        }
        let vault = Vault::new(&settings, &classified, &notes);
        build_output(input, &vault, &notes, &mut output);
        output
    }
}

fn span(range: (usize, usize)) -> (u32, u32) {
    (range.0 as u32, range.1 as u32)
}

/// A name that is unique in one note. The second use of a name gets `~2`, and so on.
fn unique(seen: &mut HashMap<String, u32>, name: &str) -> String {
    let count = seen.entry(name.to_string()).or_insert(0);
    *count += 1;
    if *count == 1 {
        name.to_string()
    } else {
        format!("{name}~{count}")
    }
}

fn build_output(
    input: &NativeInput,
    vault: &Vault,
    notes: &BTreeMap<String, Note>,
    output: &mut NativeOutput,
) {
    let readable: HashSet<&str> = input.sources.iter().map(|s| s.path.as_str()).collect();
    let mut tag_keys: HashSet<String> = HashSet::new();
    let mut stubs: BTreeMap<String, ProjectRelativePath> = BTreeMap::new();
    for (path, note) in notes {
        if !readable.contains(path.as_str()) {
            continue;
        }
        let Ok(rel) = ProjectRelativePath::new(path) else {
            continue;
        };
        let mut declarations = Vec::new();
        let mut headings = HashMap::new();
        let mut blocks = HashMap::new();
        let mut aliases = HashMap::new();
        let mut footnotes = HashMap::new();
        let mut elements = HashMap::new();
        let mut spans: HashSet<(usize, usize)> = HashSet::new();
        for anchor in &note.anchors {
            // One tag can give two ids. One span has one declaration.
            if !spans.insert(anchor.span) {
                continue;
            }
            let (kind, name) = match anchor.kind {
                AnchorKind::Heading => (DeclKind::Section, unique(&mut headings, &anchor.text)),
                AnchorKind::Block => (
                    DeclKind::Anchor,
                    unique(&mut blocks, &format!("^{}", anchor.text)),
                ),
                AnchorKind::Footnote => (
                    DeclKind::Anchor,
                    unique(&mut footnotes, &format!("fn:{}", anchor.text)),
                ),
                AnchorKind::Html => (
                    DeclKind::Anchor,
                    unique(&mut elements, &format!("id:{}", anchor.text)),
                ),
            };
            declarations.push(NativeDeclaration {
                span: span(anchor.span),
                name,
                kind,
                of_module: false,
            });
        }
        for (alias, start, end) in &note.aliases {
            if alias.is_empty() || start >= end {
                continue;
            }
            declarations.push(NativeDeclaration {
                span: span((*start, *end)),
                name: unique(&mut aliases, &format!("alias:{alias}")),
                kind: DeclKind::Alias,
                of_module: true,
            });
        }
        let mut references = Vec::new();
        for record in vault.records(path, note) {
            if record.kind == Kind::Tag {
                let key = casefold(&record.target);
                if key.is_empty() || key.split('/').any(str::is_empty) {
                    continue;
                }
                if tag_keys.insert(key.clone()) {
                    output.virtual_symbols.push(VirtualSymbol {
                        key: key.clone(),
                        identity: ExternalIdentity {
                            package: Package {
                                manager: "loci".to_string(),
                                name: TAG_PACKAGE.to_string(),
                                version: String::new(),
                            },
                            descriptors: key
                                .split('/')
                                .map(|s| Descriptor::Namespace(s.to_string()))
                                .collect(),
                            kind: SymbolKind::Key,
                            display_name: key.clone(),
                        },
                    });
                }
                references.push(NativeReference {
                    span: span(record.span),
                    role: ReferenceRole::Reference,
                    targets: vec![NativeTarget::Virtual(key)],
                });
                continue;
            }
            let module = |p: &str| {
                ProjectRelativePath::new(p)
                    .ok()
                    .map(|p| NativeTarget::Declaration(p, (0, 0)))
            };
            let targets: Vec<NativeTarget> = match (record.state, &record.target_path) {
                (State::Resolved, Some(target)) => {
                    let anchor = (record.anchor_state == "resolved")
                        .then_some(record.anchor_span)
                        .flatten();
                    if record.target_is_document == Some(false) {
                        if let Ok(p) = ProjectRelativePath::new(target) {
                            stubs.entry(target.clone()).or_insert(p);
                        }
                    }
                    match (anchor, ProjectRelativePath::new(target)) {
                        (Some(a), Ok(p)) => vec![NativeTarget::Declaration(p, span(a))],
                        _ => module(target).into_iter().collect(),
                    }
                }
                (State::Ambiguous, _) => {
                    // Every candidate needs a declaration, or a tie would look like one target.
                    // A candidate that is not a note is an attachment: it gets a stub.
                    for c in &record.candidates {
                        if !notes.contains_key(c) {
                            if let Ok(p) = ProjectRelativePath::new(c) {
                                stubs.entry(c.clone()).or_insert(p);
                            }
                        }
                    }
                    record.candidates.iter().filter_map(|c| module(c)).collect()
                }
                _ => Vec::new(),
            };
            references.push(NativeReference {
                span: span(record.span),
                role: if record.embed {
                    ReferenceRole::Read
                } else {
                    ReferenceRole::Reference
                },
                targets,
            });
        }
        output.documents.push(NativeDocument {
            path: rel,
            stub: false,
            module: true,
            declarations,
            references,
        });
    }
    for (_, path) in stubs {
        output.documents.push(NativeDocument {
            path,
            stub: true,
            module: true,
            declarations: Vec::new(),
            references: Vec::new(),
        });
    }
}

impl Record {
    /// The record in the shape of `graph/relations` of loci-core (USER.md section 28).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "source": self.source,
            "kind": self.kind.name(),
            "embed": self.embed,
            "span": [self.span.0, self.span.1],
            "target": self.target,
            "label": self.label,
            "heading": self.heading,
            "block": self.block,
            "property": self.property,
            "origin": self.origin,
            "state": self.state.name(),
            "target_path": self.target_path,
            "target_is_document": self.target_is_document,
            "candidates": self.candidates,
            "anchor_state": self.anchor_state,
            "anchor_kind": self.anchor_kind,
            "anchor_span": self.anchor_span.map(|s| [s.0, s.1]),
        })
    }
}

/// Every regular file below `root` with its size, sorted by path. It follows no link.
pub fn list_files(root: &Path) -> Vec<(String, u64)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                walk(root, &path, out);
            } else if kind.is_file() {
                if let (Ok(relative), Ok(meta)) = (path.strip_prefix(root), entry.metadata()) {
                    if let Some(rel) = relative.to_str() {
                        out.push((rel.replace('\\', "/"), meta.len()));
                    }
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, &mut files);
    files.sort();
    files
}

/// The relations of a vault, with the resolution of loci-core. It serves the oracle tests.
///
/// `files` lists every file of the vault with its size. `read` gives the bytes of a note.
pub fn export_relations(
    settings: &Settings,
    files: &[(String, u64)],
    mut read: impl FnMut(&str) -> Option<Vec<u8>>,
) -> Vec<Record> {
    let classified = classify(files, settings);
    let mut notes = BTreeMap::new();
    for path in &classified.sources {
        let note = read(path).map_or_else(Note::default, |raw| analyze_note(&raw, settings));
        notes.insert(path.clone(), note);
    }
    let vault = Vault::new(settings, &classified, &notes);
    notes
        .iter()
        .flat_map(|(path, note)| vault.records(path, note))
        .collect()
}
