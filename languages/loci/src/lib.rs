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
//! `[diagnostics] id_anchors`, `[diagnostics] clip_url_properties`, `[[analyzer]]`, `[[root]]`, `roots_file`, and `[policy]`:
//! `note_max_bytes`, `ignore_file`, and `id_template`. `discovery.link_targets` is read and
//! never applied, as in loci-core.
//!
//! The policy fails closed, as in loci-core. A bad `[policy]` value, a bad root, or an ignore
//! list or roots file that cannot be read gives the error `vault-manifest-invalid`, and the pack
//! indexes nothing. A path that the ignore list or `discovery.exclude` hides is hidden from every
//! pack, so a Python file in a hidden folder gets no index either.
//!
//! # Roots
//!
//! A root is a folder outside the vault, such as a code repository. Its files have the vault
//! path `<root name>/<path in the root>`. The pack lists them as loci-core does: `git ls-files`
//! for a `git` root, a walk for a `walk` root, then the filters of the root. It hands them to the
//! core, which routes each file to a pack by name. A Python file in a root goes to the Python
//! pack, and a `.md` file to this pack. A claimed file is a document with no structure. Any other
//! file is a link target. A link in a root resolves root-first. The Python and TypeScript packs
//! see the virtual paths, so they read no manifest of a root.
//!
//! A note that is not UTF-8 is skipped with the warning `invalid-utf8`. loci-core reads it
//! lossy.
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
//! A link with no anchor, or with an anchor that does not resolve, is one reference over the
//! whole link. It names the note. A link with a resolved anchor has two references: one over the
//! target part names the note, and one over the anchor part names the heading, the block, or the
//! id. A link to its own note has no target part, so it has the anchor reference only.
//! An embed has the read-access role. A link that is `missing` or `ambiguous` has no occurrence.
//! A tag is a reference to a symbol of the tag package.

mod config;
mod front;
mod note;
mod policy;
mod resolve;
mod roots;
mod scan;
mod text;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use scippy_core::{
    DeclKind, Descriptor, Diagnostic, Engine, ExternalIdentity, ExtraFile, ExtraFiles, Identity,
    InputLimits, LanguagePack, NativeAnalysis, NativeDeclaration, NativeDocument, NativeInput,
    NativeOutput, NativeReference, NativeTarget, Package, ProjectFile, ProjectModel,
    ProjectRelativePath, Reach, ReferenceRole, Scope, Severity, Stage, SymbolKind, VirtualSymbol,
    WalkRules,
};

pub use config::{GlobSet, Settings};
pub use note::{Note, analyze_note};
pub use policy::{IdTemplate, IgnoreList};
pub use resolve::{Classified, Record, Resolution, State, Vault, classify};
pub use roots::{Listing, RootSpec};
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
        let hide = Mutex::new(Excluder::new(&settings));
        let ignore = settings.ignore.clone();
        WalkRules {
            enter_ignored: Some(Box::new(move |dir| {
                !excluder.lock().expect("lock").folder(dir)
            })),
            // A path that the vault excludes or ignores is hidden from every pack.
            hidden: Some(Box::new(move |path| {
                hide.lock().expect("lock").file(path) || ignore.ignored(path)
            })),
        }
    }

    fn extra_files(&self, root: &Path) -> ExtraFiles {
        let Ok(Some(settings)) = Settings::read(root) else {
            return ExtraFiles::default();
        };
        if settings.roots.is_empty() {
            return ExtraFiles::default();
        }
        let listing = Listing::of(&settings);
        // Each root is a repository of its own. Only notes link across roots.
        let mut extra = ExtraFiles {
            shards: settings
                .roots
                .iter()
                .filter_map(|r| ProjectRelativePath::new(&r.name).ok())
                .collect(),
            ..ExtraFiles::default()
        };
        for (name, code) in &listing.diagnostics {
            extra.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                stage: Stage::Discover,
                file: None,
                range: None,
                code: if *code == "root_unavailable" {
                    "root-unavailable"
                } else {
                    "root-list-failed"
                },
                message: format!("the root {name} lists no files"),
            });
        }
        for (path, size) in listing.sources.iter().chain(&listing.link_targets) {
            let (Ok(rel), Some(real)) = (
                ProjectRelativePath::new(path),
                Listing::real_path(&settings, path),
            ) else {
                continue;
            };
            extra.files.push(ExtraFile {
                path: rel,
                real,
                size: *size,
            });
        }
        extra.state = Some(Box::new(listing));
        extra
    }

    fn input_limits(&self, root: &Path) -> Option<InputLimits> {
        let settings = Settings::read(root).ok().flatten()?;
        // A file of a vault is a note. The cap of the vault and of its roots is the limit of a file.
        let notes = settings
            .roots
            .iter()
            .map(|r| r.note_max_bytes)
            .chain([settings.note_max_bytes]);
        Some(InputLimits {
            max_file_bytes: notes.max().unwrap_or(settings.note_max_bytes),
            max_total_bytes: VAULT_MAX_TOTAL_BYTES,
        })
    }
}

/// The total input limit when a vault is active: 4 GiB. A vault has far more text than code.
const VAULT_MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;

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
        let in_root = |path: &str| {
            path.split_once('/')
                .is_some_and(|(head, _)| settings.roots.iter().any(|r| r.name == head))
        };
        let files: Vec<(String, u64)> = input
            .files
            .iter()
            .filter(|f| !in_root(f.path.as_str()))
            .map(|f: &ProjectFile| (f.path.as_str().to_string(), f.size))
            .collect();
        // The listing of the roots comes from `extra_files`. A caller that gave no state gets a new one.
        let fresh;
        let listing = match input.extra.and_then(|e| e.downcast_ref::<Listing>()) {
            Some(listing) => listing,
            None => {
                fresh = Listing::of(&settings);
                &fresh
            }
        };
        let classified = classify(&files, &settings, listing);
        let wanted: HashSet<&str> = classified
            .sources
            .iter()
            .map(String::as_str)
            .filter(|p| is_note(p))
            .collect();
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
        for path in classified.sources.iter().filter(|p| is_note(p)) {
            notes.entry(path.clone()).or_default();
        }
        let vault = Vault::new(&settings, &classified, &notes);
        build_output(input, &vault, &notes, &mut output);
        output
    }
}

/// A note is a Markdown file. Another source is a claimed file, a document with no structure.
fn is_note(path: &str) -> bool {
    path.to_lowercase().ends_with(".md")
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
                    // A target that is not a note is an attachment or a claimed file: it gets a stub.
                    if !notes.contains_key(target) {
                        if let Ok(p) = ProjectRelativePath::new(target) {
                            stubs.entry(target.clone()).or_insert(p);
                        }
                    }
                    let role = if record.embed {
                        ReferenceRole::Read
                    } else {
                        ReferenceRole::Reference
                    };
                    // A resolved anchor gives two references: the note and the anchor.
                    if let (Some(a), Some((target_part, anchor_part)), Ok(p)) =
                        (anchor, record.parts, ProjectRelativePath::new(target))
                    {
                        if target_part.0 < target_part.1 {
                            references.push(NativeReference {
                                span: span(target_part),
                                role,
                                targets: module(target).into_iter().collect(),
                            });
                        }
                        references.push(NativeReference {
                            span: span(anchor_part),
                            role,
                            targets: vec![NativeTarget::Declaration(p, span(a))],
                        });
                        continue;
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
/// `files` lists every file of the vault root with its size, and `extra` lists the files of the
/// extra roots. `read` gives the bytes of a note, by its vault path.
pub fn export_relations(
    settings: &Settings,
    files: &[(String, u64)],
    extra: &Listing,
    mut read: impl FnMut(&str) -> Option<Vec<u8>>,
) -> Vec<Record> {
    let classified = classify(files, settings, extra);
    let mut notes = BTreeMap::new();
    for path in classified.sources.iter().filter(|p| is_note(p)) {
        let note = read(path).map_or_else(Note::default, |raw| analyze_note(&raw, settings));
        notes.insert(path.clone(), note);
    }
    let vault = Vault::new(settings, &classified, &notes);
    notes
        .iter()
        .flat_map(|(path, note)| vault.records(path, note))
        .collect()
}

/// The relations of the vault at `root`, with its roots. It serves the oracle tests and the
/// example. It reads each note from its place on disk and writes nothing.
pub fn export_workspace(root: &Path) -> Result<Vec<Record>, String> {
    let settings = Settings::read(root)?.ok_or("the folder has no .loci/vault.toml")?;
    let files = list_files(root);
    let extra = Listing::of(&settings);
    Ok(export_relations(&settings, &files, &extra, |path| {
        let real = Listing::real_path(&settings, path).unwrap_or_else(|| root.join(path));
        std::fs::read(real).ok()
    }))
}
