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
//!      path makes the end a value-flow end. A module is not a value: a path that enters a
//!      module to reach a name in it does not count.
//!    - A member reference, such as `a.x`, must pass the definition of its object, `a`.
//!      The path to every end passes it. Only a definition that is itself an end of the
//!      reference counts: the end of a path that passes the field `x` is a value-flow end.
//!      The pack marks member references. The core never reads syntax.
//!    - A keyword argument name, such as `f(name=v)`, starts at the callee. The path to every end
//!      passes the definitions of the callee: a class, its `__init__`, a function. They are no
//!      value flow, so they never make an end a value-flow end. A definition that is itself an
//!      end counts, as for a member. The default value of the parameter is such an end.
//!      The pack marks keyword names with the `@keyword` capture.
//! 5. A definition in a project file wins over a definition in an external asset, such as the
//!    builtins. A project definition shadows the builtin of the same name.
//! 6. The ends that join to declarations give the symbols. One distinct symbol means Resolved.
//!    Two or more distinct symbols mean Ambiguous. No symbol means Unresolved.
//! 7. A reference never takes a document-local symbol of another document.
//!    A path that passes the definition node at the span of the reference itself is a value-flow
//!    path too. The assignment `x.a = x.b` defines the name `a`, and the value of `b` flows into
//!    that definition. A reference to `a` that the rules also let reach `b` drops the end `b`.
//! 8. A reference at the span of a declaration of the same document names that declaration.
//!    It is the same token. An attribute assignment target, such as `self.x = v`, is a
//!    declaration and a reference at once. The receiver can reach the field of a base class
//!    too, but the token declares the field of the class that holds the assignment.
//!    The definition occurrence stands for the reference. The outcome is the declared symbol.
//!
//! The language pack can refuse a declaration as the target of a reference. The refusal runs
//! after the join and before the nearest-binding rule. The reconciler drops each joined end
//! that the pack refuses. A reference whose every joined end is refused has no target.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::bindings::{Bound, Place, RawBinding, RawEnd};
use crate::extract::SourceAnchor;
use crate::identity::SymbolId;
use crate::language::{DeclKind, ProjectModel, ReferenceKind, Scope};

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
    /// The language pack refused every declaration that the ends joined.
    NotTargetable,
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

/// A declaration in the join table.
struct Entry {
    symbol: SymbolId,
    /// The enclosing declarations, from the outermost to the innermost.
    scopes: Vec<Scope>,
    declaration: Scope,
}

/// What the reconciler asks the language pack about a reference.
///
/// It holds the containers of one document. They give the declarations that enclose a reference.
pub(crate) struct Targeting<'a> {
    model: Option<&'a dyn ProjectModel>,
    /// Containers of the document with their extents and scopes, outermost first.
    containers: Vec<((u32, u32), Scope)>,
}

impl<'a> Targeting<'a> {
    /// Ask the model for each end. `containers` are the extents of the container declarations.
    pub fn new(model: &'a dyn ProjectModel, mut containers: Vec<((u32, u32), Scope)>) -> Self {
        containers.sort_by_key(|(extent, _)| (extent.0, std::cmp::Reverse(extent.1)));
        Self {
            model: Some(model),
            containers,
        }
    }

    /// Accept every end.
    #[cfg(test)]
    pub fn accept_all() -> Self {
        Self {
            model: None,
            containers: Vec::new(),
        }
    }

    /// The containers that enclose the span, from the outermost to the innermost.
    fn enclosing(&self, start: u32, end: u32) -> Vec<Scope> {
        self.containers
            .iter()
            .take_while(|(extent, _)| extent.0 <= start)
            .filter(|(extent, _)| end <= extent.1)
            .map(|(_, scope)| scope.clone())
            .collect()
    }
}

/// Declaration anchors with their symbols. It is the join table.
///
/// Project declarations and declarations of external assets live in separate tables.
#[derive(Default)]
pub(crate) struct DeclarationIndex {
    project: HashMap<SourceAnchor, Entry>,
    external: HashMap<SourceAnchor, Entry>,
}

impl DeclarationIndex {
    #[cfg(test)]
    #[cfg(test)]
    pub fn insert(&mut self, anchor: SourceAnchor, symbol: SymbolId) {
        self.insert_scoped(anchor, symbol, Vec::new(), test_scope());
    }

    /// Insert a project declaration with the scope chain that the pack sees.
    pub fn insert_scoped(
        &mut self,
        anchor: SourceAnchor,
        symbol: SymbolId,
        scopes: Vec<Scope>,
        declaration: Scope,
    ) {
        self.project.insert(
            anchor,
            Entry {
                symbol,
                scopes,
                declaration,
            },
        );
    }

    #[cfg(test)]
    pub fn insert_external(&mut self, anchor: SourceAnchor, symbol: SymbolId) {
        self.insert_external_scoped(anchor, symbol, Vec::new(), test_scope());
    }

    /// Insert an external declaration with the scope chain that the pack sees.
    pub fn insert_external_scoped(
        &mut self,
        anchor: SourceAnchor,
        symbol: SymbolId,
        scopes: Vec<Scope>,
        declaration: Scope,
    ) {
        self.external.insert(
            anchor,
            Entry {
                symbol,
                scopes,
                declaration,
            },
        );
    }

    fn entry(&self, place: &Place) -> Option<&Entry> {
        if place.external {
            self.external.get(&place.anchor)
        } else {
            self.project.get(&place.anchor)
        }
    }

    pub fn get(&self, place: &Place) -> Option<&SymbolId> {
        self.entry(place).map(|e| &e.symbol)
    }
}

#[cfg(test)]
fn test_scope() -> Scope {
    Scope {
        kind: crate::language::DeclKind::Variable,
        name: "x".to_string(),
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
    /// Joined ends that the language pack refused for the kind of the reference.
    pub refused: usize,
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
/// `keywords` holds the spans of the references that name a parameter of a callee, as the
/// `name` in `f(name=v)`. A path to such a name passes the definitions of the callee,
/// like the path to a member passes the definition of its object.
/// `targeting` asks the language pack whether a reference can name a declaration.
/// The result is sorted by span.
pub(crate) fn reconcile(
    declarations: &DeclarationIndex,
    bindings: Vec<RawBinding>,
    members: &HashSet<(u32, u32)>,
    keywords: &HashSet<(u32, u32)>,
    targeting: &Targeting,
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
            let span = (reference.start_byte, reference.end_byte);
            let is_member = members.contains(&span);
            let is_keyword = keywords.contains(&span);
            let kind = if is_member {
                ReferenceKind::Member
            } else if is_keyword {
                ReferenceKind::Keyword
            } else {
                ReferenceKind::Plain
            };
            // The path to the end of such a reference starts at a value: an object or a callee.
            let through_value = is_member || is_keyword;

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
            // The language pack can refuse a declaration for this kind of reference.
            let before = joined.len();
            if let Some(model) = targeting.model.filter(|_| !joined.is_empty()) {
                let from = targeting.enclosing(reference.start_byte, reference.end_byte);
                joined.retain(|(_, place, _)| {
                    declarations.entry(place).is_none_or(|e| {
                        model.accepts_target(kind, &from, &e.scopes, &e.declaration)
                    })
                });
            }
            let refused = before - joined.len();
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
                        // An assignment target is a definition node at the span of the
                        // reference itself. A path through it carries the assigned value on.
                        if through.anchor == reference {
                            return false;
                        }
                        declarations.entry(through).is_none_or(|e| {
                            // A path enters a module to reach a name in it. No value flows
                            // through a module.
                            e.declaration.kind == DeclKind::Module
                                || e.symbol == **symbol
                                || (through_value && !end_places.contains(through))
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

            let own = declarations.project.get(&reference).map(|e| &e.symbol);
            let outcome = if let Some(own) = own {
                BindingOutcome::Resolved(own.clone())
            } else if let Some(bound) = bound {
                BindingOutcome::Bounded(bound)
            } else if groups.is_empty() {
                BindingOutcome::Unresolved(UnresolvedReason::NoPath)
            } else if joined.is_empty() && refused > 0 {
                BindingOutcome::Unresolved(UnresolvedReason::NotTargetable)
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
                refused,
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
    use crate::language::ReferenceKind;
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
        reconcile(
            &index(),
            bindings,
            &HashSet::new(),
            &HashSet::new(),
            &Targeting::accept_all(),
        )
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
        let all = reconcile(
            &index(),
            vec![one, two],
            &HashSet::new(),
            &HashSet::new(),
            &Targeting::accept_all(),
        );
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
    fn a_keyword_name_passes_the_callee_and_stops_at_the_parameter() {
        // `f(x=1)`, with `def f(x=DEFAULT)`. `a.py:10` is `f`, `a.py:0` is the parameter `x`,
        // and `m.py:0` is `DEFAULT`. The path to each end passes `f`. The path to `DEFAULT`
        // also passes the parameter.
        let mut index = index();
        index.insert(anchor("m.py", 0), sym(2));
        let reference = anchor("app.py", 50);
        let ends = || {
            vec![
                end_through("a.py", 0, &[("a.py", 10)]),
                end_through("m.py", 0, &[("a.py", 10), ("a.py", 0)]),
            ]
        };
        let run = |keywords: &HashSet<(u32, u32)>| {
            reconcile(
                &index,
                vec![binding(reference.clone(), ends(), None)],
                &HashSet::new(),
                keywords,
                &Targeting::accept_all(),
            )
            .remove(0)
            .outcome
        };
        // A plain reference sees two definitions of other symbols on each path.
        assert_eq!(
            run(&HashSet::new()),
            BindingOutcome::Ambiguous(vec![sym(0), sym(2)])
        );
        // A keyword name passes the callee. Only the default value is a value-flow end.
        assert_eq!(
            run(&HashSet::from([(50, 53)])),
            BindingOutcome::Resolved(sym(0))
        );
    }

    #[test]
    fn a_path_through_the_own_definition_node_is_value_flow() {
        // `x.a = x.b`: the name `a` is a definition node that no declaration claims.
        // The value of `b` flows into it, so the end `b` is a value-flow end.
        let reference = anchor("app.py", 50);
        let ends = vec![
            end("a.py", 0, None),
            end_through("a.py", 10, &[("app.py", 50)]),
        ];
        assert_eq!(
            outcome(vec![binding(reference, ends, None)]),
            BindingOutcome::Resolved(sym(0))
        );
    }

    #[test]
    fn a_reference_at_a_declaration_names_that_declaration() {
        // `self.x = v`: the receiver reaches two fields, but the token declares one of them.
        let reference = anchor("a.py", 0);
        let ends = vec![end("a.py", 0, None), end("a.py", 10, None)];
        assert_eq!(
            outcome(vec![binding(reference.clone(), ends, None)]),
            BindingOutcome::Resolved(sym(0))
        );
        // A bound does not change it. The token still declares the entity.
        assert_eq!(
            outcome(vec![binding(reference, vec![], Some(Bound::Work))]),
            BindingOutcome::Resolved(sym(0))
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
        reconcile(
            index,
            bindings,
            members,
            &HashSet::new(),
            &Targeting::accept_all(),
        )
        .remove(0)
        .outcome
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
    fn a_module_on_the_path_is_not_a_value_flow() {
        use crate::language::DeclKind;
        let module = |name: &str| Scope {
            kind: DeclKind::Module,
            name: name.to_string(),
        };
        let mut index = DeclarationIndex::default();
        // `m.py:0` is a module. `a.py:0` and `a.py:10` are the two names that the path reaches.
        index.insert_scoped(anchor("m.py", 0), sym(2), vec![], module("m"));
        index.insert(anchor("a.py", 0), sym(0));
        index.insert(anchor("a.py", 10), sym(1));
        let reference = anchor("app.py", 50);
        // Both paths enter the module. The path to `a.py:10` also passes the name `a.py:0`.
        let ends = vec![
            end_through("a.py", 0, &[("m.py", 0)]),
            end_through("a.py", 10, &[("m.py", 0), ("a.py", 0)]),
        ];
        let reconciled = outcome_with(
            &index,
            vec![binding(reference, ends, None)],
            &HashSet::new(),
        );
        // The module does not make the first end indirect. The name does make the second one.
        assert_eq!(reconciled, BindingOutcome::Resolved(sym(0)));
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
            &HashSet::new(),
            &Targeting::accept_all(),
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

    /// A model that refuses every declaration with the name `field` for a plain reference.
    struct Refuser;

    impl ProjectModel for Refuser {
        fn resolver_globals(&self, _: &ProjectRelativePath) -> Vec<(String, String)> {
            Vec::new()
        }

        fn identity(
            &self,
            _: &ProjectRelativePath,
            _: &[Scope],
            _: &Scope,
            _: &[Scope],
        ) -> crate::language::Identity {
            unreachable!("the reconciler never asks for an identity")
        }

        fn accepts_target(
            &self,
            reference: ReferenceKind,
            _from: &[Scope],
            _scopes: &[Scope],
            declaration: &Scope,
        ) -> bool {
            !(reference == ReferenceKind::Plain && declaration.name == "field")
        }
    }

    fn refusing_index() -> DeclarationIndex {
        use crate::language::DeclKind;
        let scope = |name: &str| Scope {
            kind: DeclKind::Variable,
            name: name.to_string(),
        };
        let mut index = DeclarationIndex::default();
        index.insert_scoped(anchor("a.py", 0), sym(0), vec![], scope("field"));
        index.insert_scoped(anchor("a.py", 10), sym(1), vec![], scope("other"));
        index
    }

    fn refusing_outcome(ends: Vec<RawEnd>, members: &HashSet<(u32, u32)>) -> Reconciled {
        let containers = vec![(
            (0, 100),
            Scope {
                kind: crate::language::DeclKind::Function,
                name: "f".to_string(),
            },
        )];
        let targeting = Targeting::new(&Refuser, containers);
        let reference = anchor("app.py", 50);
        reconcile(
            &refusing_index(),
            vec![binding(reference, ends, None)],
            members,
            &HashSet::new(),
            &targeting,
        )
        .remove(0)
    }

    #[test]
    fn the_pack_can_drop_an_end_for_a_plain_reference() {
        let ends = vec![end("a.py", 0, None), end("a.py", 10, None)];
        let one = refusing_outcome(ends.clone(), &HashSet::new());
        assert_eq!(one.outcome, BindingOutcome::Resolved(sym(1)));
        assert_eq!(one.refused, 1);
        // A member reference is not refused. Both ends stay.
        let members = HashSet::from([(50, 53)]);
        let two = refusing_outcome(ends, &members);
        assert_eq!(two.outcome, BindingOutcome::Ambiguous(vec![sym(0), sym(1)]));
        assert_eq!(two.refused, 0);
    }

    #[test]
    fn a_reference_whose_every_end_is_refused_has_no_target() {
        let one = refusing_outcome(vec![end("a.py", 0, None)], &HashSet::new());
        assert_eq!(
            one.outcome,
            BindingOutcome::Unresolved(UnresolvedReason::NotTargetable)
        );
    }

    #[test]
    fn the_containers_of_a_reference_reach_the_pack() {
        let scope = |name: &str| Scope {
            kind: crate::language::DeclKind::Class,
            name: name.to_string(),
        };
        let targeting = Targeting::new(
            &Refuser,
            vec![
                ((20, 40), scope("inner")),
                ((0, 100), scope("outer")),
                ((60, 90), scope("other")),
            ],
        );
        let names = |start, end| -> Vec<String> {
            targeting
                .enclosing(start, end)
                .into_iter()
                .map(|s| s.name)
                .collect()
        };
        assert_eq!(names(25, 28), ["outer", "inner"]);
        assert_eq!(names(45, 48), ["outer"]);
        assert_eq!(names(70, 73), ["outer", "other"]);
        assert!(names(150, 153).is_empty());
    }
}
