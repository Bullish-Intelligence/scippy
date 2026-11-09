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

use std::collections::{BTreeMap, BTreeSet};

use crate::identity::SymbolId;

/// The classes of a project: their supertypes and their methods.
#[derive(Default)]
pub(crate) struct Inheritance {
    /// For each class, the symbols of its resolved supertypes in header order.
    supertypes: BTreeMap<SymbolId, Vec<SymbolId>>,
    /// For each class, its methods by name.
    methods: BTreeMap<SymbolId, BTreeMap<String, SymbolId>>,
}

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
        }
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
    fn a_supertype_outside_the_project_gives_nothing() {
        let mut i = Inheritance::default();
        class(&mut i, "Child", &["Exception"], &["run"]);
        assert!(overrides(&i).is_empty());
    }
}
