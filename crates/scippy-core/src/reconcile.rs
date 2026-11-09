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
//! 4. A reference prefers the nearest binding. The engine follows values, so a name can reach
//!    its own binding and then the definition that the value comes from.
//!    A value-flow end is an end whose path passes the definition of another symbol.
//!    The reconciler drops value-flow ends when a nearest end exists.
//!    - A plain name has no reason to pass a definition. Any definition of another symbol on the
//!      path makes the end a value-flow end.
//!    - A member reference, such as `a.x`, must pass the definition of its object, `a`.
//!      The path to every end passes it. Only a definition that is itself an end of the
//!      reference counts: the end of a path that passes the field `x` is a value-flow end.
//!      The pack marks member references. The core never reads syntax.
//! 5. A definition in a project file wins over a definition in an external asset, such as the
//!    builtins. A project definition shadows the builtin of the same name.
//! 6. The ends that join to declarations give the symbols. One distinct symbol means Resolved.
//!    Two or more distinct symbols mean Ambiguous. No symbol means Unresolved.
//! 7. A reference never takes a document-local symbol of another document.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::bindings::{Bound, Place, RawBinding, RawEnd};
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
///
/// Project declarations and declarations of external assets live in separate tables.
#[derive(Default)]
pub(crate) struct DeclarationIndex {
    project: HashMap<SourceAnchor, SymbolId>,
    external: HashMap<SourceAnchor, SymbolId>,
}

impl DeclarationIndex {
    pub fn insert(&mut self, anchor: SourceAnchor, symbol: SymbolId) {
        self.project.insert(anchor, symbol);
    }

    pub fn insert_external(&mut self, anchor: SourceAnchor, symbol: SymbolId) {
        self.external.insert(anchor, symbol);
    }

    pub fn get(&self, place: &Place) -> Option<&SymbolId> {
        if place.external {
            self.external.get(&place.anchor)
        } else {
            self.project.get(&place.anchor)
        }
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
    /// Ends in an external asset, such as the builtins.
    pub outside: usize,
    /// Ends in the project that joined to a declaration, counted by anchor.
    pub joined: usize,
    /// Joined ends that the nearest-binding rule dropped.
    pub value_flow_dropped: usize,
    /// The ends that joined no declaration, as `syntax_type` or `-`.
    pub unjoined: Vec<String>,
    /// The joined declarations as `file:start`, for the trace. An external end has the prefix `~`.
    pub targets: Vec<String>,
}

/// The ends of one definition node. Paths with different routes reach one node.
struct EndGroup<'a> {
    place: Option<&'a Place>,
    syntax_type: &'a Option<String>,
    /// The `through` lists of the paths that reach the node.
    routes: Vec<&'a [Place]>,
}

fn group_ends(ends: &BTreeSet<RawEnd>) -> Vec<EndGroup<'_>> {
    let mut groups: Vec<EndGroup> = Vec::new();
    for end in ends {
        // The set is sorted, so the entries of one node are adjacent.
        match groups.last_mut() {
            Some(last)
                if last.place == end.place.as_ref() && last.syntax_type == &end.syntax_type =>
            {
                last.routes.push(&end.through);
            }
            _ => groups.push(EndGroup {
                place: end.place.as_ref(),
                syntax_type: &end.syntax_type,
                routes: vec![&end.through],
            }),
        }
    }
    groups
}

/// Merge the raw bindings of one document by reference span and reconcile each location.
///
/// `members` holds the spans of the references that name a member of a value.
/// The result is sorted by span.
pub(crate) fn reconcile(
    declarations: &DeclarationIndex,
    bindings: Vec<RawBinding>,
    members: &HashSet<(u32, u32)>,
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
            let groups = group_ends(&ends);
            // A node that is shadowed in one search and kept in another is not shadowed.
            let shadowed = group_ends(&shadowed)
                .iter()
                .filter(|s| {
                    !groups
                        .iter()
                        .any(|g| g.place == s.place && g.syntax_type == s.syntax_type)
                })
                .count();
            let is_member = members.contains(&(reference.start_byte, reference.end_byte));

            let mut unjoined = Vec::new();
            let mut joined: Vec<(&EndGroup, &Place, &SymbolId)> = Vec::new();
            for group in &groups {
                match group
                    .place
                    .and_then(|place| declarations.get(place).map(|symbol| (place, symbol)))
                {
                    Some((place, symbol)) => joined.push((group, place, symbol)),
                    None => unjoined.push(group.syntax_type.clone().unwrap_or_else(|| "-".into())),
                }
            }
            let outside = groups
                .iter()
                .filter(|g| g.place.is_some_and(|p| p.external))
                .count();
            let project_joined = joined
                .iter()
                .filter(|(_, place, _)| !place.external)
                .count();

            // The nearest binding. A route is direct when it passes no definition of another
            // symbol. A member reference counts only the definitions of its own ends.
            let end_places: HashSet<&Place> = joined.iter().map(|(_, place, _)| *place).collect();
            let direct = |(group, _, symbol): &(&EndGroup, &Place, &SymbolId)| {
                group.routes.iter().any(|route| {
                    route.iter().all(|through| {
                        declarations.get(through).is_none_or(|s| {
                            s == *symbol || (is_member && !end_places.contains(through))
                        })
                    })
                })
            };
            let nearest: Vec<_> = joined.iter().filter(|j| direct(j)).collect();
            let (kept, value_flow_dropped): (Vec<_>, usize) = if nearest.is_empty() {
                (joined.iter().collect(), 0)
            } else {
                let dropped = joined.len() - nearest.len();
                (nearest, dropped)
            };

            // A definition in the project shadows a definition of an external asset.
            let in_project = kept.iter().any(|(_, place, _)| !place.external);
            let mut usable: BTreeSet<SymbolId> = BTreeSet::new();
            let mut foreign = false;
            let mut targets = Vec::new();
            for (_, place, symbol) in kept.iter().filter(|(_, p, _)| p.external != in_project) {
                targets.push(format!(
                    "{}{}:{}",
                    if place.external { "~" } else { "" },
                    place.anchor.path.as_str(),
                    place.anchor.start_byte
                ));
                // A document-local symbol is valid only in the document that declares it.
                if symbol.is_local() && place.anchor.path != reference.path {
                    foreign = true;
                } else {
                    usable.insert((*symbol).clone());
                }
            }

            let outcome = if let Some(bound) = bound {
                BindingOutcome::Bounded(bound)
            } else if groups.is_empty() {
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
                ends: groups.len(),
                shadowed,
                outside,
                joined: project_joined,
                value_flow_dropped,
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

    fn place(path: &str, start: u32) -> Place {
        Place {
            anchor: anchor(path, start),
            external: false,
        }
    }

    fn end(path: &str, start: u32, syntax: Option<&str>) -> RawEnd {
        RawEnd {
            place: Some(place(path, start)),
            syntax_type: syntax.map(str::to_string),
            through: vec![],
        }
    }

    /// An end that the path reaches after it passes the given definitions.
    fn end_through(path: &str, start: u32, through: &[(&str, u32)]) -> RawEnd {
        RawEnd {
            through: through.iter().map(|(p, s)| place(p, *s)).collect(),
            ..end(path, start, None)
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
        reconcile(&index(), bindings, &HashSet::new())
            .remove(0)
            .outcome
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
        let all = reconcile(&index(), vec![one, two], &HashSet::new());
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
            place: None,
            syntax_type: Some("function".into()),
            through: vec![],
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

    fn external_end(start: u32) -> RawEnd {
        RawEnd {
            place: Some(Place {
                anchor: anchor("<builtins>", start),
                external: true,
            }),
            syntax_type: Some("function".into()),
            through: vec![],
        }
    }

    /// The index with two external declarations: `sym(5)` and `sym(6)`.
    fn external_index() -> DeclarationIndex {
        let mut index = index();
        index.insert_external(anchor("<builtins>", 0), sym(5));
        index.insert_external(anchor("<builtins>", 10), sym(6));
        index
    }

    fn outcome_with(
        index: &DeclarationIndex,
        bindings: Vec<RawBinding>,
        members: &HashSet<(u32, u32)>,
    ) -> BindingOutcome {
        reconcile(index, bindings, members).remove(0).outcome
    }

    #[test]
    fn a_plain_name_drops_an_end_that_passes_another_definition() {
        // `a.py:0` is the binding. The path to `a.py:10` passes it: a value flows on.
        let reference = anchor("app.py", 50);
        let ends = vec![
            end("a.py", 0, None),
            end_through("a.py", 10, &[("a.py", 0)]),
        ];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Resolved(sym(0))
        );
    }

    #[test]
    fn a_path_through_a_binding_of_the_same_symbol_is_direct() {
        // A rebinding passes its earlier site. Both sites are one symbol.
        let reference = anchor("app.py", 50);
        let ends = vec![
            end("a.py", 10, None),
            end_through("b.py", 0, &[("a.py", 0)]),
        ];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Ambiguous(vec![sym(0), sym(1)])
        );
    }

    #[test]
    fn an_unjoined_definition_on_the_path_does_not_count() {
        // An import alias is a definition that joins no declaration.
        let reference = anchor("app.py", 50);
        let ends = vec![end_through("a.py", 0, &[("app.py", 20)])];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Resolved(sym(0))
        );
    }

    #[test]
    fn when_every_end_passes_a_definition_every_end_stays() {
        let reference = anchor("app.py", 50);
        let ends = vec![
            end_through("a.py", 0, &[("a.py", 10)]),
            end_through("a.py", 10, &[("a.py", 0)]),
        ];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Ambiguous(vec![sym(0), sym(1)])
        );
    }

    #[test]
    fn two_direct_ends_stay_ambiguous() {
        let reference = anchor("app.py", 50);
        let ends = vec![end("a.py", 0, None), end("a.py", 10, None)];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Ambiguous(vec![sym(0), sym(1)])
        );
    }

    #[test]
    fn a_member_reference_passes_the_definition_of_its_object() {
        // `c.py:5` is the definition of the object. Every path passes it.
        let reference = anchor("app.py", 50);
        let members = HashSet::from([(50, 53)]);
        let object = ("c.py", 5);
        let ends = vec![
            end_through("a.py", 0, &[object]),
            end_through("a.py", 10, &[object]),
        ];
        let reconciled = outcome_with(
            &index(),
            vec![binding(reference.clone(), ends, None)],
            &members,
        );
        // The object is not an end, so no end is a value-flow end.
        assert_eq!(reconciled, BindingOutcome::Ambiguous(vec![sym(0), sym(1)]));
        // The end of a path that passes another end is a value-flow end.
        let ends = vec![
            end_through("a.py", 0, &[object]),
            end_through("a.py", 10, &[object, ("a.py", 0)]),
        ];
        let reconciled = outcome_with(&index(), vec![binding(reference, ends, None)], &members);
        assert_eq!(reconciled, BindingOutcome::Resolved(sym(0)));
    }

    #[test]
    fn an_end_in_an_external_asset_resolves_to_an_external_symbol() {
        let reference = anchor("app.py", 50);
        let index = external_index();
        let one = vec![binding(reference.clone(), vec![external_end(0)], None)];
        assert_eq!(
            outcome_with(&index, one, &HashSet::new()),
            BindingOutcome::Resolved(sym(5))
        );
        let two = vec![binding(
            reference,
            vec![external_end(0), external_end(10)],
            None,
        )];
        assert_eq!(
            outcome_with(&index, two, &HashSet::new()),
            BindingOutcome::Ambiguous(vec![sym(5), sym(6)])
        );
    }

    #[test]
    fn a_project_definition_shadows_an_external_one() {
        let reference = anchor("app.py", 50);
        let ends = vec![end("a.py", 0, None), external_end(0)];
        let all = reconcile(
            &external_index(),
            vec![binding(reference, ends, None)],
            &HashSet::new(),
        );
        assert_eq!(all[0].outcome, BindingOutcome::Resolved(sym(0)));
        assert_eq!(all[0].outside, 1);
    }

    #[test]
    fn an_external_end_without_a_declaration_is_unresolved() {
        let reference = anchor("app.py", 50);
        let ends = vec![external_end(99)];
        assert_eq!(
            outcome_with(
                &external_index(),
                vec![binding(reference, ends, None)],
                &HashSet::new()
            ),
            BindingOutcome::Unresolved(UnresolvedReason::NoDeclaration)
        );
    }

    #[test]
    fn an_external_span_never_joins_a_project_declaration() {
        // `a.py:0` is a project declaration. The same span in an asset is not.
        let reference = anchor("app.py", 50);
        let ends = vec![RawEnd {
            place: Some(Place {
                anchor: anchor("a.py", 0),
                external: true,
            }),
            syntax_type: None,
            through: vec![],
        }];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Unresolved(UnresolvedReason::NoDeclaration)
        );
    }
}
