//! Build SCIP symbol identities.
//!
//! The builder formats a symbol with `scip::symbol`, then parses it back.
//! It rejects the symbol unless the parsed parts equal the input parts.
//! An identity holds only names, never arena indexes, traversal order, or hash seeds.

use std::collections::BTreeMap;

use scip::symbol::{format_symbol, parse_symbol};
use scip::types::{
    Descriptor as ScipDescriptor, Package as ScipPackage, Symbol, descriptor::Suffix,
};

use crate::error::ScippyError;
use crate::extract::SourceAnchor;

/// A valid SCIP symbol string. Only this module creates one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId(String);

impl SymbolId {
    /// A symbol that is visible in one document only.
    pub fn local(id: u32) -> Self {
        Self(format_symbol(Symbol::new_local(id as usize)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_local(&self) -> bool {
        scip::symbol::is_local_symbol(&self.0)
    }
}

/// The package part of a global symbol. An empty field means "not set".
#[derive(Clone, Debug)]
pub struct Package {
    pub manager: String,
    pub name: String,
    pub version: String,
}

/// One descriptor of a global symbol, from the outer scope to the inner scope.
#[derive(Clone, Debug)]
pub enum Descriptor {
    Namespace(String),
    Type(String),
    Term(String),
    Method { name: String, disambiguator: String },
}

impl Descriptor {
    fn to_scip(&self) -> ScipDescriptor {
        let (name, suffix, disambiguator) = match self {
            Self::Namespace(n) => (n, Suffix::Namespace, ""),
            Self::Type(n) => (n, Suffix::Type, ""),
            Self::Term(n) => (n, Suffix::Term, ""),
            Self::Method {
                name,
                disambiguator,
            } => (name, Suffix::Method, disambiguator.as_str()),
        };
        ScipDescriptor {
            name: name.clone(),
            disambiguator: disambiguator.to_string(),
            suffix: suffix.into(),
            ..Default::default()
        }
    }
}

/// Build global symbols for one scheme and one package.
///
/// The builder also records which declaration owns each symbol it defines.
pub struct IdentityBuilder {
    scheme: String,
    package: Package,
    owners: BTreeMap<SymbolId, SourceAnchor>,
}

impl IdentityBuilder {
    pub fn new(scheme: &str, package: Package) -> Self {
        Self {
            scheme: scheme.to_string(),
            package,
            owners: BTreeMap::new(),
        }
    }

    /// Build the symbol for the given descriptors.
    pub fn symbol(&self, descriptors: &[Descriptor]) -> Result<SymbolId, ScippyError> {
        let fail = |symbol: &str, reason: &str| ScippyError::InvalidSymbol {
            symbol: symbol.to_string(),
            reason: reason.to_string(),
        };
        if descriptors.is_empty() {
            return Err(fail("", "a global symbol needs at least one descriptor"));
        }
        if self.scheme.is_empty() || self.scheme == "local" {
            return Err(fail(&self.scheme, "the scheme is empty or reserved"));
        }

        // `format_symbol` escapes descriptor names but not the package fields.
        // SCIP escapes a space in those fields as two spaces.
        let escape = |s: &str| s.replace(' ', "  ");
        let formatted = format_symbol(Symbol {
            scheme: escape(&self.scheme),
            package: Some(ScipPackage {
                manager: escape(&self.package.manager),
                name: escape(&self.package.name),
                version: escape(&self.package.version),
                ..Default::default()
            })
            .into(),
            descriptors: descriptors.iter().map(Descriptor::to_scip).collect(),
            ..Default::default()
        });

        let parsed = parse_symbol(&formatted).map_err(|e| fail(&formatted, &format!("{e:?}")))?;
        let (manager, name, version) = parsed.package.as_ref().map_or(("", "", ""), |p| {
            (p.manager.as_str(), p.name.as_str(), p.version.as_str())
        });
        let same_package = (manager, name, version)
            == (
                self.package.manager.as_str(),
                self.package.name.as_str(),
                self.package.version.as_str(),
            );
        let same_descriptors = parsed.descriptors.len() == descriptors.len()
            && parsed
                .descriptors
                .iter()
                .zip(descriptors)
                .all(|(parsed, wanted)| {
                    let wanted = wanted.to_scip();
                    // SCIP parses `name/` as a package descriptor, whatever the writer meant.
                    let wanted_suffix = match wanted.suffix.enum_value() {
                        Ok(Suffix::Namespace) => Ok(Suffix::Package),
                        other => other,
                    };
                    parsed.name == wanted.name
                        && parsed.disambiguator == wanted.disambiguator
                        && parsed.suffix.enum_value() == wanted_suffix
                });
        if parsed.scheme != self.scheme || !same_package || !same_descriptors {
            return Err(fail(
                &formatted,
                "the symbol does not parse back to its parts",
            ));
        }
        Ok(SymbolId(formatted))
    }

    /// Build the symbol and record `owner` as its declaration.
    ///
    /// Defining one symbol for the same declaration twice returns the same symbol.
    /// Defining it for a different declaration is a collision.
    pub fn define(
        &mut self,
        descriptors: &[Descriptor],
        owner: SourceAnchor,
    ) -> Result<SymbolId, ScippyError> {
        let symbol = self.symbol(descriptors)?;
        match self.owners.get(&symbol) {
            Some(first) if *first != owner => Err(ScippyError::SymbolCollision {
                symbol: symbol.0,
                detail: format!(
                    "declared at {}:{}..{} and at {}:{}..{}",
                    first.path.as_str(),
                    first.start_byte,
                    first.end_byte,
                    owner.path.as_str(),
                    owner.start_byte,
                    owner.end_byte
                ),
            }),
            Some(_) => Ok(symbol),
            None => {
                self.owners.insert(symbol.clone(), owner);
                Ok(symbol)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectRelativePath;

    fn builder() -> IdentityBuilder {
        IdentityBuilder::new(
            "scippy",
            Package {
                manager: "pip".into(),
                name: "proj".into(),
                version: "0.1".into(),
            },
        )
    }

    fn method(name: &str) -> Vec<Descriptor> {
        vec![
            Descriptor::Namespace("services".into()),
            Descriptor::Method {
                name: name.into(),
                disambiguator: String::new(),
            },
        ]
    }

    fn anchor(path: &str, start: u32) -> SourceAnchor {
        SourceAnchor {
            path: ProjectRelativePath::new(path).unwrap(),
            start_byte: start,
            end_byte: start + 3,
        }
    }

    #[test]
    fn plain_symbol() {
        let s = builder().symbol(&method("process")).unwrap();
        assert_eq!(s.as_str(), "scippy pip proj 0.1 services/process().");
    }

    #[test]
    fn escapes_names_and_round_trips() {
        for name in ["café", "with space", "a`b", "日本", "x-y$z+1", "``"] {
            let s = builder().symbol(&method(name)).unwrap();
            let parsed = parse_symbol(s.as_str()).unwrap();
            assert_eq!(parsed.descriptors[1].name, name, "{}", s.as_str());
        }
        let s = builder().symbol(&method("a`b")).unwrap();
        assert_eq!(s.as_str(), "scippy pip proj 0.1 services/`a``b`().");
        let s = builder().symbol(&method("café")).unwrap();
        assert_eq!(s.as_str(), "scippy pip proj 0.1 services/`café`().");
    }

    #[test]
    fn escapes_spaces_in_package_fields() {
        let b = IdentityBuilder::new(
            "scippy",
            Package {
                manager: "pip".into(),
                name: "my proj".into(),
                version: "1 0".into(),
            },
        );
        let s = b.symbol(&method("f")).unwrap();
        assert_eq!(s.as_str(), "scippy pip my  proj 1  0 services/f().");
        let parsed = parse_symbol(s.as_str()).unwrap();
        assert_eq!(parsed.package.name, "my proj");
    }

    #[test]
    fn empty_package_fields_are_dots() {
        let b = IdentityBuilder::new(
            "scippy",
            Package {
                manager: String::new(),
                name: "p".into(),
                version: String::new(),
            },
        );
        assert_eq!(
            b.symbol(&method("f")).unwrap().as_str(),
            "scippy . p . services/f()."
        );
    }

    #[test]
    fn all_descriptor_kinds() {
        let s = builder()
            .symbol(&[
                Descriptor::Namespace("m".into()),
                Descriptor::Type("C".into()),
                Descriptor::Term("attr".into()),
                Descriptor::Method {
                    name: "run".into(),
                    disambiguator: "2".into(),
                },
            ])
            .unwrap();
        assert_eq!(s.as_str(), "scippy pip proj 0.1 m/C#attr.run(2).");
    }

    #[test]
    fn rejects_invalid_input() {
        let b = builder();
        assert!(b.symbol(&[]).is_err(), "no descriptors");
        assert!(b.symbol(&method("")).is_err(), "empty name");
        let bad_disambiguator = [Descriptor::Method {
            name: "f".into(),
            disambiguator: "a b".into(),
        }];
        assert!(b.symbol(&bad_disambiguator).is_err());
        let reserved = IdentityBuilder::new(
            "local",
            Package {
                manager: "a".into(),
                name: "b".into(),
                version: "c".into(),
            },
        );
        assert!(reserved.symbol(&method("f")).is_err(), "reserved scheme");
        let empty_scheme = IdentityBuilder::new(
            "",
            Package {
                manager: "a".into(),
                name: "b".into(),
                version: "c".into(),
            },
        );
        assert!(empty_scheme.symbol(&method("f")).is_err(), "empty scheme");
        let dot = IdentityBuilder::new(
            "s",
            Package {
                manager: "a".into(),
                name: ".".into(),
                version: "c".into(),
            },
        );
        assert!(
            dot.symbol(&method("f")).is_err(),
            "a lone dot reads as an empty field"
        );
    }

    #[test]
    fn local_symbols() {
        assert_eq!(SymbolId::local(7).as_str(), "local 7");
        assert!(SymbolId::local(7).is_local());
        assert!(!builder().symbol(&method("f")).unwrap().is_local());
    }

    #[test]
    fn same_declaration_may_define_a_symbol_again() {
        let mut b = builder();
        let one = b.define(&method("f"), anchor("a.py", 0)).unwrap();
        let two = b.define(&method("f"), anchor("a.py", 0)).unwrap();
        assert_eq!(one, two);
    }

    #[test]
    fn rejects_a_collision() {
        let mut b = builder();
        b.define(&method("f"), anchor("a.py", 0)).unwrap();
        let err = b.define(&method("f"), anchor("b.py", 0)).unwrap_err();
        assert!(matches!(err, ScippyError::SymbolCollision { .. }), "{err}");
        // A different name does not collide.
        b.define(&method("g"), anchor("b.py", 0)).unwrap();
    }
}
