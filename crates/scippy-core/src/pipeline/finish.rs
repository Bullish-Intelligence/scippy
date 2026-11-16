//! The finishing stage of a document: count, trace, inheritance, and build.

use super::{Declared, Doc, PackRun};
use crate::bindings::Bound;
use crate::emit::{Document, Occurrence, Role, SymbolInfo};
use crate::identity::SymbolId;
use crate::inherit::Inheritance;
use crate::language::DeclKind;
use crate::positions::LineIndex;
use crate::project::ProjectRelativePath;
use crate::reconcile::{BindingOutcome, Reconciled, UnresolvedReason};
use crate::report::{IndexReport, LostFile, MultiTargetStats};
use crate::snapshot::{AnalysisEdge, AnalysisReference, Capture};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// A method that can override or be overridden: a function that a class directly holds,
/// with a symbol that other documents can address.
pub(super) fn is_method(d: &Declared) -> bool {
    d.fact.kind == DeclKind::Function
        && !d.symbol.is_local()
        && d.scopes.last().is_some_and(|s| s.kind.is_class_like())
}

/// The innermost class declaration whose extent holds the span.
pub(super) fn enclosing_class(declared: &[Declared], start: u32, end: u32) -> Option<&Declared> {
    declared
        .iter()
        .filter(|d| d.fact.kind.is_class_like())
        .filter(|d| d.fact.extent.0 <= start && end <= d.fact.extent.1)
        .min_by_key(|d| d.fact.extent.1 - d.fact.extent.0)
}

/// Add the supertypes and the methods of the classes of one document.
pub(super) fn record_inheritance(
    doc: &Doc,
    reconciled: &[Reconciled],
    inheritance: &mut Inheritance,
    capture: &mut Option<Capture>,
) {
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
            if let Some(capture) = capture {
                capture.edges.push(AnalysisEdge {
                    kind: "inheritance".to_owned(),
                    path: doc.path.as_str().to_owned(),
                    source: Some(class.symbol.as_str().to_owned()),
                    span: (start, end),
                    reference_span: Some((start, end)),
                    anchor_span: None,
                    link_kind: None,
                    label: None,
                    state: None,
                    anchor_state: None,
                });
            }
        }
    }
}

/// Record a file that the run accepted and did not index. The run fills the pack and the real
/// path of the file at the end.
pub(super) fn lose(report: &mut IndexReport, path: &ProjectRelativePath, reason: &'static str) {
    report.lost_files.push(LostFile {
        path: path.as_str().to_owned(),
        pack: None,
        real_path: None,
        reason,
        sha256: None,
    });
}

pub(super) fn count(report: &mut IndexReport, r: &Reconciled) {
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
        BindingOutcome::Bounded(bound) => {
            report.references_bounded += 1;
            let kinds = &mut report.bounded_by_kind;
            match bound {
                Bound::Work => kinds.work += 1,
                Bound::QueueLength => kinds.queue_length += 1,
                Bound::ArenaCells => kinds.arena_cells += 1,
                Bound::Failed => kinds.failed += 1,
            }
            if multi {
                stats.bounded += 1;
            }
        }
    }
}

pub(super) fn reason_code(reason: UnresolvedReason) -> &'static str {
    match reason {
        UnresolvedReason::NoPath => "no-path",
        UnresolvedReason::NoDeclaration => "no-declaration",
        UnresolvedReason::NotAddressable => "not-addressable",
        UnresolvedReason::NotTargetable => "not-targetable",
        UnresolvedReason::TargetSkipped => "target-skipped",
    }
}

pub(super) fn trace_line(lines: &LineIndex, r: &Reconciled) -> String {
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
        BindingOutcome::Bounded(bound) => format!("bounded\t{}", bound.code()),
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

/// The finishing stage that both pack paths share. After a path reconciles a document, the
/// finisher counts the references, captures and traces them, and builds the document.
pub(super) struct Finisher<'a> {
    /// The SCIP language of the documents.
    language: String,
    /// True for a native pack. It changes how the snapshot records a reference.
    native: bool,
    /// The external symbols that a reference can resolve to.
    externals: &'a BTreeMap<SymbolId, SymbolInfo>,
    /// The external symbols that a reference resolved to.
    referenced: BTreeMap<SymbolId, SymbolInfo>,
}

impl<'a> Finisher<'a> {
    pub(super) fn new(
        language: String,
        native: bool,
        externals: &'a BTreeMap<SymbolId, SymbolInfo>,
    ) -> Self {
        Self {
            language,
            native,
            externals,
            referenced: BTreeMap::new(),
        }
    }

    /// Count the references of one document, capture them for the snapshot, and trace them.
    pub(super) fn account(&mut self, run: &mut PackRun, doc: &Doc, reconciled: &[Reconciled]) {
        for r in reconciled {
            count(run.report, r);
            if let Some(capture) = run.capture {
                capture
                    .references
                    .push(AnalysisReference::from_reconciled(r, self.native));
            }
            if let BindingOutcome::Resolved(symbol) = &r.outcome {
                if let Some(info) = self.externals.get(symbol) {
                    run.report.references_external += 1;
                    self.referenced
                        .entry(symbol.clone())
                        .or_insert_with(|| info.clone());
                }
            }
        }
        if run.request.trace.is_some() {
            let lines = LineIndex::new(&doc.source);
            run.trace
                .extend(reconciled.iter().map(|r| trace_line(&lines, r)));
        }
    }

    /// Build the document. A stub document has no language.
    pub(super) fn build(&self, doc: Doc, reconciled: &[Reconciled], stub: bool) -> Document {
        let language = if stub { "" } else { &self.language };
        build_document(language, doc, reconciled)
    }

    /// The external symbols that the references of all documents resolved to.
    pub(super) fn into_referenced(self) -> BTreeMap<SymbolId, SymbolInfo> {
        self.referenced
    }
}

/// Build the document: a definition occurrence for each declaration,
/// and an occurrence for each reference that resolved.
/// An unresolved, ambiguous, or bounded reference gets no occurrence.
pub(super) fn build_document(language: &str, doc: Doc, reconciled: &[Reconciled]) -> Document {
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
                decl: d.decl.clone(),
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
