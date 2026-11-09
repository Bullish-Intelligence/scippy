//! The contract between the core and a language pack.
//!
//! A pack supplies file routing and the SCIP language name.
//! A pack that supports indexing also supplies an [`Analysis`]:
//! - the grammar, the binding rules, and the optional builtins source of the resolver;
//! - one Tree-sitter query that marks declarations and import sites;
//! - a project model that maps a file to resolver globals and a declaration to a symbol identity.
//!
//! The core never reads the syntax of a language. It runs the query, reads the capture names,
//! and asks the project model for every language decision.

use std::path::Path;

use crate::identity::{Descriptor, Package};
use crate::project::ProjectRelativePath;

/// What a declaration is, in terms that every language shares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeclKind {
    Class,
    Function,
    Variable,
    Parameter,
    Field,
    /// The module of a document. The core gives it an empty anchor at the start of the module node.
    Module,
}

impl DeclKind {
    /// Parse the part after `definition.` in a capture name.
    pub(crate) fn from_capture(name: &str) -> Option<Self> {
        Some(match name {
            "class" => Self::Class,
            "function" => Self::Function,
            "variable" => Self::Variable,
            "parameter" => Self::Parameter,
            "field" => Self::Field,
            _ => return None,
        })
    }

    /// A container can hold other declarations.
    pub(crate) fn is_container(self) -> bool {
        matches!(self, Self::Class | Self::Function)
    }
}

/// How a reference names its target, in terms that every language shares.
///
/// The pack marks a reference as a member with the `@member` capture.
/// Every other reference is plain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    /// A name that stands alone, such as the `x` in `f(x)`.
    Plain,
    /// A name that selects a member of a value, such as the `x` in `a.x`.
    Member,
}

/// One step of the declaration chain: the kind and the name of a declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub kind: DeclKind,
    pub name: String,
}

/// How far a symbol is visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Other documents can address the symbol. It becomes a global SCIP symbol.
    Global,
    /// Only the document that declares it can address the symbol.
    /// It becomes a `local` SCIP symbol.
    Document,
}

/// The identity that a pack assigns to a declaration.
///
/// Two declarations with the same `reach` and the same `descriptors` in one document
/// declare one entity. Python rebinding is an example. They share one symbol.
#[derive(Clone, Debug)]
pub struct Identity {
    pub reach: Reach,
    /// The full descriptor path. For a `Document` reach it is only a key.
    pub descriptors: Vec<Descriptor>,
    pub kind: crate::emit::SymbolKind,
    pub display_name: String,
}

/// The identity that a pack assigns to a declaration of an external asset.
///
/// An external asset is a source file that the pack supplies and the project does not own.
/// The builtins are an example. A symbol of an external asset has the package of the asset,
/// not the package of the project. The core writes the symbols that the project references
/// to `Index.external_symbols`.
#[derive(Clone, Debug)]
pub struct ExternalIdentity {
    pub package: Package,
    pub descriptors: Vec<Descriptor>,
    pub kind: crate::emit::SymbolKind,
    pub display_name: String,
}

/// The pack's view of one project. The core builds it once for each indexing run.
pub trait ProjectModel: Send + Sync {
    /// Global variables for the binding rules of one file, for example the import root.
    fn resolver_globals(&self, path: &ProjectRelativePath) -> Vec<(String, String)>;

    /// Name the file that shadows this file, when the language lets another file win.
    ///
    /// The core skips a shadowed file, because no import can reach it and its symbols would
    /// collide with the symbols of the winner. It raises a warning that names the winner.
    /// The default shadows nothing.
    fn shadowed_by(&self, path: &ProjectRelativePath) -> Option<ProjectRelativePath> {
        let _ = path;
        None
    }

    /// Assign an identity to the module that a document defines.
    ///
    /// The core asks once for each document that has a `@module` match. The identity needs a
    /// global reach and at least one descriptor. Return `None` when the document has no module
    /// name, for example a package file at the project root. The module then has no symbol.
    /// The default gives no module a symbol.
    fn module_identity(&self, path: &ProjectRelativePath) -> Option<Identity> {
        let _ = path;
        None
    }

    /// Assign an identity to a declaration.
    ///
    /// `scopes` lists the enclosing declarations from the outermost to the innermost.
    fn identity(
        &self,
        path: &ProjectRelativePath,
        scopes: &[Scope],
        declaration: &Scope,
    ) -> Identity;

    /// Say whether a reference can name a declaration.
    ///
    /// The reconciler asks for each candidate end that joins a declaration.
    /// It drops the end when the answer is no.
    /// A reference whose every joined end is refused has no target.
    /// - `reference` tells whether the reference is plain or a member.
    /// - `from` lists the declarations that enclose the reference, from the outermost to the
    ///   innermost. An enclosing declaration is a container: a class or a function.
    ///   A reference in a parameter list or in a base class list counts as inside its container.
    /// - `scopes` lists the declarations that enclose the target, in the same order.
    /// - `declaration` is the target.
    ///
    /// The default accepts every declaration.
    fn accepts_target(
        &self,
        reference: ReferenceKind,
        from: &[Scope],
        scopes: &[Scope],
        declaration: &Scope,
    ) -> bool {
        let _ = (reference, from, scopes, declaration);
        true
    }

    /// Assign an identity to a declaration of an external asset, such as the builtins.
    ///
    /// `asset` is the name that the resolver gives to the asset.
    /// `scopes` lists the enclosing declarations from the outermost to the innermost.
    /// Return `None` when other code cannot address the declaration.
    /// The default gives no identity, so references into an asset stay unresolved.
    fn external_identity(
        &self,
        asset: &str,
        scopes: &[Scope],
        declaration: &Scope,
    ) -> Option<ExternalIdentity> {
        let _ = (asset, scopes, declaration);
        None
    }
}

/// The assets that the resolver needs to compile a language.
pub struct BindingRules {
    /// Where the rules come from. It appears in messages only.
    pub tsg_path: &'static str,
    /// The `.tsg` stack graph rules.
    pub tsg_source: &'static str,
    /// The builtins source, when the language has one.
    pub builtins: Option<Builtins>,
}

/// A source file that defines the names every file can see.
///
/// It is an external asset. The reconciler resolves a reference that ends in it to an external
/// symbol, when the project model gives the declaration an [`ExternalIdentity`].
pub struct Builtins {
    pub path: &'static str,
    pub source: &'static str,
}

/// The Tree-sitter query of a pack.
///
/// A match with `@name` and `@definition.<kind>` is a declaration. `@name` marks the identifier.
/// `@definition.<kind>` marks the whole declaration. A match with `@import` marks an identifier
/// that names an imported symbol. A match with `@member` marks an identifier that names a member
/// of a value, such as the `x` in `a.x`. The path of such a reference passes the definition of the
/// object, so the reconciler uses a weaker nearest-binding rule for it. Other captures are
/// ignored.
pub struct ExtractionSpec {
    pub query: &'static str,
}

/// A language pack. The CLI registers packs in a static list.
///
/// Every pack routes files. A pack that also gives an [`Analysis`] gets real indexing.
/// A pack without analysis is a stub. The core skips its files and says so in a diagnostic.
/// A grammar alone does not make a language supported.
pub trait LanguagePack: Send + Sync {
    /// Short stable identifier, for example `python`.
    fn id(&self) -> &'static str;
    /// The SCIP language name written to `Document.language`, for example `Python`.
    fn scip_language(&self) -> &'static str;
    /// Return true when this pack owns the file.
    fn accepts(&self, path: &Path) -> bool;
    /// The analysis assets. `None` when the pack does not support indexing yet.
    fn analysis(&self) -> Option<&dyn Analysis> {
        None
    }
}

/// What a pack supplies so that the core can extract, resolve, and identify.
pub trait Analysis: Send + Sync {
    /// The Tree-sitter grammar.
    fn grammar(&self) -> tree_sitter::Language;
    /// The rules of the resolver.
    fn binding_rules(&self) -> BindingRules;
    /// The query that finds declarations and import sites.
    fn extraction_spec(&self) -> ExtractionSpec;
    /// The package identity that a project without a manifest uses.
    fn default_package(&self) -> Package;
    /// Build the project model from all files that the pack owns.
    fn project_model(&self, files: &[ProjectRelativePath]) -> Box<dyn ProjectModel>;
}
