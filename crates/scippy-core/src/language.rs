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

use std::any::Any;
use std::path::{Path, PathBuf};

use crate::identity::{Descriptor, Package};
use crate::project::{InputLimits, ProjectRelativePath};
use crate::report::Diagnostic;

/// What a declaration is, in terms that every language shares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeclKind {
    Class,
    Function,
    Variable,
    Parameter,
    Field,
    /// An interface: a type that classes implement. It holds members like a class.
    Interface,
    /// An enumeration. It holds its members.
    Enum,
    /// A name for a type: `type A = B`.
    TypeAlias,
    /// A namespace that holds declarations. It is not the module of a document.
    Namespace,
    /// A type parameter. Only the document that declares it can address it.
    TypeParameter,
    /// The module of a document. The core gives it an empty anchor at the start of the module node.
    Module,
    /// A titled part of a document, such as a heading.
    Section,
    /// A named point inside a document, such as a block marker.
    Anchor,
    /// A second name of a document or an entity.
    Alias,
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
            "interface" => Self::Interface,
            "enum" => Self::Enum,
            "type" => Self::TypeAlias,
            "namespace" => Self::Namespace,
            "typeparameter" => Self::TypeParameter,
            _ => return None,
        })
    }

    /// A container can hold other declarations.
    pub(crate) fn is_container(self) -> bool {
        matches!(
            self,
            Self::Class
                | Self::Function
                | Self::Interface
                | Self::Enum
                | Self::Namespace
                | Self::TypeAlias
        )
    }

    /// A class-like declaration holds methods that other class-likes override or implement.
    pub fn is_class_like(self) -> bool {
        matches!(self, Self::Class | Self::Interface)
    }
}

/// How a reference names its target, in terms that every language shares.
///
/// The pack marks a reference as a member with the `@member` capture, and a reference as a
/// keyword with the `@keyword` capture. Every other reference is plain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    /// A name that stands alone, such as the `x` in `f(x)`.
    Plain,
    /// A name that selects a member of a value, such as the `x` in `a.x`.
    Member,
    /// A name that selects a parameter of a callee, such as the `x` in `f(x=1)`.
    Keyword,
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
    /// The package that the project names in its manifest, when the pack found one.
    /// It replaces `default_package`. The request can still replace the name and the version.
    fn package(&self) -> Option<Package> {
        None
    }

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
    /// `siblings` lists the other declarations that the same innermost container holds
    /// directly, in source order. A language can let a later binding share the entity of an
    /// earlier one, for example a variable that rebinds a parameter.
    fn identity(
        &self,
        path: &ProjectRelativePath,
        scopes: &[Scope],
        declaration: &Scope,
        siblings: &[Scope],
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

/// A project file that is not source code and still shapes name resolution.
///
/// `tsconfig.json` is an example: it maps module names to directories. The analyzer adds
/// nodes to the stack graph for the file. The binding rules never parse it.
pub struct SupportFile {
    /// The project-relative path of the file. It becomes the file name in the graph.
    pub path: ProjectRelativePath,
    pub source: String,
    pub analyzer: Box<dyn tree_sitter_stack_graphs::FileAnalyzer>,
}

/// A syntax variant of a language that needs its own grammar and its own binding rules.
///
/// TypeScript and TSX are an example: one language, two grammars. All files of one pack share
/// one stack graph, so a file of one dialect can import a file of another dialect.
pub struct Dialect {
    pub grammar: tree_sitter::Language,
    /// Where the rules come from. It appears in messages only.
    pub tsg_path: &'static str,
    /// The `.tsg` stack graph rules of the dialect.
    pub tsg_source: &'static str,
    /// Query patterns for this dialect only. The core appends them to the query of the pack.
    /// A pattern can name nodes that only this grammar has. Use an empty string for none.
    pub query: &'static str,
}

/// The assets that the resolver needs to compile a language.
pub struct BindingRules {
    /// Where the rules come from. It appears in messages only.
    pub tsg_path: &'static str,
    /// The `.tsg` stack graph rules.
    pub tsg_source: &'static str,
    /// The builtins source, when the language has one.
    pub builtins: Option<Builtins>,
    /// More external assets, such as the stub modules of the standard library.
    /// The resolver builds each one as a file of its own, next to the builtins.
    pub assets: Vec<ExternalAsset>,
    /// Ask the stitcher for shadow aware similar path detection. Set it when the rules need
    /// it, as the Python rules do. The resolver sets it for the indexing step and for the
    /// resolution step. Without it, a name can lose a binding.
    pub shadow_aware_similar_paths: bool,
}

impl BindingRules {
    /// Every external asset: the builtins first, then the other assets.
    pub(crate) fn external_assets(&self) -> Vec<ExternalAsset> {
        let builtins = self.builtins.iter().map(|b| ExternalAsset {
            name: crate::bindings::BUILTINS_NAME.to_string(),
            path: b.path,
            source: b.source,
            globals: Vec::new(),
        });
        builtins.chain(self.assets.iter().cloned()).collect()
    }
}

/// A source file that the pack supplies and the project does not own.
///
/// The resolver builds it as a file named `name`, with the global `FILE_PATH` set to `name`
/// and the globals of the asset. A reference that ends in it resolves to an external symbol,
/// when the project model gives the declaration an [`ExternalIdentity`].
/// The model receives `name` as the `asset` argument.
#[derive(Clone, Debug)]
pub struct ExternalAsset {
    /// The file name in the graph. It cannot collide with a project path.
    pub name: String,
    /// Where the source comes from. It appears in messages only.
    pub path: &'static str,
    pub source: &'static str,
    /// Globals for the binding rules, besides `FILE_PATH`.
    pub globals: Vec<(String, String)>,
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
/// object, so the reconciler uses a weaker nearest-binding rule for it.
///
/// A match with `@anchor` marks a node in a declaration match. Its span is a second anchor of the
/// declaration. Use it when the binding rules end a path at a node other than the identifier,
/// for example the whole statement of a default export.
///
/// A match with `@alias.name` and `@alias.target` marks a binding name and a node. The binding
/// name takes the target of the reference at the span of the node. For `import * as ns from "m"`,
/// the name is `ns` and the node is the string `"m"`. A reference that ends at `ns` then names the
/// module. The name gets a reference occurrence of its own. A match with `@hidden` marks a node
/// whose reference gets no occurrence, for example a reference that the binding rules anchor at a
/// whole statement.
///
/// A match with `@scope` marks a node that holds declarations and has no name, such as an arrow
/// function. The chain of scopes of a declaration inside it has a function entry. Its name is
/// `@` and the byte offset of the start of the node, for example `@120`. No identifier has such a
/// name, and no two nodes of a document share it. The pack decides what the scope means for the
/// declarations inside.
///
/// A match with `@module` marks the root node of a document. The binding rules define the module
/// at the start of that node, with an empty span. The core joins the module declaration to that
/// span.
///
/// A match with `@supertype` marks an identifier that names a supertype in the header of a
/// class, such as a base class. The core resolves it like a reference. When it names a class of
/// the project, a method of the subclass that has the name of a method of the supertype overrides
/// that method.
///
/// Other captures are ignored.
pub struct ExtractionSpec {
    pub query: &'static str,
}

/// A language pack. The CLI registers packs in a static list.
///
/// Every pack routes files. A pack that also gives an [`Engine`] gets real indexing.
/// A pack without an engine is a stub. The core skips its files and says so in a diagnostic.
/// A grammar alone does not make a language supported.
pub trait LanguagePack: Send + Sync {
    /// Short stable identifier, for example `python`.
    fn id(&self) -> &'static str;
    /// The SCIP language name written to `Document.language`, for example `Python`.
    fn scip_language(&self) -> &'static str;
    /// Return true when this pack owns the file. `path` is the file name only.
    fn accepts(&self, path: &Path) -> bool;
    /// The Stack Graphs analysis assets. `None` when the pack does not use that engine.
    fn analysis(&self) -> Option<&dyn Analysis> {
        None
    }
    /// The engine that indexes the files of this pack. `None` when the pack does not support
    /// indexing yet. The default is the Stack Graphs engine, when [`LanguagePack::analysis`]
    /// gives assets.
    fn engine(&self) -> Option<Engine<'_>> {
        self.analysis().map(Engine::StackGraphs)
    }
    /// Say whether the pack takes part in an indexing run for the project at `root`.
    /// A pack that is not enabled claims no file. The default is yes.
    fn enabled(&self, root: &Path) -> bool {
        let _ = root;
        true
    }
    /// Rules for the walk of the project at `root`. The default changes nothing.
    fn walk_rules(&self, root: &Path) -> WalkRules {
        let _ = root;
        WalkRules::default()
    }
    /// Files outside the project root that belong to the project, such as the code
    /// repositories of a notes workspace. The default is none. The core adds each one to the
    /// walk under its virtual path and routes it to a pack like any other file.
    fn extra_files(&self, root: &Path) -> ExtraFiles {
        let _ = root;
        ExtraFiles::default()
    }
    /// The input limits that suit this project, when the pack knows them. The command-line
    /// tool uses them for a limit that the user did not set. The default is none.
    fn input_limits(&self, root: &Path) -> Option<InputLimits> {
        let _ = root;
        None
    }
}

/// A file that the project owns but that lies outside its root.
#[derive(Clone, Debug)]
pub struct ExtraFile {
    /// The virtual path of the file in the project.
    pub path: ProjectRelativePath,
    /// Where the core reads the file.
    pub real: PathBuf,
    pub size: u64,
}

/// What [`LanguagePack::extra_files`] gives back.
#[derive(Default)]
pub struct ExtraFiles {
    pub files: Vec<ExtraFile>,
    pub diagnostics: Vec<Diagnostic>,
    /// The virtual folders that hold independent code, such as the code repositories of a
    /// notes workspace. Code in one folder never refers to code in another.
    /// The core resolves the files of each folder with a binding engine of its own and frees
    /// the engine before the next folder. The peak memory then follows the largest folder.
    pub shards: Vec<ProjectRelativePath>,
    /// Data for the pack itself. The core hands it to [`NativeInput::extra`].
    pub state: Option<Box<dyn Any + Send + Sync>>,
}

/// How a pack changes the walk of the project.
///
/// The walker never enters some directories, such as `node_modules`.
/// A pack can ask to enter such a directory, for its own files only.
#[derive(Default)]
pub struct WalkRules {
    /// Called with the project-relative path of a directory that the walker ignores by default.
    /// Return true to enter it. The pack then owns the files below it, and no other pack does.
    #[allow(clippy::type_complexity)]
    pub enter_ignored: Option<Box<dyn Fn(&str) -> bool + Send + Sync>>,
    /// Called with the project-relative path of each directory and file. Return true to hide it
    /// from every pack. The walker then neither enters the directory nor lists the file.
    #[allow(clippy::type_complexity)]
    pub hidden: Option<Box<dyn Fn(&str) -> bool + Send + Sync>>,
}

/// The engine that turns the files of a pack into bindings.
pub enum Engine<'a> {
    /// A Tree-sitter grammar, Stack Graphs rules, and a query. The resolver is a graph search.
    StackGraphs(&'a dyn Analysis),
    /// The pack reads its own files and resolves its own references.
    /// It returns declarations and the targets of each reference. The core still joins the
    /// targets to declarations, assigns identities, and writes the index.
    Native(&'a dyn NativeAnalysis),
}

/// A regular file that the walker saw. A native pack sees all of them, not only its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFile {
    pub path: ProjectRelativePath,
    pub size: u64,
}

/// A file that the core read for a native pack.
#[derive(Clone, Debug)]
pub struct NativeSource {
    pub path: ProjectRelativePath,
    pub text: String,
}

/// What the core gives a native pack.
pub struct NativeInput<'a> {
    pub root: &'a Path,
    /// Every regular file of the project that the walker saw, in walk order.
    pub files: &'a [ProjectFile],
    /// The files that the pack accepts and that the core could read, in walk order.
    pub sources: &'a [NativeSource],
    /// The data that the pack gave in [`ExtraFiles::state`].
    pub extra: Option<&'a (dyn Any + Send + Sync)>,
}

/// How a reference occurrence reads its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceRole {
    Reference,
    /// The occurrence reads the content of the target, as an embed does.
    Read,
}

/// A declaration that a native pack found.
#[derive(Clone, Debug)]
pub struct NativeDeclaration {
    /// The span of the declared name, in bytes. It is the definition occurrence and the join key.
    pub span: (u32, u32),
    /// The name that the project model sees. The pack makes it unique inside its document.
    pub name: String,
    pub kind: DeclKind,
    /// The declaration is a second name of the module of its document, such as an alias.
    /// The symbol records a reference to the module.
    pub of_module: bool,
}

/// The target of a native reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeTarget {
    /// A declaration in a document of the project: the document and the span of the name.
    /// The module of a document has the span `(0, 0)`.
    Declaration(ProjectRelativePath, (u32, u32)),
    /// A symbol that the pack declares in [`NativeOutput::virtual_symbols`], by its key.
    Virtual(String),
}

/// A reference that a native pack found, with every target that it can name.
///
/// One target is a resolved reference. Two or more targets are an ambiguous reference.
/// No target is an unresolved reference.
#[derive(Clone, Debug)]
pub struct NativeReference {
    pub span: (u32, u32),
    pub role: ReferenceRole,
    pub targets: Vec<NativeTarget>,
}

/// A document of a native pack.
#[derive(Clone, Debug)]
pub struct NativeDocument {
    pub path: ProjectRelativePath,
    /// A stub has no text of its own. It exists to define a symbol, for example for a file
    /// that is not source code. The core gives it empty text, no language, and a definition
    /// at the start, and it merges the stub into a document of another pack at the same path.
    pub stub: bool,
    /// The document has a module. The module has the empty span `(0, 0)`.
    pub module: bool,
    pub declarations: Vec<NativeDeclaration>,
    pub references: Vec<NativeReference>,
}

/// A symbol that no document of the project declares, such as a tag.
#[derive(Clone, Debug)]
pub struct VirtualSymbol {
    pub key: String,
    pub identity: ExternalIdentity,
}

/// What a native pack gives back.
#[derive(Debug, Default)]
pub struct NativeOutput {
    pub documents: Vec<NativeDocument>,
    pub virtual_symbols: Vec<VirtualSymbol>,
    pub diagnostics: Vec<Diagnostic>,
}

/// A pack that reads its own files and resolves its own references.
///
/// The pack sees the whole project at once, so a reference can name a file of any kind.
/// The core runs the shared stages after it: the join of targets to declarations, the policy
/// of the project model, the symbol identities, and the writer.
pub trait NativeAnalysis: Send + Sync {
    /// The package identity that a project without a manifest uses.
    fn default_package(&self) -> Package;
    /// Build the project model for one run. `root` is the project root.
    fn project_model(&self, root: &Path) -> Box<dyn ProjectModel>;
    /// Analyze the project: find the declarations and resolve the references.
    fn analyze(&self, input: &NativeInput) -> NativeOutput;
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
    /// `root` is the project root. A pack can read its manifest files there.
    fn project_model(&self, root: &Path, files: &[ProjectRelativePath]) -> Box<dyn ProjectModel>;
    /// Repair source text that the grammar rejects but the language accepts.
    ///
    /// The core calls this for a file that has a syntax error. The pack returns the text with
    /// the offending syntax blanked out, or `None`. The repaired text must have the same length
    /// in bytes and the same line breaks, so that every position stays valid.
    /// The core parses the repaired text. If it still has an error, the core skips the file.
    /// The default repairs nothing.
    fn repair_syntax(&self, source: &str) -> Option<String> {
        let _ = source;
        None
    }
    /// The support files of the project, such as its configuration. The default is none.
    /// `root` is the project root. `files` lists the source files that the pack owns.
    /// The core adds each support file to the stack graph after the source files.
    fn support_files(&self, root: &Path, files: &[ProjectRelativePath]) -> Vec<SupportFile> {
        let _ = (root, files);
        Vec::new()
    }
    /// The dialects after the primary one. The primary dialect is the one that
    /// [`Analysis::grammar`] and [`Analysis::binding_rules`] give. It has the number 0.
    /// The dialect `extra_dialects()[i]` has the number `i + 1`.
    /// The query of the pack, the builtins, and the other assets are the same for every dialect.
    /// A dialect can add query patterns of its own.
    /// The default is no other dialect.
    fn extra_dialects(&self) -> Vec<Dialect> {
        Vec::new()
    }
    /// The number of the dialect of a file. The default is the primary dialect.
    fn dialect_of(&self, path: &ProjectRelativePath) -> usize {
        let _ = path;
        0
    }
}
