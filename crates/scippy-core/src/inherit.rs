//! Override relationships.
//!
//! A pack marks the supertypes in the header of a class with `@supertype`. The reconciler
//! resolves each one to a class symbol. This module joins that inheritance data to the
//! methods of the classes.
//!
//! A method overrides a method of a supertype when both have one name. Look for the base method
//! along each supertype of the class on its own. On each branch, the nearest class that has a
//! method of that name provides the base method. A class with several supertypes can therefore
//! override one method on each branch. The module does not compute a method resolution order.
//! It reads no syntax. It never guesses: an unresolved supertype and a supertype outside the
//! project give no relationship.
//!
//! The module also tells whether the candidates of an ambiguous reference form a union: members of
//! distinct classes. See [`Inheritance::is_union`]. For an override chain of attributes, it picks
//! one candidate. See [`Inheritance::pick_attribute`].
//!
//! The module also chooses one target for a member reference by the method resolution order
//! (MRO) of the receiver class. See [`Inheritance::select`] for the combined policy.

use std::collections::{BTreeMap, BTreeSet};

use crate::identity::SymbolId;

/// The classes of a project: their supertypes and their methods.
#[derive(Default)]
pub(crate) struct Inheritance {
    /// For each class, the symbols of its resolved supertypes in header order.
    supertypes: BTreeMap<SymbolId, Vec<SymbolId>>,
    /// For each class, its methods by name.
    methods: BTreeMap<SymbolId, BTreeMap<String, SymbolId>>,
    /// The class that owns each member: a method, an attribute, or a property.
    owners: BTreeMap<SymbolId, SymbolId>,
    /// The symbols of all methods.
    method_symbols: BTreeSet<SymbolId>,
    /// The members that an annotation declares (`x: T` in a class body, `self.x: T = v`).
    annotated: BTreeSet<SymbolId>,
    /// For each class, its members by name. A name can have several symbols: a class variable
    /// and a field that a method stores, for example.
    named: BTreeMap<SymbolId, BTreeMap<String, BTreeSet<SymbolId>>>,
    /// The name of each member.
    names: BTreeMap<SymbolId, String>,
}

/// The most candidates that an ambiguous reference can have and still count as a union.
pub(crate) const UNION_CAP: usize = 4;

impl Inheritance {
    /// Record that `class` names `supertype` in its header.
    pub fn add_supertype(&mut self, class: &SymbolId, supertype: SymbolId) {
        let list = self.supertypes.entry(class.clone()).or_default();
        if !list.contains(&supertype) {
            list.push(supertype);
        }
    }

    /// Record a method that `class` declares. A symbol that the code declares twice is one method.
    pub fn add_method(&mut self, class: &SymbolId, name: &str, method: SymbolId) {
        self.method_symbols.insert(method.clone());
        self.methods
            .entry(class.clone())
            .or_default()
            .entry(name.to_string())
            .or_insert(method);
    }

    /// Record that `class` owns `member`: a method, an attribute, or a property.
    pub fn add_member(&mut self, class: &SymbolId, member: SymbolId) {
        self.owners.entry(member).or_insert_with(|| class.clone());
    }

    /// Record that an annotation declares `member`.
    pub fn add_annotated(&mut self, member: SymbolId) {
        self.annotated.insert(member);
    }

    /// Record that `class` owns the member `name`. `annotated` is true when an annotation
    /// declares the member.
    pub fn add_named_member(
        &mut self,
        class: &SymbolId,
        name: &str,
        member: SymbolId,
        annotated: bool,
    ) {
        self.names
            .entry(member.clone())
            .or_insert_with(|| name.to_string());
        self.named
            .entry(class.clone())
            .or_default()
            .entry(name.to_string())
            .or_default()
            .insert(member.clone());
        if annotated {
            self.add_annotated(member.clone());
        }
        self.add_member(class, member);
    }

    /// The one target of an attribute override chain, when there is one.
    ///
    /// Every candidate is an attribute, not a method. The owners are distinct classes, and each
    /// pair of them is related by inheritance. An annotated declaration wins over a plain store.
    /// Among the remaining candidates, the most derived owner wins. A type checker reads the same
    /// value: the declared type first, else the most derived store. Without a unique winner, the
    /// result is `None`.
    pub fn pick_attribute(&self, candidates: &[SymbolId]) -> Option<SymbolId> {
        if candidates.len() < 2 || candidates.iter().any(|c| self.method_symbols.contains(c)) {
            return None;
        }
        let owners = candidates
            .iter()
            .map(|c| self.owners.get(c))
            .collect::<Option<Vec<_>>>()?;
        let derives = |a: &SymbolId, b: &SymbolId| self.ancestors(a).contains(b);
        let is_chain = owners.iter().enumerate().all(|(i, a)| {
            owners[i + 1..]
                .iter()
                .all(|b| derives(a, b) || derives(b, a))
        });
        if !is_chain {
            return None;
        }
        let mut pool: Vec<usize> = (0..candidates.len())
            .filter(|&i| self.annotated.contains(&candidates[i]))
            .collect();
        if pool.is_empty() {
            pool = (0..candidates.len()).collect();
        }
        let mut winners = pool.iter().filter(|&&i| {
            pool.iter()
                .all(|&j| i == j || derives(owners[i], owners[j]))
        });
        match (winners.next(), winners.next()) {
            (Some(&one), None) => Some(candidates[one].clone()),
            _ => None,
        }
    }

    /// Every direct or indirect supertype of `class`. A cycle ends.
    fn ancestors(&self, class: &SymbolId) -> BTreeSet<&SymbolId> {
        let mut seen = BTreeSet::new();
        let mut todo: Vec<&SymbolId> = self.supertypes.get(class).into_iter().flatten().collect();
        while let Some(next) = todo.pop() {
            if seen.insert(next) {
                todo.extend(self.supertypes.get(next).into_iter().flatten());
            }
        }
        seen
    }

    /// The method resolution order of `class`: the C3 linearization of Python. The order starts
    /// with the class itself. A supertype that the project does not know is a class without
    /// supertypes. When the hierarchy is inconsistent or has a cycle, C3 fails. The order is then
    /// the depth-first, left-to-right order, with each class once.
    pub fn mro(&self, class: &SymbolId) -> Vec<SymbolId> {
        self.c3(class, &mut Vec::new()).unwrap_or_else(|| {
            let mut out = Vec::new();
            self.depth_first(class, &mut out);
            out
        })
    }

    fn depth_first(&self, class: &SymbolId, out: &mut Vec<SymbolId>) {
        if out.contains(class) {
            return;
        }
        out.push(class.clone());
        for supertype in self.supertypes.get(class).into_iter().flatten() {
            self.depth_first(supertype, out);
        }
    }

    /// The C3 linearization, or `None` for a cycle or an inconsistent hierarchy.
    fn c3(&self, class: &SymbolId, visiting: &mut Vec<SymbolId>) -> Option<Vec<SymbolId>> {
        if visiting.contains(class) {
            return None;
        }
        let bases: &[SymbolId] = self.supertypes.get(class).map_or(&[], |v| v.as_slice());
        visiting.push(class.clone());
        let mut lists: Vec<Vec<SymbolId>> = Vec::new();
        for base in bases {
            lists.push(self.c3(base, visiting)?);
        }
        visiting.pop();
        lists.push(bases.to_vec());
        let mut out = vec![class.clone()];
        loop {
            lists.retain(|l| !l.is_empty());
            if lists.is_empty() {
                return Some(out);
            }
            // The first head that no list holds in its tail.
            let head = lists
                .iter()
                .map(|l| &l[0])
                .find(|h| lists.iter().all(|l| !l[1..].contains(h)))?
                .clone();
            for l in &mut lists {
                if l[0] == head {
                    l.remove(0);
                }
            }
            out.push(head);
        }
    }

    /// Choose one candidate of a member reference. Return `None` when the choice is open, so the
    /// caller keeps the outcome: a union, or an ambiguous reference.
    ///
    /// The policy:
    /// - Known receiver. `enclosing` is the class of the own object of a method (`self`, `cls`,
    ///   `super()`), and every candidate belongs to it or to one of its ancestors. The MRO of
    ///   that class decides (see below).
    /// - Unknown receiver, several candidates. An attribute override chain gives one target
    ///   (`pick_attribute`): the annotated declaration first, else the most derived store.
    ///   Candidates of distinct classes, with a method among them or not, form a union
    ///   (`is_union`): the choice stays open. A method among several candidates always stays
    ///   open: the receiver can have the class of any override.
    /// - Unknown receiver, one owner. The receiver is the class that the owners of the
    ///   candidates lead to: the one class that has all owners in its MRO and no other such
    ///   class above it. Without such a class, the choice stays open.
    ///
    /// The MRO of the receiver decides in two steps:
    /// - Policy 1: the first class in the MRO that has a member of this name with an
    ///   annotation wins, whether or not it is a candidate.
    /// - Policy 2: the first class in the MRO that owns a candidate wins.
    ///
    /// A candidate that no project class owns must be external (`is_external`), such as a member
    /// of the builtin `object`. It comes after the classes of the project, so it is not chosen
    /// while a project candidate exists.
    pub fn select(
        &self,
        candidates: &[SymbolId],
        enclosing: Option<&SymbolId>,
        is_external: &dyn Fn(&SymbolId) -> bool,
    ) -> Option<SymbolId> {
        let mut owned: Vec<(&SymbolId, &SymbolId)> = Vec::new();
        for c in candidates {
            match self.owners.get(c) {
                Some(owner) => owned.push((c, owner)),
                None if is_external(c) => {}
                None => return None,
            }
        }
        let (first, _) = *owned.first()?;
        let name = self.names.get(first)?;
        if owned.iter().any(|(c, _)| self.names.get(*c) != Some(name)) {
            return None;
        }
        let owners: Vec<&SymbolId> = owned.iter().map(|(_, o)| *o).collect();
        let within = |class: &SymbolId, owner: &SymbolId| {
            class == owner || self.ancestors(class).contains(owner)
        };
        let receiver = match enclosing {
            Some(class) if owners.iter().all(|o| within(class, o)) => class.clone(),
            _ => {
                if let Some(pick) = self.pick_attribute(candidates) {
                    return Some(pick);
                }
                let owned_candidates: Vec<SymbolId> =
                    owned.iter().map(|(c, _)| (*c).clone()).collect();
                if self.is_union(&owned_candidates) {
                    return None;
                }
                if candidates.len() > 1
                    && candidates.iter().any(|c| self.method_symbols.contains(c))
                {
                    return None;
                }
                let known: BTreeSet<&SymbolId> =
                    self.supertypes.keys().chain(self.owners.values()).collect();
                let leads: Vec<&SymbolId> = known
                    .into_iter()
                    .filter(|k| owners.iter().all(|o| within(k, o)))
                    .collect();
                let roots: Vec<&&SymbolId> = leads
                    .iter()
                    .filter(|k| {
                        !leads
                            .iter()
                            .any(|j| j != *k && self.ancestors(k).contains(j))
                    })
                    .collect();
                match roots.as_slice() {
                    [one] => (**one).clone(),
                    _ => return None,
                }
            }
        };
        let order = self.mro(&receiver);
        // Policy 1: an annotated declaration of the member.
        for class in &order {
            let Some(symbols) = self.named.get(class).and_then(|m| m.get(name)) else {
                continue;
            };
            let mut annotated = symbols.iter().filter(|s| self.annotated.contains(*s));
            if let Some(first) = annotated.next() {
                let own = candidates
                    .iter()
                    .find(|c| symbols.contains(*c) && self.annotated.contains(*c));
                return Some(own.unwrap_or(first).clone());
            }
        }
        // Policy 2: the most derived class that owns a candidate.
        for class in &order {
            let mut here = owned.iter().filter(|(_, o)| *o == class);
            if let Some((c, _)) = here.next() {
                return if here.next().is_none() {
                    Some((*c).clone())
                } else {
                    None
                };
            }
        }
        None
    }

    /// True when the candidates of an ambiguous reference are members of distinct classes.
    ///
    /// A union is a value with the type `A | B`, or a value that `isinstance(x, (A, B))`
    /// narrows. Each candidate then belongs to its own class, and the reference names the member
    /// of whichever class the value has. The owners can be related: a base declaration and its
    /// overrides form an override chain, and the receiver can have any class of the chain.
    /// The conditions:
    /// - there are two candidates at least, and `UNION_CAP` at most;
    /// - every candidate is a member that a class of the project owns;
    /// - no two candidates have one owner. Such a set is a shadowed member;
    /// - when two owners are related, the set is an override chain from one base: every
    ///   candidate is a method of one name, and one owner is a base of all the other owners.
    ///   The oracle gate (`scripts/oracle/owners.py`) applies the same rule. Related attributes
    ///   are no union: an attribute chain has one target (`pick_attribute`), or none.
    pub fn is_union(&self, candidates: &[SymbolId]) -> bool {
        if candidates.len() < 2 || candidates.len() > UNION_CAP {
            return false;
        }
        let Some(owners) = candidates
            .iter()
            .map(|c| self.owners.get(c))
            .collect::<Option<BTreeSet<_>>>()
        else {
            return false;
        };
        if owners.len() != candidates.len() {
            return false;
        }
        let related = |a: &SymbolId, b: &SymbolId| self.ancestors(a).contains(b);
        let any_related = owners.iter().any(|a| owners.iter().any(|b| related(a, b)));
        if !any_related {
            return true;
        }
        let names: BTreeSet<Option<&String>> =
            candidates.iter().map(|c| self.names.get(c)).collect();
        candidates.iter().all(|c| self.method_symbols.contains(c))
            && names.len() == 1
            && owners
                .iter()
                .any(|top| owners.iter().all(|o| o == top || related(o, top)))
    }

    /// The base methods that each method overrides, by method symbol.
    pub fn overrides(&self) -> BTreeMap<SymbolId, BTreeSet<SymbolId>> {
        let mut out: BTreeMap<SymbolId, BTreeSet<SymbolId>> = BTreeMap::new();
        for (class, methods) in &self.methods {
            let Some(supertypes) = self.supertypes.get(class) else {
                continue;
            };
            for (name, method) in methods {
                let mut found = BTreeSet::new();
                // A class can inherit one class on two branches. Visit it once.
                let mut seen = BTreeSet::from([class]);
                for supertype in supertypes {
                    self.nearest(supertype, name, &mut seen, &mut found);
                }
                found.remove(method);
                if !found.is_empty() {
                    out.entry(method.clone()).or_default().extend(found);
                }
            }
        }
        out
    }

    /// Find the nearest method of one name on one branch of the supertypes.
    fn nearest<'a>(
        &'a self,
        class: &'a SymbolId,
        name: &str,
        seen: &mut BTreeSet<&'a SymbolId>,
        found: &mut BTreeSet<SymbolId>,
    ) {
        if !seen.insert(class) {
            return;
        }
        if let Some(method) = self.methods.get(class).and_then(|m| m.get(name)) {
            found.insert(method.clone());
            return;
        }
        for supertype in self.supertypes.get(class).into_iter().flatten() {
            self.nearest(supertype, name, seen, found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{Descriptor, IdentityBuilder, Package};

    fn sym(name: &str) -> SymbolId {
        IdentityBuilder::new(
            "scippy",
            Package {
                manager: "test".into(),
                name: "p".into(),
                version: String::new(),
            },
        )
        .symbol(&[Descriptor::Term(name.to_string())])
        .unwrap()
    }

    /// A class `name` with the methods `names`. Its method symbol is `name.method`.
    fn class(inheritance: &mut Inheritance, name: &str, supertypes: &[&str], methods: &[&str]) {
        for supertype in supertypes {
            inheritance.add_supertype(&sym(name), sym(supertype));
        }
        for method in methods {
            inheritance.add_method(&sym(name), method, sym(&format!("{name}.{method}")));
            inheritance.add_member(&sym(name), sym(&format!("{name}.{method}")));
        }
    }

    /// The member `name.member` of the class `name`. A union tests members, so the class needs
    /// no method of that name.
    fn member(name: &str, member: &str) -> SymbolId {
        sym(&format!("{name}.{member}"))
    }

    fn overrides(inheritance: &Inheritance) -> Vec<(String, Vec<String>)> {
        inheritance
            .overrides()
            .into_iter()
            .map(|(method, bases)| {
                let name = |s: &SymbolId| {
                    let last = s.as_str().rsplit(' ').next().unwrap();
                    last.trim_end_matches('.').trim_matches('`').to_string()
                };
                (name(&method), bases.iter().map(name).collect())
            })
            .collect()
    }

    #[test]
    fn a_method_overrides_the_method_of_its_base() {
        let mut i = Inheritance::default();
        class(&mut i, "Base", &[], &["run", "stop"]);
        class(&mut i, "Child", &["Base"], &["run", "extra"]);
        assert_eq!(
            overrides(&i),
            [("Child.run".to_string(), vec!["Base.run".to_string()])]
        );
    }

    #[test]
    fn the_nearest_class_that_has_the_method_wins() {
        let mut i = Inheritance::default();
        class(&mut i, "A", &[], &["run"]);
        class(&mut i, "B", &["A"], &["run"]);
        // `C` has no `run`. `D` overrides the `run` of `B`, not the `run` of `A`.
        class(&mut i, "C", &["B"], &[]);
        class(&mut i, "D", &["C"], &["run"]);
        assert_eq!(
            overrides(&i),
            [
                ("B.run".to_string(), vec!["A.run".to_string()]),
                ("D.run".to_string(), vec!["B.run".to_string()]),
            ]
        );
    }

    #[test]
    fn each_branch_of_several_supertypes_gives_a_base_method() {
        let mut i = Inheritance::default();
        class(&mut i, "Left", &[], &["run"]);
        class(&mut i, "Right", &[], &["run"]);
        class(&mut i, "Both", &["Left", "Right"], &["run"]);
        assert_eq!(
            overrides(&i),
            [(
                "Both.run".to_string(),
                vec!["Left.run".to_string(), "Right.run".to_string()]
            )]
        );
    }

    #[test]
    fn a_diamond_and_a_cycle_end() {
        let mut i = Inheritance::default();
        class(&mut i, "Top", &[], &["run"]);
        class(&mut i, "L", &["Top"], &[]);
        class(&mut i, "R", &["Top"], &[]);
        class(&mut i, "Bottom", &["L", "R"], &["run"]);
        assert_eq!(
            overrides(&i),
            [("Bottom.run".to_string(), vec!["Top.run".to_string()])]
        );
        // A cycle is invalid code. The run must still end.
        class(&mut i, "X", &["Y"], &["go"]);
        class(&mut i, "Y", &["X"], &["go"]);
        assert_eq!(overrides(&i).len(), 3);
    }

    #[test]
    fn members_of_unrelated_classes_are_a_union() {
        let mut i = Inheritance::default();
        class(&mut i, "ListExpr", &[], &["items"]);
        class(&mut i, "TupleExpr", &[], &["items"]);
        let both = [member("ListExpr", "items"), member("TupleExpr", "items")];
        assert!(i.is_union(&both));
        // The order of the candidates does not matter.
        assert!(i.is_union(&[both[1].clone(), both[0].clone()]));
    }

    #[test]
    fn related_owners_are_an_override_chain_and_form_a_union() {
        let mut i = Inheritance::default();
        class(&mut i, "Base", &[], &["run"]);
        class(&mut i, "Child", &["Base"], &["run"]);
        class(&mut i, "Grand", &["Child"], &["run"]);
        class(&mut i, "Other", &[], &["run"]);
        // Direct and indirect inheritance, in both orders.
        assert!(i.is_union(&[member("Base", "run"), member("Child", "run")]));
        assert!(i.is_union(&[member("Grand", "run"), member("Base", "run")]));
        assert!(i.is_union(&[
            member("Base", "run"),
            member("Child", "run"),
            member("Grand", "run")
        ]));
        // An unrelated class joins the chain: no owner is a base of all the others.
        assert!(!i.is_union(&[
            member("Other", "run"),
            member("Base", "run"),
            member("Grand", "run")
        ]));
        // Two subclasses of one base form a chain from that base. A sibling pair alone is
        // unrelated, and it is a union.
        class(&mut i, "Left", &["Base"], &["run"]);
        class(&mut i, "Right", &["Base"], &["run"]);
        assert!(i.is_union(&[
            member("Base", "run"),
            member("Left", "run"),
            member("Right", "run")
        ]));
        assert!(i.is_union(&[member("Left", "run"), member("Right", "run")]));
        // Related attributes are no union.
        let mut a = Inheritance::default();
        attrs(&mut a, "Node", &[], &["parent"]);
        attrs(&mut a, "Attr", &["Node"], &["parent"]);
        assert!(!a.is_union(&[member("Node", "parent"), member("Attr", "parent")]));
        // One class never joins itself.
        assert!(!i.is_union(&[member("Base", "run"), member("Base", "run")]));
    }

    /// A class with attributes (members that are no methods).
    fn attrs(inheritance: &mut Inheritance, name: &str, supertypes: &[&str], attributes: &[&str]) {
        for supertype in supertypes {
            inheritance.add_supertype(&sym(name), sym(supertype));
        }
        for a in attributes {
            inheritance.add_member(&sym(name), sym(&format!("{name}.{a}")));
        }
    }

    #[test]
    fn an_attribute_chain_picks_the_annotated_declaration_else_the_most_derived() {
        let mut i = Inheritance::default();
        attrs(&mut i, "Base", &[], &["t"]);
        attrs(&mut i, "Mid", &["Base"], &["t"]);
        attrs(&mut i, "Leaf", &["Mid"], &["t"]);
        attrs(&mut i, "Other", &[], &["t"]);
        let all = [member("Base", "t"), member("Mid", "t"), member("Leaf", "t")];
        // No annotation: the most derived store wins.
        assert_eq!(i.pick_attribute(&all), Some(member("Leaf", "t")));
        // An annotation in the base wins over the stores of the subclasses.
        i.add_annotated(member("Base", "t"));
        assert_eq!(i.pick_attribute(&all), Some(member("Base", "t")));
        // Two annotations: the most derived annotated one wins.
        i.add_annotated(member("Mid", "t"));
        assert_eq!(i.pick_attribute(&all), Some(member("Mid", "t")));
        // An unrelated owner makes the set a union, not a chain.
        assert_eq!(
            i.pick_attribute(&[member("Base", "t"), member("Other", "t")]),
            None
        );
        // A method chain gives no pick.
        let mut m = Inheritance::default();
        class(&mut m, "Base", &[], &["run"]);
        class(&mut m, "Child", &["Base"], &["run"]);
        assert_eq!(
            m.pick_attribute(&[member("Base", "run"), member("Child", "run")]),
            None
        );
    }

    #[test]
    fn two_members_of_one_class_are_no_union() {
        let mut i = Inheritance::default();
        class(&mut i, "A", &[], &["x", "y"]);
        assert!(!i.is_union(&[member("A", "x"), member("A", "y")]));
    }

    #[test]
    fn a_candidate_that_no_class_owns_is_no_union() {
        let mut i = Inheritance::default();
        class(&mut i, "A", &[], &["run"]);
        // `loose` is a symbol that no class owns: a module-level name, or an external member.
        assert!(!i.is_union(&[member("A", "run"), sym("loose")]));
        assert!(!i.is_union(&[member("A", "run")]));
        assert!(!i.is_union(&[]));
    }

    #[test]
    fn a_set_above_the_cap_is_no_union() {
        let mut i = Inheritance::default();
        let names = ["A", "B", "C", "D", "E"];
        for name in names {
            class(&mut i, name, &[], &["run"]);
        }
        let set =
            |n: usize| -> Vec<SymbolId> { names[..n].iter().map(|c| member(c, "run")).collect() };
        assert!(i.is_union(&set(UNION_CAP)));
        assert!(!i.is_union(&set(UNION_CAP + 1)));
    }

    #[test]
    fn a_cycle_of_supertypes_ends() {
        let mut i = Inheritance::default();
        class(&mut i, "X", &["Y"], &["go"]);
        class(&mut i, "Y", &["X"], &["go"]);
        assert_eq!(i.ancestors(&sym("X")).len(), 2);
    }

    /// A member `name.m` of the class `name`, with a type annotation when `annotated` is true.
    fn named(
        i: &mut Inheritance,
        class_name: &str,
        member_name: &str,
        annotated: bool,
    ) -> SymbolId {
        let symbol = member(class_name, member_name);
        i.add_named_member(&sym(class_name), member_name, symbol.clone(), annotated);
        symbol
    }

    fn none_external(_: &SymbolId) -> bool {
        false
    }

    fn mro_names(i: &Inheritance, class: &str) -> Vec<String> {
        i.mro(&sym(class))
            .iter()
            .map(|s| {
                s.as_str()
                    .rsplit(' ')
                    .next()
                    .unwrap()
                    .trim_end_matches('.')
                    .to_string()
            })
            .collect()
    }

    /// A diamond: `D(B, C)`, `B(A)`, `C(A)`.
    fn diamond() -> Inheritance {
        let mut i = Inheritance::default();
        class(&mut i, "A", &[], &[]);
        class(&mut i, "B", &["A"], &[]);
        class(&mut i, "C", &["A"], &[]);
        class(&mut i, "D", &["B", "C"], &[]);
        i
    }

    #[test]
    fn the_mro_of_a_diamond_is_c3() {
        let i = diamond();
        // Depth first would give D B A C.
        assert_eq!(mro_names(&i, "D"), ["D", "B", "C", "A"]);
    }

    #[test]
    fn a_diamond_picks_the_class_before_the_shared_base() {
        let mut i = diamond();
        let a = named(&mut i, "A", "m", false);
        let c = named(&mut i, "C", "m", false);
        // `d.m` has the candidates `A.m` and `C.m`. Python reads `C` first.
        assert_eq!(
            i.select(&[a.clone(), c.clone()], None, &none_external),
            Some(c.clone())
        );
        assert_eq!(
            i.select(&[c.clone(), a], Some(&sym("D")), &none_external),
            Some(c)
        );
    }

    #[test]
    fn a_member_of_the_second_base_is_found() {
        // The argparse shape: the first base has no member, the second has it.
        let mut i = Inheritance::default();
        class(&mut i, "H", &[], &[]);
        class(&mut i, "C", &[], &[]);
        class(&mut i, "P", &["H", "C"], &[]);
        let m = named(&mut i, "C", "m", false);
        assert_eq!(
            i.select(std::slice::from_ref(&m), Some(&sym("P")), &none_external),
            Some(m)
        );
    }

    #[test]
    fn object_comes_last() {
        let mut i = Inheritance::default();
        class(&mut i, "H", &["object"], &[]);
        class(&mut i, "C", &["object"], &[]);
        class(&mut i, "P", &["H", "C"], &[]);
        assert_eq!(mro_names(&i, "P"), ["P", "H", "C", "object"]);
        let m = named(&mut i, "C", "__init__", false);
        let object_init = sym("object.__init__");
        let external = |s: &SymbolId| *s == sym("object.__init__");
        // `object.__init__` has no project owner. It is external.
        assert_eq!(
            i.select(&[object_init, m.clone()], Some(&sym("P")), &external),
            Some(m)
        );
    }

    #[test]
    fn an_annotated_base_wins_over_a_derived_store() {
        let mut i = Inheritance::default();
        class(&mut i, "A", &[], &[]);
        class(&mut i, "B", &["A"], &[]);
        let base = named(&mut i, "A", "x", true);
        let store = named(&mut i, "B", "x", false);
        // The reference finds the store only: the store of `B` shadows `A`.
        assert_eq!(
            i.select(
                std::slice::from_ref(&store),
                Some(&sym("B")),
                &none_external
            ),
            Some(base.clone())
        );
        assert_eq!(
            i.select(&[store, base.clone()], None, &none_external),
            Some(base)
        );
    }

    #[test]
    fn an_annotated_derived_store_wins_over_a_plain_base() {
        let mut i = Inheritance::default();
        class(&mut i, "A", &[], &[]);
        class(&mut i, "B", &["A"], &[]);
        let base = named(&mut i, "A", "x", false);
        let store = named(&mut i, "B", "x", true);
        assert_eq!(
            i.select(&[base, store.clone()], None, &none_external),
            Some(store)
        );
    }

    #[test]
    fn the_most_derived_candidate_wins_without_a_receiver() {
        let mut i = Inheritance::default();
        class(&mut i, "A", &[], &[]);
        class(&mut i, "B", &["A"], &[]);
        class(&mut i, "C", &["B"], &[]);
        let a = named(&mut i, "A", "x", false);
        let c = named(&mut i, "C", "x", false);
        assert_eq!(i.select(&[a, c.clone()], None, &none_external), Some(c));
    }

    #[test]
    fn an_inconsistent_hierarchy_falls_back_to_depth_first() {
        let mut i = Inheritance::default();
        class(&mut i, "X", &[], &[]);
        class(&mut i, "Y", &["X"], &[]);
        // `Z(X, Y)` has no C3 order: `X` stands before its subclass `Y`.
        class(&mut i, "Z", &["X", "Y"], &[]);
        assert_eq!(mro_names(&i, "Z"), ["Z", "X", "Y"]);
        let y = named(&mut i, "Y", "m", false);
        assert_eq!(
            i.select(std::slice::from_ref(&y), Some(&sym("Z")), &none_external),
            Some(y)
        );
        // A cycle ends too.
        class(&mut i, "P", &["Q"], &[]);
        class(&mut i, "Q", &["P"], &[]);
        assert_eq!(mro_names(&i, "P"), ["P", "Q"]);
    }

    /// A method `name.member` of the class `name`, as a method and as a named member.
    fn method_of(i: &mut Inheritance, class_name: &str, member_name: &str) -> SymbolId {
        let symbol = member(class_name, member_name);
        i.add_method(&sym(class_name), member_name, symbol.clone());
        i.add_named_member(&sym(class_name), member_name, symbol.clone(), false);
        symbol
    }

    #[test]
    fn a_known_receiver_picks_one_method_by_the_mro() {
        // `self.m` in `D(B, C)`: the rules return `A.m` and `C.m`. Python reads `C` first.
        let mut i = diamond();
        let a = method_of(&mut i, "A", "m");
        let c = method_of(&mut i, "C", "m");
        assert_eq!(
            i.select(&[a, c.clone()], Some(&sym("D")), &none_external),
            Some(c)
        );
    }

    #[test]
    fn a_receiver_typed_as_a_base_with_overrides_is_a_union() {
        // `r: Base` then `r.run()`: the receiver can have the class of any override. No pick.
        let mut i = Inheritance::default();
        class(&mut i, "Base", &[], &[]);
        class(&mut i, "Child", &["Base"], &[]);
        class(&mut i, "Grand", &["Child"], &[]);
        let all = [
            method_of(&mut i, "Base", "run"),
            method_of(&mut i, "Child", "run"),
            method_of(&mut i, "Grand", "run"),
        ];
        assert!(i.is_union(&all));
        assert_eq!(i.select(&all, None, &none_external), None);
        // A method chain above the cap stays open too: no most derived pick.
        let names = ["A", "B", "C", "D", "E"];
        let mut j = Inheritance::default();
        class(&mut j, "A", &[], &[]);
        for pair in names.windows(2) {
            class(&mut j, pair[1], &[pair[0]], &[]);
        }
        let chain: Vec<SymbolId> = names.iter().map(|n| method_of(&mut j, n, "run")).collect();
        assert!(!j.is_union(&chain));
        assert_eq!(j.select(&chain, None, &none_external), None);
        // The own object of a method of the last class is a known receiver: the MRO decides.
        assert_eq!(
            j.select(&chain, Some(&sym("E")), &none_external),
            Some(member("E", "run"))
        );
    }

    #[test]
    fn an_attribute_chain_with_an_annotated_base_gives_the_annotated_declaration() {
        let mut i = Inheritance::default();
        class(&mut i, "Base", &[], &[]);
        class(&mut i, "Leaf", &["Base"], &[]);
        let base = named(&mut i, "Base", "t", true);
        let store = named(&mut i, "Leaf", "t", false);
        assert_eq!(
            i.select(&[store.clone(), base.clone()], None, &none_external),
            Some(base.clone())
        );
        // Without an annotation, the most derived store wins.
        let mut j = Inheritance::default();
        class(&mut j, "Base", &[], &[]);
        class(&mut j, "Leaf", &["Base"], &[]);
        let plain_base = named(&mut j, "Base", "t", false);
        let plain_leaf = named(&mut j, "Leaf", "t", false);
        assert_eq!(
            j.select(&[plain_base, plain_leaf.clone()], None, &none_external),
            Some(plain_leaf)
        );
    }

    #[test]
    fn a_base_class_name_in_a_subclass_body_gives_the_base_member() {
        // The scrapy shape: `attributes = Request.attributes + (...)` in `JsonRequest`. The
        // reference has no own object, so only the member of `Request` is a candidate.
        let mut i = Inheritance::default();
        class(&mut i, "Request", &[], &[]);
        class(&mut i, "JsonRequest", &["Request"], &[]);
        let base = named(&mut i, "Request", "attributes", true);
        let sub = named(&mut i, "JsonRequest", "attributes", true);
        assert_eq!(
            i.select(std::slice::from_ref(&base), None, &none_external),
            Some(base.clone())
        );
        // `self.attributes` in a method of `JsonRequest` gives the member of the subclass.
        assert_eq!(
            i.select(
                &[base, sub.clone()],
                Some(&sym("JsonRequest")),
                &none_external
            ),
            Some(sub)
        );
    }

    #[test]
    fn unrelated_owners_without_a_receiver_stay_open() {
        let mut i = Inheritance::default();
        class(&mut i, "ListExpr", &[], &[]);
        class(&mut i, "TupleExpr", &[], &[]);
        let a = named(&mut i, "ListExpr", "items", false);
        let b = named(&mut i, "TupleExpr", "items", false);
        assert!(i.is_union(&[a.clone(), b.clone()]));
        assert_eq!(i.select(&[a, b], None, &none_external), None);
    }

    #[test]
    fn a_candidate_without_an_owner_keeps_the_choice_open() {
        let mut i = Inheritance::default();
        class(&mut i, "A", &[], &[]);
        let a = named(&mut i, "A", "x", false);
        assert_eq!(i.select(&[a, sym("loose")], None, &none_external), None);
    }

    #[test]
    fn a_supertype_outside_the_project_gives_nothing() {
        let mut i = Inheritance::default();
        class(&mut i, "Child", &["Exception"], &["run"]);
        assert!(overrides(&i).is_empty());
    }
}
