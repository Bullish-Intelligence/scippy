//! The indexing pipeline: discover, parse, extract, bind, reconcile, identify, emit.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::IndexRequest;
use crate::bindings::{BUILTINS_NAME, BindingEngine, Bound, RawBinding, ResolveLimits};
use crate::emit::{self, Document, IndexData, Occurrence, Role, SymbolInfo};
use crate::error::ScippyError;
use crate::extract::{DeclarationFact, Extractor, Facts};
use crate::identity::{Descriptor, IdentityBuilder, SymbolId};
use crate::inherit::Inheritance;
use crate::language::{Analysis, DeclKind, LanguagePack, ProjectModel, Reach, Scope};
use crate::positions::LineIndex;
use crate::project::{self, FoundFile, ProjectRelativePath, Router};
use crate::reconcile::{
    BindingOutcome, DeclarationIndex, Reconciled, Targeting, UnresolvedReason, reconcile,
};
use crate::report::{Diagnostic, IndexReport, MultiTargetStats, Severity, Stage};

/// The SCIP scheme of every symbol that Scippy writes.
/// It is Scippy-owned. Scippy does not claim the identifiers of another indexer.
pub(crate) const SCHEME: &str = "scippy";

struct PackRoute<'a>(&'a dyn LanguagePack);

impl Router for PackRoute<'_> {
    fn id(&self) -> &'static str {
        self.0.id()
    }
    fn accepts(&self, path: &Path) -> bool {
        self.0.accepts(path)
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
    let routes: Vec<PackRoute> = languages.iter().map(|p| PackRoute(*p)).collect();
    let routers: Vec<&dyn Router> = routes.iter().map(|r| r as &dyn Router).collect();
    let found = project::discover(&request.root, &request.output, &routers, request.limits)?;

    let mut report = IndexReport {
        files_discovered: found.files_discovered,
        diagnostics: found.diagnostics,
        ..Default::default()
    };
    let mut documents = Vec::new();
    let mut external_symbols: BTreeMap<SymbolId, SymbolInfo> = BTreeMap::new();
    let mut trace = Vec::new();

    for (index, pack) in languages.iter().enumerate() {
        let files: Vec<&FoundFile> = found.files.iter().filter(|f| f.pack == index).collect();
        if files.is_empty() {
            continue;
        }
        let Some(analysis) = pack.analysis() else {
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
        let indexed = index_pack(
            *pack,
            analysis,
            &found.root,
            &files,
            request,
            &mut report,
            &mut trace,
        )?;
        documents.extend(indexed.documents);
        external_symbols.extend(indexed.external_symbols);
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
    external_symbols: BTreeMap<SymbolId, SymbolInfo>,
}

fn index_pack(
    pack: &dyn LanguagePack,
    analysis: &dyn Analysis,
    root: &Path,
    files: &[&FoundFile],
    request: &IndexRequest,
    report: &mut IndexReport,
    trace: &mut Vec<String>,
) -> Result<PackIndex, ScippyError> {
    let paths: Vec<ProjectRelativePath> = files.iter().map(|f| f.path.clone()).collect();
    let model = analysis.project_model(&paths);
    let extractor = Extractor::new(pack.id(), analysis)?;
    let mut engine = BindingEngine::new(pack.id(), analysis)?;

    // Stages B to D, for each file: read, parse, extract, and build the binding graph.
    let mut docs: Vec<Doc> = Vec::new();
    for file in files {
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
            continue;
        }
        let source = match read_source(root, &file.path, request.limits.max_file_bytes) {
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
                continue;
            }
        };
        let tree = match extractor.parse(&source) {
            Ok(tree) => tree,
            Err(error) => {
                report.files_skipped += 1;
                report.diagnostics.push(diagnostic(
                    Stage::Parse,
                    Severity::Warning,
                    "syntax-error",
                    Some((error.start_byte, error.end_byte)),
                    "skipped: the file has a syntax error".to_string(),
                ));
                continue;
            }
        };
        let facts = extractor.facts(&file.path, &source, &tree);
        drop(tree);
        if let Err(failure) =
            engine.add_file(&file.path, &source, &model.resolver_globals(&file.path))
        {
            report.files_skipped += 1;
            report.diagnostics.push(diagnostic(
                Stage::Bind,
                Severity::Warning,
                "bind-error",
                None,
                format!("skipped: {}", failure.message),
            ));
            continue;
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
        docs.push(Doc {
            path: file.path.clone(),
            source,
            facts,
            declared: Vec::new(),
        });
    }

    // Stage F: assign identities.
    let mut package = analysis.default_package();
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
            index.insert_scoped(
                d.fact.anchor.clone(),
                d.symbol.clone(),
                d.scopes.clone(),
                d.declaration.clone(),
            );
        }
    }

    // The declarations of external assets, such as the builtins.
    let externals = declare_external(&extractor, analysis, &*model, report, &mut index);

    // Stage D: resolve. Stage E: reconcile.
    let mut by_document: BTreeMap<ProjectRelativePath, Vec<RawBinding>> = BTreeMap::new();
    for binding in engine.resolve(ResolveLimits::default()) {
        by_document
            .entry(binding.reference.path.clone())
            .or_default()
            .push(binding);
    }
    drop(engine);

    let language = pack.scip_language().to_string();
    let mut out = Vec::with_capacity(docs.len());
    let mut inheritance = Inheritance::default();
    // The document that defines each method that can override another.
    let mut method_documents: BTreeMap<SymbolId, usize> = BTreeMap::new();
    let mut referenced: BTreeMap<SymbolId, SymbolInfo> = BTreeMap::new();
    for doc in docs {
        let bindings = by_document.remove(&doc.path).unwrap_or_default();
        let members: HashSet<(u32, u32)> = doc.facts.members.iter().copied().collect();
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
            .collect();
        let targeting = Targeting::new(&*model, containers);
        let reconciled = reconcile(&index, bindings, &members, &targeting);
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
        external_symbols: referenced,
    })
}

/// A method that can override or be overridden: a function that a class directly holds,
/// with a symbol that other documents can address.
fn is_method(d: &Declared) -> bool {
    d.fact.kind == DeclKind::Function
        && !d.symbol.is_local()
        && d.scopes.last().is_some_and(|s| s.kind == DeclKind::Class)
}

/// The innermost class declaration whose extent holds the span.
fn enclosing_class(declared: &[Declared], start: u32, end: u32) -> Option<&Declared> {
    declared
        .iter()
        .filter(|d| d.fact.kind == DeclKind::Class)
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
    let Some(builtins) = analysis.binding_rules().builtins else {
        return symbols;
    };
    let warn = |code, message: String| Diagnostic {
        severity: Severity::Warning,
        stage: Stage::Identity,
        file: None,
        range: None,
        code,
        message,
    };
    let Ok(path) = ProjectRelativePath::new(BUILTINS_NAME) else {
        return symbols;
    };
    let tree = match extractor.parse(builtins.source) {
        Ok(tree) => tree,
        Err(_) => {
            report.diagnostics.push(warn(
                "external-asset-syntax-error",
                format!(
                    "the asset {} has a syntax error. Its names stay unresolved",
                    builtins.path
                ),
            ));
            return symbols;
        }
    };
    let facts = extractor.facts(&path, builtins.source, &tree);
    let declarations = &facts.declarations;
    let containers = container_indexes(declarations);
    for (i, fact) in declarations.iter().enumerate() {
        let scopes = scope_chain(declarations, &containers, i);
        let declaration = Scope {
            kind: fact.kind,
            name: fact.name.clone(),
        };
        let Some(identity) = model.external_identity(BUILTINS_NAME, &scopes, &declaration) else {
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
                });
            }
            Err(error) => report
                .diagnostics
                .push(warn("external-symbol-invalid", error.to_string())),
        }
    }
    symbols
}

/// Read a file. Return a diagnostic code and message on failure.
fn read_source(
    root: &Path,
    path: &ProjectRelativePath,
    max: u64,
) -> Result<String, (&'static str, String)> {
    let full = root.join(path.as_str());
    let mut bytes = Vec::new();
    File::open(&full)
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
    let facts = &doc.facts.declarations;
    let containers = container_indexes(facts);
    let mut locals: BTreeMap<Vec<Descriptor>, u32> = BTreeMap::new();
    let mut out = Vec::with_capacity(facts.len() + 1);
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
    for (i, fact) in facts.iter().enumerate() {
        let scopes = scope_chain(facts, &containers, i);
        let declaration = Scope {
            kind: fact.kind,
            name: fact.name.clone(),
        };
        let identity = model.identity(&doc.path, &scopes, &declaration);
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
            });
    }
    let imports: BTreeSet<(u32, u32)> = doc.facts.imports.iter().copied().collect();
    for r in reconciled {
        let BindingOutcome::Resolved(symbol) = &r.outcome else {
            continue;
        };
        let span = (r.reference.start_byte, r.reference.end_byte);
        // A declaration site is a definition occurrence already.
        if defined.contains(&span) {
            continue;
        }
        occurrences.push(Occurrence {
            start_byte: span.0,
            end_byte: span.1,
            symbol: symbol.clone(),
            role: if imports.contains(&span) {
                Role::Import
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
