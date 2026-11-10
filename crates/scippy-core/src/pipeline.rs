//! The indexing pipeline: discover, parse, extract, bind, reconcile, identify, emit.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::IndexRequest;
use crate::bindings::{
    BindingEngine, Bound, CompiledRules, Place, RawBinding, RawEnd, ResolveLimits,
};
use crate::emit::{self, Document, IndexData, Occurrence, Role, SymbolInfo};
use crate::error::ScippyError;
use crate::extract::{DeclarationFact, Extractor, Facts, SourceAnchor};
use crate::identity::{Descriptor, IdentityBuilder, SymbolId};
use crate::inherit::Inheritance;
use crate::language::{
    Analysis, DeclKind, Engine, ExternalAsset, ExtraFile, LanguagePack, Manifest, NativeAnalysis,
    NativeDocument, NativeInput, NativeSource, NativeTarget, ProjectFile, ProjectModel, Reach,
    ReferenceRole, Scope, SupportFile, WalkRules,
};
use crate::positions::LineIndex;
use crate::project::{self, FoundFile, ProjectRelativePath, Router};
use crate::reconcile::{
    BindingOutcome, DeclarationIndex, Reconciled, Targeting, UnresolvedReason, reconcile,
};
use crate::report::{Diagnostic, IndexReport, MultiTargetStats, Severity, Stage};

/// The SCIP scheme of every symbol that Scippy writes.
/// It is Scippy-owned. Scippy does not claim the identifiers of another indexer.
pub(crate) const SCHEME: &str = "scippy";

struct PackRoute<'a> {
    pack: &'a dyn LanguagePack,
    rules: WalkRules,
}

impl Router for PackRoute<'_> {
    fn id(&self) -> &'static str {
        self.pack.id()
    }
    fn accepts(&self, path: &Path) -> bool {
        self.pack.accepts(path)
    }
    fn enter_ignored(&self, relative_dir: &str) -> bool {
        self.rules
            .enter_ignored
            .as_ref()
            .is_some_and(|enter| enter(relative_dir))
    }
    fn hidden(&self, relative: &str) -> bool {
        self.rules
            .hidden
            .as_ref()
            .is_some_and(|hidden| hidden(relative))
    }
}

/// A document that parsed and built.
struct Doc {
    path: ProjectRelativePath,
    source: String,
    facts: Facts,
    /// The declarations with their symbols, in anchor order.
    declared: Vec<Declared>,
}

struct Declared {
    fact: DeclarationFact,
    symbol: SymbolId,
    kind: crate::emit::SymbolKind,
    display_name: String,
    /// The enclosing declarations and the declaration itself, for the target policy of the pack.
    scopes: Vec<Scope>,
    declaration: Scope,
}

pub(crate) fn run(
    request: &IndexRequest,
    languages: &[&dyn LanguagePack],
) -> Result<IndexReport, ScippyError> {
    // A pack that is not enabled for this project takes no part in the run.
    let languages: Vec<&dyn LanguagePack> = languages
        .iter()
        .copied()
        .filter(|p| p.enabled(&request.root))
        .collect();
    let languages = &languages[..];
    let routes: Vec<PackRoute> = languages
        .iter()
        .map(|p| PackRoute {
            pack: *p,
            rules: p.walk_rules(&request.root),
        })
        .collect();
    let routers: Vec<&dyn Router> = routes.iter().map(|r| r as &dyn Router).collect();
    let mut extra_files = Vec::new();
    let mut extra_diagnostics = Vec::new();
    let mut extra_states: Vec<Option<Box<dyn std::any::Any + Send + Sync>>> = Vec::new();
    let mut shards = Vec::new();
    for pack in languages {
        let extra = pack.extra_files(&request.root);
        extra_files.extend(extra.files);
        extra_diagnostics.extend(extra.diagnostics);
        shards.extend(extra.shards);
        extra_states.push(extra.state);
    }
    let found = project::discover(
        &request.root,
        &request.output,
        &routers,
        request.limits,
        &extra_files,
    )?;

    let mut report = IndexReport {
        files_discovered: found.files_discovered,
        diagnostics: found.diagnostics,
        ..Default::default()
    };
    report.diagnostics.extend(extra_diagnostics);
    let mut documents: Vec<Document> = Vec::new();
    let mut stubs: Vec<Document> = Vec::new();
    let mut external_symbols: BTreeMap<SymbolId, SymbolInfo> = BTreeMap::new();
    let mut trace = Vec::new();

    for (index, pack) in languages.iter().enumerate() {
        let files: Vec<&FoundFile> = found.files.iter().filter(|f| f.pack == index).collect();
        if files.is_empty() {
            continue;
        }
        let Some(engine) = pack.engine() else {
            report.files_skipped += files.len();
            report.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                stage: Stage::Discover,
                file: None,
                range: None,
                code: "unsupported-language",
                message: format!(
                    "the {} pack does not support indexing yet: skipped {} file(s)",
                    pack.id(),
                    files.len()
                ),
            });
            continue;
        };
        let indexed = match engine {
            Engine::StackGraphs(analysis) => index_pack(
                *pack,
                analysis,
                &found.root,
                &files,
                &shards,
                &extra_files,
                request,
                &mut report,
                &mut trace,
            )?,
            Engine::Native(native) => index_native_pack(
                *pack,
                native,
                &found.root,
                &files,
                &found.all_files,
                extra_states[index].as_deref(),
                request,
                &mut report,
                &mut trace,
            )?,
        };
        documents.extend(indexed.documents);
        stubs.extend(indexed.stubs);
        external_symbols.extend(indexed.external_symbols);
    }
    // A stub defines a symbol in a file that another pack may own. Merge it into that document.
    for stub in stubs {
        match documents
            .iter_mut()
            .find(|d: &&mut Document| d.path == stub.path)
        {
            Some(own) => {
                own.occurrences.extend(stub.occurrences);
                own.symbols.extend(stub.symbols);
            }
            None => documents.push(stub),
        }
    }

    report.files_indexed = documents.len();
    report.occurrences = documents.iter().map(|d| d.occurrences.len()).sum();

    if request.strict {
        let errors = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        if errors > 0 {
            return Err(ScippyError::Strict { errors });
        }
    }

    if let Some(path) = &request.trace {
        trace.sort();
        let io = |source| ScippyError::Io {
            path: path.clone(),
            source,
        };
        let mut file = File::create(path).map_err(io)?;
        for line in trace {
            writeln!(file, "{line}").map_err(io)?;
        }
    }

    emit::write_index(
        IndexData {
            project_root: found.root,
            documents,
            external_symbols: external_symbols.into_values().collect(),
        },
        &request.output,
    )?;
    Ok(report)
}

/// What one pack gives back: its documents and the external symbols that they reference.
struct PackIndex {
    documents: Vec<Document>,
    /// Documents of a native pack that only define symbols. See [`NativeDocument::stub`].
    stubs: Vec<Document>,
    external_symbols: BTreeMap<SymbolId, SymbolInfo>,
}

#[allow(clippy::too_many_arguments)]
fn index_pack(
    pack: &dyn LanguagePack,
    analysis: &dyn Analysis,
    root: &Path,
    files: &[&FoundFile],
    shards: &[ProjectRelativePath],
    extra_files: &[ExtraFile],
    request: &IndexRequest,
    report: &mut IndexReport,
    trace: &mut Vec<String>,
) -> Result<PackIndex, ScippyError> {
    let paths: Vec<ProjectRelativePath> = files.iter().map(|f| f.path.clone()).collect();
    let manifests = analysis
        .manifest_name()
        .map(|name| read_manifests(root, name, shards, extra_files))
        .unwrap_or_default();
    let model = analysis.project_model(root, &paths, &manifests);
    // One extractor for each dialect. The first one is the primary dialect.
    let mut extractors = vec![Extractor::new(pack.id(), analysis, analysis.grammar(), "")?];
    for dialect in analysis.extra_dialects() {
        extractors.push(Extractor::new(
            pack.id(),
            analysis,
            dialect.grammar,
            dialect.query,
        )?);
    }
    // Stages B to D, one shard at a time. Each shard gets a binding engine of its own.
    // The engines share one compiled copy of the rules.
    // The engine resolves the references of the shard. The pipeline frees it before the next
    // shard, so the peak memory follows the largest shard.
    let mut supports: HashMap<Option<&str>, Vec<SupportFile>> = HashMap::new();
    for support in analysis.support_files(root, &paths) {
        let shard = shard_of(&support.path, shards);
        supports.entry(shard).or_default().push(support);
    }
    let mut docs: Vec<(usize, Doc)> = Vec::new();
    let mut by_document: BTreeMap<ProjectRelativePath, Vec<RawBinding>> = BTreeMap::new();
    let groups = shard_groups(files, shards);
    let members: HashMap<&str, &[usize]> = groups
        .iter()
        .filter_map(|(shard, group)| shard.map(|s| (s, &group[..])))
        .collect();
    let compiled = CompiledRules::new(pack.id(), analysis)?;
    for (shard, group) in &groups {
        let mut engine = BindingEngine::new(pack.id(), analysis, &compiled)?;
        let first = docs.len();
        for &at in group {
            let file = files[at];
            if let Some(doc) = load_file(
                analysis,
                &extractors,
                &*model,
                &mut engine,
                root,
                file,
                request,
                report,
                true,
            ) {
                docs.push((at, doc));
            }
        }
        // The code that the shard imports from other shards. The engine reads it to resolve
        // the references of the shard. The shard that owns the code indexes it.
        for dependency in dependency_closure(&*model, shard.unwrap_or_default(), &members) {
            for &at in members[dependency.as_str()] {
                let file = files[at];
                if model.provides(&file.path) {
                    let mut scratch = IndexReport::default();
                    load_file(
                        analysis,
                        &extractors,
                        &*model,
                        &mut engine,
                        root,
                        file,
                        request,
                        &mut scratch,
                        false,
                    );
                }
            }
        }
        // The support files of the shard, such as `tsconfig.json`.
        let sources: Vec<ProjectRelativePath> =
            docs[first..].iter().map(|(_, d)| d.path.clone()).collect();
        for support in supports.remove(shard).unwrap_or_default() {
            if let Err(failure) = engine.add_support_file(&support, &sources) {
                report.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    stage: Stage::Bind,
                    file: Some(support.path.as_str().to_string()),
                    range: None,
                    code: "support-file-error",
                    message: format!("ignored: {}", failure.message),
                });
            }
        }
        for binding in engine.resolve(ResolveLimits::default()) {
            by_document
                .entry(binding.reference.path.clone())
                .or_default()
                .push(binding);
        }
    }
    // The later stages see the documents in the order of the walk.
    docs.sort_by_key(|(at, _)| *at);
    let mut docs: Vec<Doc> = docs.into_iter().map(|(_, doc)| doc).collect();

    // Stage F: assign identities.
    let mut package = model
        .package()
        .unwrap_or_else(|| analysis.default_package());
    if let Some(name) = &request.package_name {
        package.name = name.clone();
    }
    if let Some(version) = &request.package_version {
        package.version = version.clone();
    }
    let mut builder = IdentityBuilder::new(SCHEME, package);
    let mut index = DeclarationIndex::default();
    for doc in &mut docs {
        doc.declared = assign_identities(&*model, &mut builder, doc)?;
        report.declarations += doc.declared.len();
        for d in &doc.declared {
            let extra = d
                .fact
                .extra_anchors
                .iter()
                .map(|&(start, end)| SourceAnchor {
                    path: d.fact.anchor.path.clone(),
                    start_byte: start,
                    end_byte: end,
                });
            for anchor in std::iter::once(d.fact.anchor.clone()).chain(extra) {
                index.insert_scoped(
                    anchor,
                    d.symbol.clone(),
                    d.scopes.clone(),
                    d.declaration.clone(),
                );
            }
        }
    }

    // The declarations of external assets, such as the builtins.
    let externals = declare_external(&extractors[0], analysis, &*model, report, &mut index);

    // A binding name takes the target of its reference. It joins as a module-like binding.
    for doc in &docs {
        let Some(bindings) = by_document.get(&doc.path) else {
            continue;
        };
        for &(name, target) in &doc.facts.aliases {
            let symbols: BTreeSet<&SymbolId> = bindings
                .iter()
                .filter(|b| {
                    b.bound.is_none() && (b.reference.start_byte, b.reference.end_byte) == target
                })
                .flat_map(|b| b.ends.iter())
                .filter_map(|end| end.place.as_ref())
                .filter_map(|place| index.get(place))
                .filter(|symbol| !symbol.is_local())
                .collect();
            if let [symbol] = symbols.into_iter().collect::<Vec<_>>()[..] {
                let anchor = SourceAnchor {
                    path: doc.path.clone(),
                    start_byte: name.0,
                    end_byte: name.1,
                };
                let text = doc.source.get(name.0 as usize..name.1 as usize);
                let binding = Scope {
                    kind: DeclKind::Module,
                    name: text.unwrap_or_default().to_string(),
                };
                let symbol = symbol.clone();
                index.insert_scoped(anchor, symbol, Vec::new(), binding);
            }
        }
    }

    // Stage E: reconcile.
    let language = pack.scip_language().to_string();
    let mut out = Vec::with_capacity(docs.len());
    let mut inheritance = Inheritance::default();
    // The document that defines each method that can override another.
    let mut method_documents: BTreeMap<SymbolId, usize> = BTreeMap::new();
    let mut referenced: BTreeMap<SymbolId, SymbolInfo> = BTreeMap::new();
    for doc in docs {
        let bindings = by_document.remove(&doc.path).unwrap_or_default();
        let members: HashSet<(u32, u32)> = doc.facts.members.iter().copied().collect();
        let keywords: HashSet<(u32, u32)> = doc.facts.keywords.iter().copied().collect();
        let containers = doc
            .facts
            .declarations
            .iter()
            .filter(|f| f.kind.is_container())
            .map(|f| {
                let scope = Scope {
                    kind: f.kind,
                    name: f.name.clone(),
                };
                (f.extent, scope)
            })
            .chain(
                doc.facts
                    .scopes
                    .iter()
                    .map(|&extent| (extent, anonymous_scope(extent))),
            )
            .collect();
        let targeting = Targeting::new(&*model, containers);
        let reconciled = reconcile(&index, bindings, &members, &keywords, &targeting);
        for r in &reconciled {
            count(report, r);
            if let BindingOutcome::Resolved(symbol) = &r.outcome {
                if let Some(info) = externals.get(symbol) {
                    report.references_external += 1;
                    referenced
                        .entry(symbol.clone())
                        .or_insert_with(|| info.clone());
                }
            }
        }
        if request.trace.is_some() {
            let lines = LineIndex::new(&doc.source);
            trace.extend(reconciled.iter().map(|r| trace_line(&lines, r)));
        }
        record_inheritance(&doc, &reconciled, &mut inheritance);
        for d in doc.declared.iter().filter(|d| is_method(d)) {
            method_documents.insert(d.symbol.clone(), out.len());
        }
        out.push(build_document(&language, doc, &reconciled));
    }
    // Stage G: an override method implements the method that it shadows.
    for (method, bases) in inheritance.overrides() {
        let Some(&at) = method_documents.get(&method) else {
            continue;
        };
        let symbols = &mut out[at].symbols;
        if let Ok(i) = symbols.binary_search_by(|s| s.symbol.cmp(&method)) {
            symbols[i].implements = bases.into_iter().collect();
        }
    }
    Ok(PackIndex {
        documents: out,
        stubs: Vec::new(),
        external_symbols: referenced,
    })
}

/// Read the manifest `name` in the project root and in each shard.
/// A shard is a virtual folder, so its manifest is an extra file.
fn read_manifests(
    root: &Path,
    name: &str,
    shards: &[ProjectRelativePath],
    extra_files: &[ExtraFile],
) -> Vec<Manifest> {
    let real: HashMap<&str, &Path> = extra_files
        .iter()
        .map(|f| (f.path.as_str(), f.real.as_path()))
        .collect();
    let folders = std::iter::once("").chain(shards.iter().map(ProjectRelativePath::as_str));
    folders
        .filter_map(|folder| {
            let virtual_path = if folder.is_empty() {
                name.to_string()
            } else {
                format!("{folder}/{name}")
            };
            let disk = real
                .get(virtual_path.as_str())
                .map_or_else(|| root.join(&virtual_path), |p| p.to_path_buf());
            let text = std::fs::read_to_string(disk).ok()?;
            Some(Manifest {
                folder: folder.to_string(),
                text,
            })
        })
        .collect()
}

/// The shards that `shard` depends on, directly or through another dependency, in the order
/// that a breadth-first walk finds them. Only shards with files count.
fn dependency_closure(
    model: &dyn ProjectModel,
    shard: &str,
    members: &HashMap<&str, &[usize]>,
) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::from([shard.to_string()]);
    let mut queue = std::collections::VecDeque::from([shard.to_string()]);
    while let Some(next) = queue.pop_front() {
        for dependency in model.dependencies(&next) {
            if members.contains_key(dependency.as_str()) && seen.insert(dependency.clone()) {
                found.push(dependency.clone());
                queue.push_back(dependency);
            }
        }
    }
    found
}

/// The longest shard that holds the path, or `None` for a path outside every shard.
fn shard_of<'a>(path: &ProjectRelativePath, shards: &'a [ProjectRelativePath]) -> Option<&'a str> {
    let path = path.as_str();
    shards
        .iter()
        .map(ProjectRelativePath::as_str)
        .filter(|shard| {
            path.strip_prefix(shard)
                .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|shard| shard.len())
}

/// The files of each shard, as indexes into `files`. The shards come in the order of their
/// first file, and the files of a shard keep their order.
fn shard_groups<'a>(
    files: &[&FoundFile],
    shards: &'a [ProjectRelativePath],
) -> Vec<(Option<&'a str>, Vec<usize>)> {
    let mut groups: Vec<(Option<&str>, Vec<usize>)> = Vec::new();
    let mut slot: HashMap<Option<&str>, usize> = HashMap::new();
    for (at, file) in files.iter().enumerate() {
        let shard = shard_of(&file.path, shards);
        let n = *slot.entry(shard).or_insert_with(|| {
            groups.push((shard, Vec::new()));
            groups.len() - 1
        });
        groups[n].1.push(at);
    }
    groups
}

/// Read, parse, and extract one file, and add its graph to the engine.
/// Return `None` when the pipeline skips the file. A diagnostic then says why.
#[allow(clippy::too_many_arguments)]
fn load_file(
    analysis: &dyn Analysis,
    extractors: &[Extractor],
    model: &dyn ProjectModel,
    engine: &mut BindingEngine,
    root: &Path,
    file: &FoundFile,
    request: &IndexRequest,
    report: &mut IndexReport,
    resolve: bool,
) -> Option<Doc> {
    let diagnostic = |stage, severity, code, range, message: String| Diagnostic {
        severity,
        stage,
        file: Some(file.path.as_str().to_string()),
        range,
        code,
        message,
    };
    if let Some(winner) = model.shadowed_by(&file.path) {
        report.files_skipped += 1;
        report.diagnostics.push(diagnostic(
            Stage::Identity,
            Severity::Warning,
            "shadowed-file",
            None,
            format!(
                "skipped: {} shadows this file, so no import reaches it",
                winner.as_str()
            ),
        ));
        return None;
    }
    let source = match read_source(&file.disk_path(root), request.limits.max_file_bytes) {
        Ok(source) => source,
        Err((code, message)) => {
            report.files_skipped += 1;
            report.diagnostics.push(diagnostic(
                Stage::Discover,
                Severity::Warning,
                code,
                None,
                message,
            ));
            return None;
        }
    };
    let dialect = analysis.dialect_of(&file.path).min(extractors.len() - 1);
    let extractor = &extractors[dialect];
    // The text that the parser and the resolver read. It differs from `source` only when
    // the pack repaired a construct that its grammar rejects.
    let mut parsed_text: Option<String> = None;
    let first = extractor.parse(&source);
    let tree = match first {
        Ok(tree) => tree,
        Err(error) => {
            let repaired = analysis.repair_syntax(&source).filter(|fixed| {
                fixed.len() == source.len()
                    && fixed.bytes().filter(|&b| b == b'\n').count()
                        == source.bytes().filter(|&b| b == b'\n').count()
            });
            let tree = repaired.and_then(|fixed| {
                let tree = extractor.parse(&fixed).ok()?;
                parsed_text = Some(fixed);
                Some(tree)
            });
            if let Some(tree) = tree {
                report.diagnostics.push(diagnostic(
                    Stage::Parse,
                    Severity::Warning,
                    "repaired-syntax",
                    Some((error.start_byte, error.end_byte)),
                    "the grammar rejects this file. The pack blanked out the syntax that it does not know"
                        .to_string(),
                ));
                tree
            } else {
                report.files_skipped += 1;
                report.diagnostics.push(diagnostic(
                    Stage::Parse,
                    Severity::Warning,
                    "syntax-error",
                    Some((error.start_byte, error.end_byte)),
                    "skipped: the file has a syntax error".to_string(),
                ));
                return None;
            }
        }
    };
    let text = parsed_text.as_deref().unwrap_or(&source);
    let facts = extractor.facts(&file.path, text, &tree);
    drop(tree);
    if let Err(failure) = engine.add_file(
        dialect,
        &file.path,
        text,
        &model.resolver_globals(&file.path),
        resolve,
    ) {
        report.files_skipped += 1;
        report.diagnostics.push(diagnostic(
            Stage::Bind,
            Severity::Warning,
            "bind-error",
            None,
            format!("skipped: {}", failure.message),
        ));
        return None;
    }
    for &(start, end) in &facts.conflicts {
        report.diagnostics.push(diagnostic(
            Stage::Extract,
            Severity::Warning,
            "declaration-conflict",
            Some((start, end)),
            "two captures give this identifier different kinds. The extractor dropped it"
                .to_string(),
        ));
    }
    Some(Doc {
        path: file.path.clone(),
        source,
        facts,
        declared: Vec::new(),
    })
}

/// Index the files of a native pack: the pack declares and resolves, the core finishes.
///
/// The pack gives declarations and the targets of each reference. The core joins each target to a
/// declaration by document and span, applies the policy of the project model, assigns the symbol
/// identities, and builds the documents. The shadow, bounded, and nearest-binding rules of the
/// reconciler have nothing to act on, because a native reference has no paths.
#[allow(clippy::too_many_arguments)]
fn index_native_pack(
    pack: &dyn LanguagePack,
    native: &dyn NativeAnalysis,
    root: &Path,
    files: &[&FoundFile],
    all_files: &[ProjectFile],
    extra: Option<&(dyn std::any::Any + Send + Sync)>,
    request: &IndexRequest,
    report: &mut IndexReport,
    trace: &mut Vec<String>,
) -> Result<PackIndex, ScippyError> {
    // Read the files that the pack accepts.
    let mut sources: Vec<NativeSource> = Vec::with_capacity(files.len());
    for file in files {
        match read_source(&file.disk_path(root), request.limits.max_file_bytes) {
            Ok(text) => sources.push(NativeSource {
                path: file.path.clone(),
                text,
            }),
            Err((code, message)) => {
                report.files_skipped += 1;
                report.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    stage: Stage::Discover,
                    file: Some(file.path.as_str().to_string()),
                    range: None,
                    code,
                    message,
                });
            }
        }
    }
    let output = native.analyze(&NativeInput {
        root,
        files: all_files,
        sources: &sources,
        extra,
    });
    report.diagnostics.extend(output.diagnostics);
    let model = native.project_model(root);

    let mut package = model.package().unwrap_or_else(|| native.default_package());
    if let Some(name) = &request.package_name {
        package.name = name.clone();
    }
    if let Some(version) = &request.package_version {
        package.version = version.clone();
    }
    let mut builder = IdentityBuilder::new(SCHEME, package);
    let mut texts: HashMap<ProjectRelativePath, String> =
        sources.into_iter().map(|s| (s.path, s.text)).collect();

    // Stage F: assign identities. Each document gets its facts from the declarations of the pack.
    let mut index = DeclarationIndex::default();
    let mut docs: Vec<(Doc, &NativeDocument)> = Vec::with_capacity(output.documents.len());
    for native_doc in &output.documents {
        let source = if native_doc.stub {
            String::new()
        } else {
            texts.remove(&native_doc.path).unwrap_or_default()
        };
        let anchor = |span: (u32, u32)| SourceAnchor {
            path: native_doc.path.clone(),
            start_byte: span.0,
            end_byte: span.1,
        };
        let mut facts = Facts::default();
        if native_doc.module {
            facts.module = Some(DeclarationFact {
                anchor: anchor((0, 0)),
                name: String::new(),
                kind: DeclKind::Module,
                extent: (0, source.len() as u32),
                extra_anchors: Vec::new(),
            });
        }
        for d in &native_doc.declarations {
            facts.declarations.push(DeclarationFact {
                anchor: anchor(d.span),
                name: d.name.clone(),
                kind: d.kind,
                extent: d.span,
                extra_anchors: Vec::new(),
            });
            if d.of_module {
                facts.module_aliases.push(d.span);
            }
        }
        facts.reads = native_doc
            .references
            .iter()
            .filter(|r| r.role == ReferenceRole::Read)
            .map(|r| r.span)
            .collect();
        facts.reads.sort_unstable();
        let mut doc = Doc {
            path: native_doc.path.clone(),
            source,
            facts,
            declared: Vec::new(),
        };
        doc.declared = assign_identities(&*model, &mut builder, &doc)?;
        report.declarations += doc.declared.len();
        for d in &doc.declared {
            index.insert_scoped(
                d.fact.anchor.clone(),
                d.symbol.clone(),
                d.scopes.clone(),
                d.declaration.clone(),
            );
        }
        docs.push((doc, native_doc));
    }

    // The symbols that no document declares. A reference to one of them is external.
    let mut externals: BTreeMap<SymbolId, SymbolInfo> = BTreeMap::new();
    let mut virtual_anchors: HashMap<&str, SourceAnchor> = HashMap::new();
    for (n, virtual_symbol) in output.virtual_symbols.iter().enumerate() {
        let identity = &virtual_symbol.identity;
        match IdentityBuilder::new(SCHEME, identity.package.clone()).symbol(&identity.descriptors) {
            Ok(symbol) => {
                let anchor = SourceAnchor {
                    path: ProjectRelativePath::new("<virtual>")?,
                    start_byte: n as u32,
                    end_byte: n as u32 + 1,
                };
                index.insert_external_scoped(
                    anchor.clone(),
                    symbol.clone(),
                    Vec::new(),
                    Scope {
                        kind: DeclKind::Variable,
                        name: identity.display_name.clone(),
                    },
                );
                virtual_anchors.insert(&virtual_symbol.key, anchor);
                externals.entry(symbol.clone()).or_insert(SymbolInfo {
                    symbol,
                    kind: identity.kind,
                    display_name: identity.display_name.clone(),
                    implements: Vec::new(),
                    references: Vec::new(),
                });
            }
            Err(error) => report.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                stage: Stage::Identity,
                file: None,
                range: None,
                code: "external-symbol-invalid",
                message: error.to_string(),
            }),
        }
    }

    // Stage D replaced: the targets of the pack are the ends of the bindings.
    let place = |anchor: SourceAnchor, external: bool| Place { anchor, external };
    let mut by_document: BTreeMap<ProjectRelativePath, Vec<RawBinding>> = BTreeMap::new();
    for (_, native_doc) in &docs {
        let bindings = native_doc.references.iter().map(|r| {
            let ends = r
                .targets
                .iter()
                .filter_map(|target| match target {
                    NativeTarget::Declaration(path, span) => Some(place(
                        SourceAnchor {
                            path: path.clone(),
                            start_byte: span.0,
                            end_byte: span.1,
                        },
                        false,
                    )),
                    NativeTarget::Virtual(key) => virtual_anchors
                        .get(key.as_str())
                        .map(|anchor| place(anchor.clone(), true)),
                })
                .map(|place| RawEnd {
                    place: Some(place),
                    syntax_type: None,
                    through: Vec::new(),
                })
                .collect();
            RawBinding {
                reference: SourceAnchor {
                    path: native_doc.path.clone(),
                    start_byte: r.span.0,
                    end_byte: r.span.1,
                },
                ends,
                shadowed_ends: Vec::new(),
                bound: None,
            }
        });
        by_document.insert(native_doc.path.clone(), bindings.collect());
    }

    // Stage E: reconcile. Stage G: build the documents.
    let language = pack.scip_language().to_string();
    let mut out = Vec::new();
    let mut stubs = Vec::new();
    let mut referenced: BTreeMap<SymbolId, SymbolInfo> = BTreeMap::new();
    for (doc, native_doc) in docs {
        let bindings = by_document.remove(&doc.path).unwrap_or_default();
        let targeting = Targeting::new(&*model, Vec::new());
        let reconciled = reconcile(
            &index,
            bindings,
            &HashSet::new(),
            &HashSet::new(),
            &targeting,
        );
        for r in &reconciled {
            count(report, r);
            if let BindingOutcome::Resolved(symbol) = &r.outcome {
                if let Some(info) = externals.get(symbol) {
                    report.references_external += 1;
                    referenced
                        .entry(symbol.clone())
                        .or_insert_with(|| info.clone());
                }
            }
        }
        if request.trace.is_some() {
            let lines = LineIndex::new(&doc.source);
            trace.extend(reconciled.iter().map(|r| trace_line(&lines, r)));
        }
        let document_language = if native_doc.stub { "" } else { &language };
        let built = build_document(document_language, doc, &reconciled);
        if native_doc.stub {
            stubs.push(built);
        } else {
            out.push(built);
        }
    }
    Ok(PackIndex {
        documents: out,
        stubs,
        external_symbols: referenced,
    })
}

/// The scope that a node without a name gives to the declarations inside it.
/// The name is `@` and the byte offset of the start of the node. No identifier can have that
/// name, and two nodes never share it.
fn anonymous_scope(extent: (u32, u32)) -> Scope {
    Scope {
        kind: DeclKind::Function,
        name: format!("@{}", extent.0),
    }
}

/// The facts of the nodes that hold declarations and have no name. They are containers only.
fn anonymous_facts(doc: &Doc) -> Vec<DeclarationFact> {
    doc.facts
        .scopes
        .iter()
        .map(|&extent| DeclarationFact {
            anchor: SourceAnchor {
                path: doc.path.clone(),
                start_byte: extent.0,
                end_byte: extent.0,
            },
            name: anonymous_scope(extent).name,
            kind: DeclKind::Function,
            extent,
            extra_anchors: Vec::new(),
        })
        .collect()
}

/// A method that can override or be overridden: a function that a class directly holds,
/// with a symbol that other documents can address.
fn is_method(d: &Declared) -> bool {
    d.fact.kind == DeclKind::Function
        && !d.symbol.is_local()
        && d.scopes.last().is_some_and(|s| s.kind.is_class_like())
}

/// The innermost class declaration whose extent holds the span.
fn enclosing_class(declared: &[Declared], start: u32, end: u32) -> Option<&Declared> {
    declared
        .iter()
        .filter(|d| d.fact.kind.is_class_like())
        .filter(|d| d.fact.extent.0 <= start && end <= d.fact.extent.1)
        .min_by_key(|d| d.fact.extent.1 - d.fact.extent.0)
}

/// Add the supertypes and the methods of the classes of one document.
fn record_inheritance(doc: &Doc, reconciled: &[Reconciled], inheritance: &mut Inheritance) {
    for method in doc.declared.iter().filter(|d| is_method(d)) {
        let (start, end) = method.fact.extent;
        if let Some(class) = enclosing_class(&doc.declared, start, end) {
            if !class.symbol.is_local() {
                inheritance.add_method(&class.symbol, &method.display_name, method.symbol.clone());
            }
        }
    }
    let resolved: HashMap<(u32, u32), &SymbolId> = reconciled
        .iter()
        .filter_map(|r| match &r.outcome {
            BindingOutcome::Resolved(symbol) => {
                Some(((r.reference.start_byte, r.reference.end_byte), symbol))
            }
            _ => None,
        })
        .collect();
    for &(start, end) in &doc.facts.supertypes {
        let (Some(class), Some(supertype)) = (
            enclosing_class(&doc.declared, start, end),
            resolved.get(&(start, end)),
        ) else {
            continue;
        };
        if !class.symbol.is_local() {
            inheritance.add_supertype(&class.symbol, (*supertype).clone());
        }
    }
}

/// Parse the external assets of the pack and give their declarations identities.
///
/// The assets are the builtins and the other assets that the pack lists, such as stub modules.
/// The declarations join to references by anchor, like project declarations.
/// The result maps each external symbol to its metadata.
fn declare_external(
    extractor: &Extractor,
    analysis: &dyn Analysis,
    model: &dyn ProjectModel,
    report: &mut IndexReport,
    index: &mut DeclarationIndex,
) -> BTreeMap<SymbolId, SymbolInfo> {
    let mut symbols = BTreeMap::new();
    for asset in analysis.binding_rules().external_assets() {
        declare_asset(extractor, &asset, model, report, index, &mut symbols);
    }
    symbols
}

/// Give the declarations of one external asset their identities.
fn declare_asset(
    extractor: &Extractor,
    asset: &ExternalAsset,
    model: &dyn ProjectModel,
    report: &mut IndexReport,
    index: &mut DeclarationIndex,
    symbols: &mut BTreeMap<SymbolId, SymbolInfo>,
) {
    if asset.source.trim().is_empty() {
        return;
    }
    let warn = |code, message: String| Diagnostic {
        severity: Severity::Warning,
        stage: Stage::Identity,
        file: None,
        range: None,
        code,
        message,
    };
    let Ok(path) = ProjectRelativePath::new(&asset.name) else {
        return;
    };
    let tree = match extractor.parse(asset.source) {
        Ok(tree) => tree,
        Err(_) => {
            report.diagnostics.push(warn(
                "external-asset-syntax-error",
                format!(
                    "the asset {} has a syntax error. Its names stay unresolved",
                    asset.path
                ),
            ));
            return;
        }
    };
    let facts = extractor.facts(&path, asset.source, &tree);
    let declarations = &facts.declarations;
    let containers = container_indexes(declarations);
    // The module of the asset is a declaration too. Its name is the stem of the file name.
    let stem = asset.name.rsplit('/').next().unwrap_or(&asset.name);
    let module = facts.module.iter().map(|fact| {
        let name = stem.rsplit_once('.').map_or(stem, |(stem, _)| stem);
        let declaration = Scope {
            kind: DeclKind::Module,
            name: name.to_string(),
        };
        (fact, Vec::new(), declaration)
    });
    let members = declarations.iter().enumerate().map(|(i, fact)| {
        let declaration = Scope {
            kind: fact.kind,
            name: fact.name.clone(),
        };
        (fact, scope_chain(declarations, &containers, i), declaration)
    });
    for (fact, scopes, declaration) in module.chain(members) {
        let Some(identity) = model.external_identity(&asset.name, &scopes, &declaration) else {
            continue;
        };
        match IdentityBuilder::new(SCHEME, identity.package).symbol(&identity.descriptors) {
            Ok(symbol) => {
                index.insert_external_scoped(
                    fact.anchor.clone(),
                    symbol.clone(),
                    scopes.clone(),
                    declaration.clone(),
                );
                symbols.entry(symbol.clone()).or_insert(SymbolInfo {
                    symbol,
                    kind: identity.kind,
                    display_name: identity.display_name,
                    implements: Vec::new(),
                    references: Vec::new(),
                });
            }
            Err(error) => report
                .diagnostics
                .push(warn("external-symbol-invalid", error.to_string())),
        }
    }
}

/// Read a file. Return a diagnostic code and message on failure.
fn read_source(full: &Path, max: u64) -> Result<String, (&'static str, String)> {
    let mut bytes = Vec::new();
    File::open(full)
        .and_then(|f| f.take(max + 1).read_to_end(&mut bytes))
        .map_err(|e| ("unreadable-file", format!("skipped: {e}")))?;
    if bytes.len() as u64 > max {
        // The file grew after the walker measured it.
        return Err((
            "file-too-large",
            format!("skipped: the file exceeds the limit of {max} bytes"),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| ("invalid-utf8", "skipped: the file is not UTF-8".to_string()))
}

/// The indexes of the declarations that can hold other declarations.
fn container_indexes(facts: &[DeclarationFact]) -> Vec<usize> {
    (0..facts.len())
        .filter(|&i| facts[i].kind.is_container())
        .collect()
}

/// The innermost container that holds declaration `i`, when there is one.
fn innermost_container(facts: &[DeclarationFact], containers: &[usize], i: usize) -> Option<usize> {
    let fact = &facts[i];
    containers
        .iter()
        .copied()
        .filter(|&j| {
            j != i && facts[j].extent.0 <= fact.extent.0 && fact.extent.1 <= facts[j].extent.1
        })
        .min_by_key(|&j| facts[j].extent.1 - facts[j].extent.0)
}

/// The enclosing declarations of declaration `i`, from the outermost to the innermost.
fn scope_chain(facts: &[DeclarationFact], containers: &[usize], i: usize) -> Vec<Scope> {
    let fact = &facts[i];
    let mut ancestors: Vec<&DeclarationFact> = containers
        .iter()
        .filter(|&&j| {
            j != i && facts[j].extent.0 <= fact.extent.0 && fact.extent.1 <= facts[j].extent.1
        })
        .map(|&j| &facts[j])
        .collect();
    ancestors.sort_by_key(|a| (a.extent.0, std::cmp::Reverse(a.extent.1)));
    ancestors
        .iter()
        .map(|a| Scope {
            kind: a.kind,
            name: a.name.clone(),
        })
        .collect()
}

/// Give each declaration of a document its symbol.
fn assign_identities(
    model: &dyn ProjectModel,
    builder: &mut IdentityBuilder,
    doc: &Doc,
) -> Result<Vec<Declared>, ScippyError> {
    // The declarations come first. The nodes without a name follow. They are containers only.
    let declared_count = doc.facts.declarations.len();
    let mut facts = doc.facts.declarations.clone();
    facts.extend(anonymous_facts(doc));
    let facts = &facts;
    let containers = container_indexes(facts);
    let mut locals: BTreeMap<Vec<Descriptor>, u32> = BTreeMap::new();
    let mut out = Vec::with_capacity(declared_count + 1);
    // The module of the document. The pack decides whether it has a symbol.
    if let (Some(fact), Some(identity)) = (&doc.facts.module, model.module_identity(&doc.path)) {
        if identity.reach == Reach::Global {
            let symbol = builder.define(&identity.descriptors, fact.anchor.clone())?;
            out.push(Declared {
                fact: fact.clone(),
                symbol,
                kind: identity.kind,
                scopes: Vec::new(),
                declaration: Scope {
                    kind: DeclKind::Module,
                    name: identity.display_name.clone(),
                },
                display_name: identity.display_name,
            });
        }
    }
    // The declarations that each container holds directly, keyed by the extent of the container.
    let holders: Vec<Option<(u32, u32)>> = (0..facts.len())
        .map(|i| innermost_container(facts, &containers, i).map(|j| facts[j].extent))
        .collect();
    let mut held: HashMap<Option<(u32, u32)>, Vec<Scope>> = HashMap::new();
    for (fact, holder) in facts.iter().zip(&holders).take(declared_count) {
        held.entry(*holder).or_default().push(Scope {
            kind: fact.kind,
            name: fact.name.clone(),
        });
    }
    for (i, fact) in facts.iter().enumerate().take(declared_count) {
        let scopes = scope_chain(facts, &containers, i);
        let declaration = Scope {
            kind: fact.kind,
            name: fact.name.clone(),
        };
        let siblings: Vec<Scope> = held[&holders[i]]
            .iter()
            .filter(|s| **s != declaration)
            .cloned()
            .collect();
        let identity = model.identity(&doc.path, &scopes, &declaration, &siblings);
        let symbol = match identity.reach {
            Reach::Global => builder.define(&identity.descriptors, fact.anchor.clone())?,
            Reach::Document => {
                let next = locals.len() as u32;
                SymbolId::local(*locals.entry(identity.descriptors).or_insert(next))
            }
        };
        out.push(Declared {
            fact: fact.clone(),
            symbol,
            kind: identity.kind,
            display_name: identity.display_name,
            scopes,
            declaration,
        });
    }
    Ok(out)
}

fn count(report: &mut IndexReport, r: &Reconciled) {
    for kind in &r.unjoined {
        *report.unjoined_end_kinds.entry(kind.clone()).or_default() += 1;
    }
    let multi = r.ends + r.shadowed >= 2;
    let stats: &mut MultiTargetStats = &mut report.multi_target;
    if multi {
        stats.raw += 1;
    }
    match &r.outcome {
        BindingOutcome::Resolved(_) => {
            report.references_resolved += 1;
            if multi {
                if r.shadowed > 0 {
                    stats.shadow_filtered += 1;
                } else if r.refused > 0 {
                    stats.pack_refused += 1;
                } else if r.value_flow_dropped > 0 {
                    stats.nearest_binding += 1;
                } else if r.joined == 1 && r.outside > 0 {
                    stats.builtin_shadow += 1;
                } else if r.joined == 1 {
                    stats.alias_collapsed += 1;
                } else {
                    stats.same_symbol += 1;
                }
            }
        }
        BindingOutcome::Ambiguous(_) => {
            report.references_ambiguous += 1;
            if multi {
                stats.ambiguous += 1;
            }
        }
        BindingOutcome::Unresolved(reason) => {
            report.references_unresolved += 1;
            *report
                .unresolved_reasons
                .entry(reason_code(*reason))
                .or_default() += 1;
            if multi {
                stats.unresolved += 1;
            }
        }
        BindingOutcome::Bounded(_) => {
            report.references_bounded += 1;
            if multi {
                stats.bounded += 1;
            }
        }
    }
}

fn reason_code(reason: UnresolvedReason) -> &'static str {
    match reason {
        UnresolvedReason::NoPath => "no-path",
        UnresolvedReason::NoDeclaration => "no-declaration",
        UnresolvedReason::NotAddressable => "not-addressable",
        UnresolvedReason::NotTargetable => "not-targetable",
    }
}

fn trace_line(lines: &LineIndex, r: &Reconciled) -> String {
    let at = match lines.range(r.reference.start_byte, r.reference.end_byte) {
        Ok(range) => format!("{}:{}", range.start_line + 1, range.start_character),
        Err(_) => format!("byte {}", r.reference.start_byte),
    };
    let detail = match &r.outcome {
        BindingOutcome::Resolved(symbol) => format!("resolved\t{}", symbol.as_str()),
        BindingOutcome::Ambiguous(symbols) => format!(
            "ambiguous\t{}",
            symbols
                .iter()
                .map(SymbolId::as_str)
                .collect::<Vec<_>>()
                .join(" | ")
        ),
        BindingOutcome::Unresolved(reason) => format!("unresolved\t{}", reason_code(*reason)),
        BindingOutcome::Bounded(bound) => format!("bounded\t{}", bound_code(*bound)),
    };
    format!(
        "{}:{at}\t{detail}\tends={} shadowed={} joined={} unjoined=[{}] targets=[{}]",
        r.reference.path.as_str(),
        r.ends,
        r.shadowed,
        r.joined,
        r.unjoined.join(","),
        r.targets.join(",")
    )
}

fn bound_code(bound: Bound) -> &'static str {
    match bound {
        Bound::Work => "work",
        Bound::QueueLength => "queue-length",
        Bound::ArenaCells => "arena-cells",
        Bound::Failed => "failed",
    }
}

/// Build the document: a definition occurrence for each declaration,
/// and an occurrence for each reference that resolved.
/// An unresolved, ambiguous, or bounded reference gets no occurrence.
fn build_document(language: &str, doc: Doc, reconciled: &[Reconciled]) -> Document {
    let mut occurrences = Vec::new();
    let mut symbols: BTreeMap<SymbolId, SymbolInfo> = BTreeMap::new();
    let mut defined: HashSet<(u32, u32)> = HashSet::new();
    for d in &doc.declared {
        // A module has no name token. Its definition is the empty range at the start of the
        // document, as in `scip-python`. The anchor that the engine uses can lie further on.
        let (start_byte, end_byte) = if d.fact.kind == DeclKind::Module {
            (0, 0)
        } else {
            defined.insert((d.fact.anchor.start_byte, d.fact.anchor.end_byte));
            (d.fact.anchor.start_byte, d.fact.anchor.end_byte)
        };
        occurrences.push(Occurrence {
            start_byte,
            end_byte,
            symbol: d.symbol.clone(),
            role: Role::Definition,
        });
        symbols
            .entry(d.symbol.clone())
            .or_insert_with(|| SymbolInfo {
                symbol: d.symbol.clone(),
                kind: d.kind,
                display_name: d.display_name.clone(),
                implements: Vec::new(),
                references: Vec::new(),
            });
    }
    // A second name of the module refers to the module.
    let module_symbol = doc
        .declared
        .iter()
        .find(|d| d.fact.kind == DeclKind::Module)
        .map(|d| d.symbol.clone());
    if let Some(module) = &module_symbol {
        for d in doc.declared.iter().filter(|d| {
            doc.facts
                .module_aliases
                .contains(&(d.fact.anchor.start_byte, d.fact.anchor.end_byte))
        }) {
            if let Some(info) = symbols.get_mut(&d.symbol) {
                info.references = vec![module.clone()];
            }
        }
    }
    let reads: BTreeSet<(u32, u32)> = doc.facts.reads.iter().copied().collect();
    let imports: BTreeSet<(u32, u32)> = doc.facts.imports.iter().copied().collect();
    let hidden: BTreeSet<(u32, u32)> = doc.facts.hidden.iter().copied().collect();
    for r in reconciled {
        let BindingOutcome::Resolved(symbol) = &r.outcome else {
            continue;
        };
        let span = (r.reference.start_byte, r.reference.end_byte);
        // A declaration site is a definition occurrence already.
        // A reference node with an empty span has no token to mark.
        if defined.contains(&span) || span.0 >= span.1 || hidden.contains(&span) {
            continue;
        }
        occurrences.push(Occurrence {
            start_byte: span.0,
            end_byte: span.1,
            symbol: symbol.clone(),
            role: if imports.contains(&span) {
                Role::Import
            } else if reads.contains(&span) {
                Role::ReadAccess
            } else {
                Role::Reference
            },
        });
    }
    Document {
        path: doc.path,
        language: language.to_string(),
        source: doc.source,
        occurrences,
        symbols: symbols.into_values().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(p: &str) -> ProjectRelativePath {
        ProjectRelativePath::new(p).unwrap()
    }

    #[test]
    fn a_file_goes_to_the_longest_shard_that_holds_it() {
        let shards = [path("code"), path("code/vendor"), path("lib")];
        assert_eq!(shard_of(&path("code/a.py"), &shards), Some("code"));
        assert_eq!(
            shard_of(&path("code/vendor/b.py"), &shards),
            Some("code/vendor")
        );
        assert_eq!(shard_of(&path("codex/c.py"), &shards), None);
        assert_eq!(shard_of(&path("tool.py"), &shards), None);
    }

    #[test]
    fn shards_keep_the_order_of_their_first_file() {
        let shards = [path("a"), path("b")];
        let found: Vec<FoundFile> = ["x.py", "b/1.py", "a/1.py", "b/2.py", "y.py"]
            .iter()
            .map(|p| FoundFile {
                path: path(p),
                pack: 0,
                real: None,
            })
            .collect();
        let files: Vec<&FoundFile> = found.iter().collect();
        assert_eq!(
            shard_groups(&files, &shards),
            [
                (None, vec![0, 4]),
                (Some("b"), vec![1, 3]),
                (Some("a"), vec![2])
            ]
        );
    }
}
