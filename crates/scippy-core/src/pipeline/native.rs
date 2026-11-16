//! The native pack path: the pack declares and resolves, the core finishes.

use super::finish::Finisher;
use super::identities::{assign_identities, identity_builder};
use super::load::read_source;
use super::{Doc, PackIndex, PackRun, SCHEME};
use crate::bindings::{Place, RawBinding, RawEnd};
use crate::emit::SymbolInfo;
use crate::error::ScippyError;
use crate::extract::{DeclarationFact, Facts, SourceAnchor};
use crate::identity::{IdentityBuilder, SymbolId};
use crate::language::{
    DeclKind, NativeAnalysis, NativeDocument, NativeInput, NativeSource, NativeTarget, ProjectFile,
    ReferenceRole, Scope,
};
use crate::project::ProjectRelativePath;
use crate::reconcile::{BindingOutcome, DeclarationIndex, Targeting, UnresolvedReason, reconcile};
use crate::report::{Diagnostic, Severity, Stage};
use crate::snapshot::AnalysisEdge;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// Index the files of a native pack: the pack declares and resolves, the core finishes.
///
/// The pack gives declarations and the targets of each reference. The core joins each target to a
/// declaration by document and span, applies the policy of the project model, assigns the symbol
/// identities, and builds the documents. The shadow, bounded, and nearest-binding rules of the
/// reconciler have nothing to act on, because a native reference has no paths.
pub(super) fn index_native_pack(
    run: &mut PackRun,
    native: &dyn NativeAnalysis,
    all_files: &[ProjectFile],
    extra: Option<&(dyn std::any::Any + Send + Sync)>,
) -> Result<PackIndex, ScippyError> {
    let PackRun {
        pack,
        root,
        files,
        store,
        request,
        ..
    } = *run;
    // Read the files that the pack accepts.
    let mut sources: Vec<NativeSource> = Vec::with_capacity(files.len());
    for file in files {
        if let Some(text) = read_source(store, file, run.report) {
            sources.push(NativeSource {
                path: file.path.clone(),
                text,
            });
        }
    }
    let output = native.analyze(&NativeInput {
        root,
        files: all_files,
        sources: &sources,
        extra,
    });
    run.report.diagnostics.extend(output.diagnostics);
    if let Some(capture) = run.capture.as_deref_mut() {
        for link in &output.links {
            capture.edges.push(AnalysisEdge {
                kind: if link.embed { "embed" } else { "link" }.to_owned(),
                path: link.path.as_str().to_owned(),
                source: None,
                span: link.span,
                reference_span: link.reference_span,
                anchor_span: link.anchor_span,
                link_kind: Some(link.kind.clone()),
                label: link.label.clone(),
                state: Some(link.state.to_owned()),
                anchor_state: Some(link.anchor_state.to_owned()),
            });
        }
    }
    let model = native.project_model(root);

    let package = model.package().unwrap_or_else(|| native.default_package());
    let mut builder = identity_builder(request, package);
    let mut texts: HashMap<ProjectRelativePath, Arc<str>> =
        sources.into_iter().map(|s| (s.path, s.text)).collect();

    // Stage F: assign identities. Each document gets its facts from the declarations of the pack.
    let mut index = DeclarationIndex::default();
    let mut docs: Vec<(Doc, &NativeDocument)> = Vec::with_capacity(output.documents.len());
    for native_doc in &output.documents {
        let source: Arc<str> = if native_doc.stub {
            Arc::from("")
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
                digests: None,
            });
        }
        for d in &native_doc.declarations {
            facts.declarations.push(DeclarationFact {
                anchor: anchor(d.span),
                name: d.name.clone(),
                kind: d.kind,
                extent: d.span,
                extra_anchors: Vec::new(),
                digests: None,
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
        // The pack gives the facts of notes, headings, and blocks. They join by the span.
        let notes: HashMap<(u32, u32), &crate::snapshot::NoteRecord> = native_doc
            .declarations
            .iter()
            .filter_map(|d| d.note.as_ref().map(|n| (d.span, n)))
            .collect();
        for d in &mut doc.declared {
            let record = if d.fact.kind == DeclKind::Module {
                native_doc.note.as_ref()
            } else {
                let anchor = &d.fact.anchor;
                notes.get(&(anchor.start_byte, anchor.end_byte)).copied()
            };
            if let (Some(record), Some(decl)) = (record, d.decl.as_mut()) {
                decl.note = Some(record.clone());
            }
        }
        run.report.declarations += doc.declared.len();
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
                    decl: None,
                });
            }
            Err(error) => run.report.diagnostics.push(Diagnostic {
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
                    echo: Vec::new(),
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
    let mut finisher = Finisher::new(pack.scip_language().to_string(), true, &externals);
    let mut out = Vec::new();
    let mut stubs = Vec::new();
    for (doc, native_doc) in docs {
        let bindings = by_document.remove(&doc.path).unwrap_or_default();
        let targeting = Targeting::new(&*model, Vec::new());
        let mut reconciled = reconcile(
            &index,
            bindings,
            &HashSet::new(),
            &HashSet::new(),
            &targeting,
        );
        // A reference to a skipped file has no end. The pack says so, and the reason is more exact.
        let skipped: HashSet<(u32, u32)> = native_doc
            .references
            .iter()
            .filter(|r| r.target_skipped)
            .map(|r| r.span)
            .collect();
        for r in &mut reconciled {
            let span = (r.reference.start_byte, r.reference.end_byte);
            if skipped.contains(&span)
                && r.outcome == BindingOutcome::Unresolved(UnresolvedReason::NoPath)
            {
                r.outcome = BindingOutcome::Unresolved(UnresolvedReason::TargetSkipped);
            }
        }
        finisher.account(run, &doc, &reconciled);
        let built = finisher.build(doc, &reconciled, native_doc.stub);
        if native_doc.stub {
            stubs.push(built);
        } else {
            out.push(built);
        }
    }
    Ok(PackIndex {
        documents: out,
        stubs,
        external_symbols: finisher.into_referenced(),
    })
}
