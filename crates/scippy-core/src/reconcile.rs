//! The source-anchor reconciler.
//!
//! The engine returns the ends of binding paths as source anchors.
//! The reconciler joins each end to a declaration fact by file and exact byte span.
//! It never joins by name.
//!
//! Policy for one reference location:
//! 1. Reference nodes at one span merge. Their ends form one set.
//! 2. A bounded search gives no target. It can lack ends.
//! 3. An end that joins to no declaration is a binding, such as an import alias or a module.
//!    The final definition replaces it.
//! 4. The ends that join to declarations give the symbols. One distinct symbol means Resolved.
//!    Two or more distinct symbols mean Ambiguous. No symbol means Unresolved.
//! 5. A reference never takes a document-local symbol of another document.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::bindings::{Bound, RawBinding, RawEnd};
use crate::extract::SourceAnchor;
use crate::identity::SymbolId;

/// Why a reference has no target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum UnresolvedReason {
    /// The engine found no path.
    NoPath,
    /// The engine found paths, but no end joins to a declaration.
    /// The ends are bindings, builtins, or modules.
    NoDeclaration,
    /// The only declarations are document-local symbols of another document.
    NotAddressable,
}

/// The normalized outcome of one reference location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BindingOutcome {
    Resolved(SymbolId),
    Ambiguous(Vec<SymbolId>),
    Unresolved(UnresolvedReason),
    /// A work bound stopped the search. The set of ends is incomplete.
    Bounded(Bound),
}

/// Declaration anchors with their symbols. It is the join table.
#[derive(Default)]
pub(crate) struct DeclarationIndex {
    by_anchor: HashMap<SourceAnchor, SymbolId>,
}

impl DeclarationIndex {
    pub fn insert(&mut self, anchor: SourceAnchor, symbol: SymbolId) {
        self.by_anchor.insert(anchor, symbol);
    }

    pub fn get(&self, anchor: &SourceAnchor) -> Option<&SymbolId> {
        self.by_anchor.get(anchor)
    }
}

/// One reference location after the merge of its reference nodes.
#[derive(Debug)]
pub(crate) struct Reconciled {
    pub reference: SourceAnchor,
    pub outcome: BindingOutcome,
    /// Distinct ends of the merged search.
    pub ends: usize,
    /// Ends that only shadowed paths reach. The reconciler ignores them.
    pub shadowed: usize,
    /// Ends outside the indexed project, such as builtins.
    pub outside: usize,
    /// Ends that joined to a declaration, counted by anchor.
    pub joined: usize,
    /// The ends that joined no declaration, as `syntax_type` or `-`.
    pub unjoined: Vec<String>,
    /// The joined declarations as `file:start`, for the trace.
    pub targets: Vec<String>,
}

/// Merge the raw bindings of one document by reference span and reconcile each location.
///
/// The result is sorted by span.
pub(crate) fn reconcile(
    declarations: &DeclarationIndex,
    bindings: Vec<RawBinding>,
) -> Vec<Reconciled> {
    type Merged = (BTreeSet<RawEnd>, BTreeSet<RawEnd>, Option<Bound>);
    let mut merged: BTreeMap<SourceAnchor, Merged> = BTreeMap::new();
    for binding in bindings {
        let entry = merged.entry(binding.reference).or_default();
        entry.0.extend(binding.ends);
        entry.1.extend(binding.shadowed_ends);
        entry.2 = entry.2.or(binding.bound);
    }

    merged
        .into_iter()
        .map(|(reference, (ends, shadowed, bound))| {
            // A node that is shadowed in one search and kept in another is not shadowed.
            let shadowed: BTreeSet<RawEnd> = shadowed.difference(&ends).cloned().collect();
            let mut joined: BTreeSet<&SourceAnchor> = BTreeSet::new();
            let mut usable: BTreeSet<SymbolId> = BTreeSet::new();
            let mut foreign = false;
            let mut unjoined = Vec::new();
            let mut targets = Vec::new();
            let outside = ends.iter().filter(|e| e.anchor.is_none()).count();
            for end in &ends {
                let target = end
                    .anchor
                    .as_ref()
                    .and_then(|a| declarations.get(a).map(|symbol| (a, symbol)));
                let Some((anchor, symbol)) = target else {
                    unjoined.push(end.syntax_type.clone().unwrap_or_else(|| "-".into()));
                    continue;
                };
                joined.insert(anchor);
                targets.push(format!("{}:{}", anchor.path.as_str(), anchor.start_byte));
                // A document-local symbol is valid only in the document that declares it.
                if symbol.is_local() && anchor.path != reference.path {
                    foreign = true;
                } else {
                    usable.insert(symbol.clone());
                }
            }

            let outcome = if let Some(bound) = bound {
                BindingOutcome::Bounded(bound)
            } else if ends.is_empty() {
                BindingOutcome::Unresolved(UnresolvedReason::NoPath)
            } else if joined.is_empty() {
                BindingOutcome::Unresolved(UnresolvedReason::NoDeclaration)
            } else if usable.is_empty() {
                BindingOutcome::Unresolved(UnresolvedReason::NotAddressable)
            } else if usable.len() == 1 && !foreign {
                BindingOutcome::Resolved(usable.into_iter().next().expect("one symbol"))
            } else {
                BindingOutcome::Ambiguous(usable.into_iter().collect())
            };
            Reconciled {
                reference,
                outcome,
                ends: ends.len(),
                shadowed: shadowed.len(),
                outside,
                joined: joined.len(),
                unjoined,
                targets,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::RawEnd;
    use crate::identity::{Descriptor, IdentityBuilder, Package};
    use crate::project::ProjectRelativePath;

    fn anchor(path: &str, start: u32) -> SourceAnchor {
        SourceAnchor {
            path: ProjectRelativePath::new(path).unwrap(),
            start_byte: start,
            end_byte: start + 3,
        }
    }

    fn end(path: &str, start: u32, syntax: Option<&str>) -> RawEnd {
        RawEnd {
            anchor: Some(anchor(path, start)),
            syntax_type: syntax.map(str::to_string),
        }
    }

    fn binding(reference: SourceAnchor, ends: Vec<RawEnd>, bound: Option<Bound>) -> RawBinding {
        RawBinding {
            reference,
            ends,
            shadowed_ends: vec![],
            bound,
        }
    }

    /// `sym(0)` and `sym(1)` are global symbols. `sym(9)` is a local symbol.
    fn sym(n: u32) -> SymbolId {
        if n == 9 {
            return SymbolId::local(0);
        }
        let builder = IdentityBuilder::new(
            "scippy",
            Package {
                manager: "test".into(),
                name: "p".into(),
                version: String::new(),
            },
        );
        builder
            .symbol(&[Descriptor::Term(format!("s{n}"))])
            .unwrap()
    }

    fn index() -> DeclarationIndex {
        let mut index = DeclarationIndex::default();
        index.insert(anchor("a.py", 0), sym(0));
        index.insert(anchor("a.py", 10), sym(1));
        index.insert(anchor("b.py", 0), sym(0));
        index.insert(anchor("c.py", 5), sym(9));
        index
    }

    fn outcome(bindings: Vec<RawBinding>) -> BindingOutcome {
        reconcile(&index(), bindings).remove(0).outcome
    }

    #[test]
    fn an_alias_binding_and_a_final_definition_resolve_to_the_definition() {
        let reference = anchor("app.py", 50);
        let ends = vec![end("app.py", 20, None), end("a.py", 0, Some("function"))];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Resolved(sym(0))
        );
    }

    #[test]
    fn duplicate_reference_nodes_at_one_span_merge() {
        let reference = anchor("app.py", 50);
        let one = binding(reference.clone(), vec![end("a.py", 0, None)], None);
        let two = binding(
            reference,
            vec![end("a.py", 0, None), end("app.py", 20, None)],
            None,
        );
        let all = reconcile(&index(), vec![one, two]);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].outcome, BindingOutcome::Resolved(sym(0)));
        assert_eq!(all[0].ends, 2);
        assert_eq!(all[0].joined, 1);
    }

    #[test]
    fn two_distinct_definitions_are_ambiguous() {
        let reference = anchor("app.py", 50);
        let ends = vec![end("a.py", 0, None), end("a.py", 10, None)];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Ambiguous(vec![sym(0), sym(1)])
        );
    }

    #[test]
    fn rebinding_of_one_entity_is_not_ambiguous() {
        // `a.py:0` and `b.py:0` stand for two sites of one symbol.
        let reference = anchor("app.py", 50);
        let ends = vec![end("a.py", 0, None), end("b.py", 0, None)];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Resolved(sym(0))
        );
    }

    #[test]
    fn a_name_alone_never_joins() {
        // The end has the right span in another file. No declaration lives there.
        let reference = anchor("app.py", 50);
        let ends = vec![end("c.py", 0, Some("function"))];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Unresolved(UnresolvedReason::NoDeclaration)
        );
    }

    #[test]
    fn no_end_is_unresolved() {
        let reference = anchor("app.py", 50);
        assert_eq!(
            outcome(vec![binding(reference, vec![], None)]),
            BindingOutcome::Unresolved(UnresolvedReason::NoPath)
        );
    }

    #[test]
    fn a_bounded_search_gives_no_target() {
        let reference = anchor("app.py", 50);
        let ends = vec![end("a.py", 0, None)];
        assert_eq!(
            outcome(vec![binding(reference, ends, Some(Bound::Work))]),
            BindingOutcome::Bounded(Bound::Work)
        );
    }

    #[test]
    fn an_end_outside_the_project_is_unresolved() {
        let reference = anchor("app.py", 50);
        let ends = vec![RawEnd {
            anchor: None,
            syntax_type: Some("function".into()),
        }];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Unresolved(UnresolvedReason::NoDeclaration)
        );
    }

    #[test]
    fn a_local_symbol_of_another_document_is_not_a_target() {
        let reference = anchor("app.py", 50);
        // `c.py:5` has a local symbol. app.py cannot use it.
        let ends = vec![end("c.py", 5, None)];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Unresolved(UnresolvedReason::NotAddressable)
        );
        // The same symbol in its own document is a valid target.
        let own = anchor("c.py", 40);
        assert_eq!(
            outcome(vec![binding(own, vec![end("c.py", 5, None)], None)]),
            BindingOutcome::Resolved(sym(9))
        );
    }
}
