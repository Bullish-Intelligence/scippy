//! The indexing pipeline: discover, parse, extract, bind, reconcile, identify, emit.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::IndexRequest;
use crate::bindings::{BindingEngine, Bound, RawBinding, ResolveLimits};
use crate::emit::{self, Document, IndexData, Occurrence, Role, SymbolInfo};
use crate::error::ScippyError;
use crate::extract::{DeclarationFact, Extractor, Facts};
use crate::identity::{Descriptor, IdentityBuilder, SymbolId};
use crate::language::{Analysis, LanguagePack, ProjectModel, Reach, Scope};
use crate::positions::LineIndex;
use crate::project::{self, FoundFile, ProjectRelativePath, Router};
use crate::reconcile::{BindingOutcome, DeclarationIndex, Reconciled, UnresolvedReason, reconcile};
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
        let pack_documents = index_pack(
            *pack,
            analysis,
            &found.root,
            &files,
            request,
            &mut report,
            &mut trace,
        )?;
        documents.extend(pack_documents);
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
        },
        &request.output,
    )?;
    Ok(report)
}

fn index_pack(
    pack: &dyn LanguagePack,
    analysis: &dyn Analysis,
    root: &Path,
    files: &[&FoundFile],
    request: &IndexRequest,
    report: &mut IndexReport,
    trace: &mut Vec<String>,
) -> Result<Vec<Document>, ScippyError> {
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
            index.insert(d.fact.anchor.clone(), d.symbol.clone());
        }
    }

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
    for doc in docs {
        let bindings = by_document.remove(&doc.path).unwrap_or_default();
        let members: HashSet<(u32, u32)> = doc.facts.members.iter().copied().collect();
        let reconciled = reconcile(&index, bindings, &members);
        for r in &reconciled {
            count(report, r);
        }
        if request.trace.is_some() {
            let lines = LineIndex::new(&doc.source);
            trace.extend(reconciled.iter().map(|r| trace_line(&lines, r)));
        }
        out.push(build_document(&language, doc, &reconciled));
    }
    Ok(out)
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

/// Give each declaration of a document its symbol.
fn assign_identities(
    model: &dyn ProjectModel,
    builder: &mut IdentityBuilder,
    doc: &Doc,
) -> Result<Vec<Declared>, ScippyError> {
    let facts = &doc.facts.declarations;
    let containers: Vec<usize> = (0..facts.len())
        .filter(|&i| facts[i].kind.is_container())
        .collect();
    let mut locals: BTreeMap<Vec<Descriptor>, u32> = BTreeMap::new();
    let mut out = Vec::with_capacity(facts.len());
    for (i, fact) in facts.iter().enumerate() {
        let mut ancestors: Vec<&DeclarationFact> = containers
            .iter()
            .filter(|&&j| {
                j != i && facts[j].extent.0 <= fact.extent.0 && fact.extent.1 <= facts[j].extent.1
            })
            .map(|&j| &facts[j])
            .collect();
        ancestors.sort_by_key(|a| (a.extent.0, std::cmp::Reverse(a.extent.1)));
        let scopes: Vec<Scope> = ancestors
            .iter()
            .map(|a| Scope {
                kind: a.kind,
                name: a.name.clone(),
            })
            .collect();
        let identity = model.identity(
            &doc.path,
            &scopes,
            &Scope {
                kind: fact.kind,
                name: fact.name.clone(),
            },
        );
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
        defined.insert((d.fact.anchor.start_byte, d.fact.anchor.end_byte));
        occurrences.push(Occurrence {
            start_byte: d.fact.anchor.start_byte,
            end_byte: d.fact.anchor.end_byte,
            symbol: d.symbol.clone(),
            role: Role::Definition,
        });
        symbols
            .entry(d.symbol.clone())
            .or_insert_with(|| SymbolInfo {
                symbol: d.symbol.clone(),
                kind: d.kind,
                display_name: d.display_name.clone(),
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
