//! The stack-graphs pack path: shards, binding engines, reconcile, and identities.

use super::finish::{Finisher, is_method, record_inheritance};
use super::identities::{anonymous_scope, assign_identities, declare_external, identity_builder};
use super::inputs::read_manifests;
use super::load::load_file;
use super::memory::{MemoryMeter, shard_estimate};
use super::shard::{dependency_closure, shard_groups, shard_of};
use super::{Doc, PackIndex, PackRun};
use crate::IndexRequest;
use crate::bindings::{BindingEngine, CompiledRules, RawBinding};
use crate::cache::CacheGate;
use crate::error::ScippyError;
use crate::extract::{Extractor, SourceAnchor};
use crate::identity::{IdentityBuilder, SymbolId};
use crate::inherit::Inheritance;
use crate::language::{Analysis, DeclKind, ExtraFile, LanguagePack, ProjectModel, Scope};
use crate::project::{FoundFile, ProjectRelativePath};
use crate::reconcile::{DeclarationIndex, Targeting, reconcile};
use crate::report::{Diagnostic, IndexReport, Severity, Stage};
use crate::schedule;
use crate::source::SourceStore;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

pub(super) fn index_pack(
    run: &mut PackRun,
    analysis: &dyn Analysis,
    shards: &[ProjectRelativePath],
    extra_files: &[ExtraFile],
    cache: &Arc<CacheGate>,
) -> Result<PackIndex, ScippyError> {
    let PackRun {
        pack,
        root,
        files,
        store,
        request,
        ..
    } = *run;
    let paths: Vec<ProjectRelativePath> = files.iter().map(|f| f.path.clone()).collect();
    let (manifests, problems) = analysis
        .manifest_name()
        .map(|name| read_manifests(root, name, shards, extra_files))
        .unwrap_or_default();
    run.report.diagnostics.extend(problems);
    run.report
        .diagnostics
        .extend(analysis.input_problems(root, &paths, &manifests));
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
    // Stages B to D, one shard at a time or several at once. Each shard gets a binding engine
    // of its own, and the engine resolves the references of the shard. A worker frees the engine
    // before it takes the next shard. The engine never leaves its worker. Each worker compiles
    // the rules once and shares them across its shards. The scheduler admits a shard only when
    // the estimates of the running shards stay under the memory budget.
    // The pipeline merges the results in shard order, so the output does not depend on the
    // number of workers.
    let groups = shard_groups(files, shards);
    let members: HashMap<&str, &[usize]> = groups
        .iter()
        .filter_map(|(shard, group)| shard.map(|s| (s, &group[..])))
        .collect();
    let context = ShardContext {
        pack,
        analysis,
        extractors: &extractors,
        model: &*model,
        root,
        store,
        files,
        paths: &paths,
        shards,
        members: &members,
        request,
        cache,
    };
    let jobs: Vec<(u64, usize)> = groups
        .iter()
        .enumerate()
        .map(|(n, (shard, group))| (context.estimate(*shard, group), n))
        .collect();
    let log = std::env::var_os("SCIPPY_SHARD_LOG").is_some();
    if log {
        eprintln!(
            "shard log: the memory figures are resident memory of the whole process; \
             they are exact for one shard only with --jobs 1"
        );
    }
    let results = schedule::run_ordered(
        jobs,
        request.jobs,
        request.memory_budget,
        || None::<CompiledRules>,
        |compiled, estimate, n| {
            let (shard, group) = &groups[n];
            let rules = match compiled {
                Some(rules) => rules,
                None => compiled.insert(CompiledRules::new(pack.id(), analysis)?),
            };
            let meter = log.then(MemoryMeter::start);
            let out = index_shard(&context, rules, *shard, group);
            if let Some(meter) = meter {
                let (start, peak) = meter.finish();
                eprintln!(
                    "shard {} files {} estimate {} start_rss_kib {} peak_rss_kib {}",
                    shard.unwrap_or("."),
                    group.len(),
                    estimate,
                    start,
                    peak
                );
            }
            out
        },
    );
    run.report.graph_cache_enabled = request.graph_cache.is_some();
    let (mut docs, mut by_document) = merge_shard_outputs(results, run.report)?;
    // Stage F: assign identities.
    let package = model
        .package()
        .unwrap_or_else(|| analysis.default_package());
    let mut builder = identity_builder(request, package);
    let mut index = DeclarationIndex::default();
    declare_documents(&mut docs, &*model, &mut builder, run.report, &mut index)?;

    // The declarations of external assets, such as the builtins.
    let externals = declare_external(&extractors[0], analysis, &*model, run.report, &mut index);

    // A binding name takes the target of its reference. It joins as a module-like binding.
    bind_aliases(&docs, &by_document, &mut index);

    // Stage E: reconcile.
    let mut out = Vec::with_capacity(docs.len());
    let mut inheritance = Inheritance::default();
    // The document that defines each method that can override another.
    let mut method_documents: BTreeMap<SymbolId, usize> = BTreeMap::new();
    let mut finisher = Finisher::new(pack.scip_language().to_string(), false, &externals);
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
        finisher.account(run, &doc, &reconciled);
        record_inheritance(
            &doc,
            &reconciled,
            &mut inheritance,
            run.capture.as_deref_mut(),
        );
        for d in doc.declared.iter().filter(|d| is_method(d)) {
            method_documents.insert(d.symbol.clone(), out.len());
        }
        out.push(finisher.build(doc, &reconciled, false));
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
        external_symbols: finisher.into_referenced(),
    })
}

/// The raw bindings of the references of each document.
type Bindings = BTreeMap<ProjectRelativePath, Vec<RawBinding>>;

/// Merge the outputs of the shards into the report. Return the documents in the order of the
/// walk, and the raw bindings of each document.
fn merge_shard_outputs(
    results: Vec<Result<ShardOutput, ScippyError>>,
    report: &mut IndexReport,
) -> Result<(Vec<Doc>, Bindings), ScippyError> {
    let mut docs: Vec<(usize, Doc)> = Vec::new();
    let mut by_document: Bindings = BTreeMap::new();
    for result in results {
        let out = result?;
        report.files_skipped += out.report.files_skipped;
        report.files_shadowed += out.report.files_shadowed;
        report.lost_files.extend(out.report.lost_files);
        report.graph_cache_hits += out.report.graph_cache_hits;
        report.graph_cache_misses += out.report.graph_cache_misses;
        report.search_work.merge(&out.report.search_work);
        report.diagnostics.extend(out.report.diagnostics);
        docs.extend(out.docs);
        for binding in out.bindings {
            by_document
                .entry(binding.reference.path.clone())
                .or_default()
                .push(binding);
        }
    }
    // The later stages see the documents in the order of the walk.
    docs.sort_by_key(|(at, _)| *at);
    let docs = docs.into_iter().map(|(_, doc)| doc).collect();
    Ok((docs, by_document))
}

/// Stage F: give each declaration its symbol, and add it to the declaration index.
fn declare_documents(
    docs: &mut [Doc],
    model: &dyn ProjectModel,
    builder: &mut IdentityBuilder,
    report: &mut IndexReport,
    index: &mut DeclarationIndex,
) -> Result<(), ScippyError> {
    for doc in docs {
        doc.declared = assign_identities(model, builder, doc)?;
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
    Ok(())
}

/// A binding name takes the target of its reference. It joins as a module-like binding.
fn bind_aliases(docs: &[Doc], by_document: &Bindings, index: &mut DeclarationIndex) {
    for doc in docs {
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
}

/// What one shard gives back.
pub(super) struct ShardOutput {
    docs: Vec<(usize, Doc)>,
    bindings: Vec<RawBinding>,
    /// The files that the shard skipped and the diagnostics of the shard.
    report: IndexReport,
}

/// What every shard reads. It is shared by all workers.
pub(super) struct ShardContext<'a> {
    pack: &'a dyn LanguagePack,
    analysis: &'a dyn Analysis,
    extractors: &'a [Extractor],
    model: &'a dyn ProjectModel,
    root: &'a Path,
    /// The content of every file. A shard never reads a file from disk.
    store: &'a SourceStore,
    files: &'a [&'a FoundFile],
    paths: &'a [ProjectRelativePath],
    shards: &'a [ProjectRelativePath],
    members: &'a HashMap<&'a str, &'a [usize]>,
    request: &'a IndexRequest,
    /// The state of the graph cache for the whole run.
    cache: &'a Arc<CacheGate>,
}

impl ShardContext<'_> {
    fn estimate(&self, shard: Option<&str>, group: &[usize]) -> u64 {
        let size = |at: usize| {
            std::fs::metadata(self.files[at].disk_path(self.root))
                .map_or(0, |m| m.len().min(self.request.limits.max_file_bytes))
        };
        let mut bytes: u64 = group.iter().map(|&at| size(at)).sum();
        for dependency in dependency_closure(self.model, shard.unwrap_or_default(), self.members) {
            for &at in self.members[dependency.as_str()] {
                if self.model.provides(&self.files[at].path) {
                    bytes += size(at);
                }
            }
        }
        shard_estimate(bytes)
    }
}

/// Stages B to D for one shard: build the graphs, then resolve the references.
pub(super) fn index_shard(
    cx: &ShardContext,
    compiled: &CompiledRules,
    shard: Option<&str>,
    group: &[usize],
) -> Result<ShardOutput, ScippyError> {
    let mut report = IndexReport::default();
    let mut engine = BindingEngine::new(cx.pack.id(), cx.analysis, compiled, cx.cache)?;
    let mut docs: Vec<(usize, Doc)> = Vec::new();
    for &at in group {
        let file = cx.files[at];
        if let Some(doc) = load_file(
            cx.analysis,
            cx.extractors,
            cx.model,
            &mut engine,
            cx.store,
            file,
            &mut report,
            true,
        ) {
            docs.push((at, doc));
        }
    }
    // The code that the shard imports from other shards. The engine reads it to resolve
    // the references of the shard. The shard that owns the code indexes it.
    for dependency in dependency_closure(cx.model, shard.unwrap_or_default(), cx.members) {
        for &at in cx.members[dependency.as_str()] {
            let file = cx.files[at];
            if cx.model.provides(&file.path) {
                let mut scratch = IndexReport::default();
                load_file(
                    cx.analysis,
                    cx.extractors,
                    cx.model,
                    &mut engine,
                    cx.store,
                    file,
                    &mut scratch,
                    false,
                );
            }
        }
    }
    // The support files of the shard, such as `tsconfig.json`. An analyzer is not `Send`,
    // so the worker makes its own.
    let sources: Vec<ProjectRelativePath> = docs.iter().map(|(_, d)| d.path.clone()).collect();
    for support in cx.analysis.support_files(cx.root, cx.paths) {
        if shard_of(&support.path, cx.shards) != shard {
            continue;
        }
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
    report.graph_cache_hits = engine.cache_hits;
    report.graph_cache_misses = engine.cache_misses;
    let (bindings, search_work) = engine.resolve(cx.request.resolve_limits);
    report.search_work = search_work;
    Ok(ShardOutput {
        docs,
        bindings,
        report,
    })
}
