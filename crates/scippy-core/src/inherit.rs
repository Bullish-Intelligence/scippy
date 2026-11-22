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
//! The module also tells whether the candidates of an ambiguous reference form a true union:
//! members of classes that have no inheritance relation. See [`Inheritance::is_union`].

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

    /// True when the candidates of an ambiguous reference are the members of unrelated classes.
    ///
    /// A true union is a value with the type `A | B`, or a value that `isinstance(x, (A, B))`
    /// narrows. Each candidate then belongs to its own class, and the reference names the member
    /// of whichever class the value has. The conditions:
    /// - there are two candidates at least, and `UNION_CAP` at most;
    /// - every candidate is a member that a class of the project owns;
    /// - no owner is one class with another owner, or a subclass or superclass of it, directly
    ///   or through other classes. Such a set is an override chain or a shadowed member.
    ///
    /// A supertype that the project does not resolve gives no relation, so a union of two classes
    /// can hide a relation that the resolver does not know.
    pub fn is_union(&self, candidates: &[SymbolId]) -> bool {
        if candidates.len() < 2 || candidates.len() > UNION_CAP {
            return false;
        }
        let Some(owners) = candidates
            .iter()
            .map(|c| self.owners.get(c))
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        owners.iter().enumerate().all(|(i, a)| {
            owners[i + 1..]
                .iter()
                .all(|b| a != b && !self.ancestors(a).contains(b) && !self.ancestors(b).contains(a))
        })
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
    fn related_owners_are_no_union() {
        let mut i = Inheritance::default();
        class(&mut i, "Base", &[], &["run"]);
        class(&mut i, "Child", &["Base"], &["run"]);
        class(&mut i, "Grand", &["Child"], &["run"]);
        class(&mut i, "Other", &[], &["run"]);
        // Direct and indirect inheritance, in both orders.
        assert!(!i.is_union(&[member("Base", "run"), member("Child", "run")]));
        assert!(!i.is_union(&[member("Grand", "run"), member("Base", "run")]));
        assert!(!i.is_union(&[member("Child", "run"), member("Grand", "run")]));
        // One related pair spoils a set of three.
        assert!(!i.is_union(&[
            member("Other", "run"),
            member("Base", "run"),
            member("Grand", "run")
        ]));
        // An unrelated class joins the base in a union.
        assert!(i.is_union(&[member("Other", "run"), member("Base", "run")]));
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
        class(&mut i, "Z", &[], &["go"]);
        assert!(!i.is_union(&[member("X", "go"), member("Y", "go")]));
        assert!(i.is_union(&[member("X", "go"), member("Z", "go")]));
    }

    #[test]
    fn a_supertype_outside_the_project_gives_nothing() {
        let mut i = Inheritance::default();
        class(&mut i, "Child", &["Exception"], &["run"]);
        assert!(overrides(&i).is_empty());
    }
}
