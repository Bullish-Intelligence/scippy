//! Python language pack.
//!
//! The pack supplies the grammar, the binding rules of the Stack Graphs Python crate,
//! a declaration query, and the Python module and symbol policy.
//!
//! # Project layout
//!
//! The pack reads ordinary projects with absolute and relative imports.
//! It finds the import root of each file as Python does for a script or a package:
//! the first directory, going up, that has no `__init__.py`.
//! A project without a manifest gets the package name `workspace`. Pass a name to change it.
//! It does not read `pyproject.toml`, namespace packages, or installed dependencies.
//!
//! # Symbols
//!
//! A symbol descriptor path starts with the project-relative path of the module,
//! then the classes that enclose the declaration, then the name.
//! The module path is the directory of the import root followed by the module path from that root.
//! Two import roots that hold a module of one name therefore get distinct symbols.
//!
//! | Declaration | Descriptor | Reach |
//! |---|---|---|
//! | module-level function | `module/name().` | global |
//! | class | `module/Name#` | global |
//! | module-level variable | `module/name.` | global |
//! | method | `module/Class#name().` | global |
//! | class variable, `self.x` attribute | `module/Class#x.` | global |
//! | parameter, local variable, anything inside a function | none | document local |
//!
//! Declarations that share one descriptor path in one document are one entity.
//! A variable that the code rebinds, and an overloaded function, each get one symbol.
//!
//! # Builtins
//!
//! The binding rules ship a builtins stub, `builtins.py`. A reference that ends in the stub
//! resolves to an external symbol. The scheme follows `scip-python`, which names the standard
//! library package `python-stdlib`, with the module `builtins`:
//!
//! | Part | Value |
//! |---|---|
//! | scheme | `scippy`, as for every symbol of Scippy |
//! | manager | `python` |
//! | package name | `python-stdlib` |
//! | version | `typeshed-` and the first 12 characters of the typeshed commit that the stub header names |
//! | descriptors | `builtins/`, the classes that enclose the declaration, then the name |
//!
//! Examples: `builtins/len().`, `builtins/str#`, `builtins/str#join().`.
//! `scip-python` takes the version from the Python interpreter. Scippy runs no interpreter and
//! the stub comes from typeshed, so the stub header is the only version source that is true.
//! The version is empty when the header names no commit.
//! A parameter or a local name of the stub has no symbol. Other code cannot address it.

use std::collections::BTreeSet;
use std::path::Path;

use scippy_core::{
    Analysis, BindingRules, Builtins, DeclKind, Descriptor, ExternalIdentity, ExtractionSpec,
    Identity, LanguagePack, Package, ProjectModel, ProjectRelativePath, Reach, Scope, SymbolKind,
};

pub struct PythonPack;

const QUERY: &str = include_str!("../queries/declarations.scm");

/// The name that the binding rules give to the builtins stub.
const BUILTINS_ASSET: &str = "<builtins>";

/// The package of the symbols of the builtins stub.
const STDLIB_MANAGER: &str = "python";
const STDLIB_PACKAGE: &str = "python-stdlib";
const BUILTINS_MODULE: &str = "builtins";

/// The typeshed commit that the header of the stub names, shortened to 12 characters.
/// The header has a line such as `# Derived from typeshed, ..., commit <40 hex digits>.`
fn stub_version(stub: &str) -> String {
    stub.lines()
        .take_while(|line| line.starts_with('#'))
        .filter_map(|line| line.split_once("commit "))
        .filter_map(|(_, rest)| {
            let hash: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
            (hash.len() == 40).then_some(hash)
        })
        .next()
        .map_or_else(String::new, |hash| format!("typeshed-{}", &hash[..12]))
}

impl LanguagePack for PythonPack {
    fn id(&self) -> &'static str {
        "python"
    }

    fn scip_language(&self) -> &'static str {
        "Python"
    }

    fn accepts(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "py")
    }

    fn analysis(&self) -> Option<&dyn Analysis> {
        Some(self)
    }
}

impl Analysis for PythonPack {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }

    fn binding_rules(&self) -> BindingRules {
        BindingRules {
            tsg_path: tree_sitter_stack_graphs_python::STACK_GRAPHS_TSG_PATH,
            tsg_source: tree_sitter_stack_graphs_python::STACK_GRAPHS_TSG_SOURCE,
            builtins: Some(Builtins {
                path: tree_sitter_stack_graphs_python::STACK_GRAPHS_BUILTINS_PATH,
                source: tree_sitter_stack_graphs_python::STACK_GRAPHS_BUILTINS_SOURCE,
            }),
        }
    }

    fn extraction_spec(&self) -> ExtractionSpec {
        ExtractionSpec { query: QUERY }
    }

    fn default_package(&self) -> Package {
        Package {
            manager: "python".into(),
            name: "workspace".into(),
            version: String::new(),
        }
    }

    fn project_model(&self, files: &[ProjectRelativePath]) -> Box<dyn ProjectModel> {
        Box::new(PythonModel::new(files))
    }
}

/// The set of package directories decides the import root of each file.
struct PythonModel {
    /// Directories that hold an `__init__.py`, as project-relative paths. The root is `""`.
    packages: BTreeSet<String>,
    /// The version of the package that holds the builtins.
    stdlib_version: String,
}

impl PythonModel {
    fn new(files: &[ProjectRelativePath]) -> Self {
        let packages = files
            .iter()
            .filter_map(|f| {
                let path = f.as_str();
                match path.rsplit_once('/') {
                    Some((dir, "__init__.py")) => Some(dir.to_string()),
                    None if path == "__init__.py" => Some(String::new()),
                    _ => None,
                }
            })
            .collect();
        Self {
            packages,
            stdlib_version: stub_version(
                tree_sitter_stack_graphs_python::STACK_GRAPHS_BUILTINS_SOURCE,
            ),
        }
    }

    /// The import root of a file: the directory above its topmost package directory,
    /// or the directory of the file when that directory is not a package.
    /// The result is a project-relative directory. The project root is `""`.
    fn import_root<'a>(&self, path: &'a str) -> &'a str {
        let mut dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        while self.packages.contains(dir) && !dir.is_empty() {
            dir = dir.rsplit_once('/').map_or("", |(parent, _)| parent);
        }
        dir
    }

    /// The path components of the import root, and the module path below it.
    fn module_parts(&self, path: &str) -> (Vec<String>, Vec<String>) {
        let root = self.import_root(path);
        let below = if root.is_empty() {
            path
        } else {
            &path[root.len() + 1..]
        };
        let split = |s: &str| -> Vec<String> {
            s.split('/')
                .filter(|p| !p.is_empty())
                .map(String::from)
                .collect()
        };
        let mut module = split(below);
        if let Some(last) = module.last_mut() {
            if last == "__init__.py" {
                module.pop();
            } else if let Some(stem) = last.strip_suffix(".py") {
                *last = stem.to_string();
            }
        }
        (split(root), module)
    }

    fn module_descriptors(&self, path: &ProjectRelativePath) -> Vec<Descriptor> {
        let (root, module) = self.module_parts(path.as_str());
        root.into_iter()
            .chain(module)
            .map(Descriptor::Namespace)
            .collect()
    }
}

/// Escape a path as a regular expression that matches at the start of a string.
/// The binding rules read the `ROOT_PATH` global as a regular expression.
fn anchored_pattern(dir: &str) -> String {
    let mut out = String::from("^");
    for c in dir.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('/');
    out
}

impl ProjectModel for PythonModel {
    fn resolver_globals(&self, path: &ProjectRelativePath) -> Vec<(String, String)> {
        let root = self.import_root(path.as_str());
        let value = if root.is_empty() {
            String::new()
        } else {
            anchored_pattern(root)
        };
        vec![("ROOT_PATH".to_string(), value)]
    }

    fn identity(
        &self,
        path: &ProjectRelativePath,
        scopes: &[Scope],
        declaration: &Scope,
    ) -> Identity {
        describe(self.module_descriptors(path), scopes, declaration)
    }

    fn external_identity(
        &self,
        asset: &str,
        scopes: &[Scope],
        declaration: &Scope,
    ) -> Option<ExternalIdentity> {
        if asset != BUILTINS_ASSET {
            return None;
        }
        let module = vec![Descriptor::Namespace(BUILTINS_MODULE.to_string())];
        let identity = describe(module, scopes, declaration);
        if identity.reach == Reach::Document {
            return None;
        }
        Some(ExternalIdentity {
            package: Package {
                manager: STDLIB_MANAGER.to_string(),
                name: STDLIB_PACKAGE.to_string(),
                version: self.stdlib_version.clone(),
            },
            descriptors: identity.descriptors,
            kind: identity.kind,
            display_name: identity.display_name,
        })
    }
}

/// The identity of a declaration below the given module descriptors.
fn describe(
    module_descriptors: Vec<Descriptor>,
    scopes: &[Scope],
    declaration: &Scope,
) -> Identity {
    let mut descriptors = module_descriptors;

    // The part of the chain that names the owner of the declaration.
    // A `self.x` attribute belongs to the nearest enclosing class.
    let owners: &[Scope] = if declaration.kind == DeclKind::Field {
        let class = scopes.iter().rposition(|s| s.kind == DeclKind::Class);
        &scopes[..class.map_or(0, |i| i + 1)]
    } else {
        scopes
    };
    let inside_function = owners.iter().any(|s| s.kind == DeclKind::Function)
        || (declaration.kind == DeclKind::Field
            && !scopes.iter().any(|s| s.kind == DeclKind::Class));

    for owner in owners {
        descriptors.push(match owner.kind {
            DeclKind::Class => Descriptor::Type(owner.name.clone()),
            _ => Descriptor::Method {
                name: owner.name.clone(),
                disambiguator: String::new(),
            },
        });
    }
    let in_class = owners.last().is_some_and(|s| s.kind == DeclKind::Class);
    let (last, kind) = match declaration.kind {
        DeclKind::Function => (
            Descriptor::Method {
                name: declaration.name.clone(),
                disambiguator: String::new(),
            },
            if in_class {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            },
        ),
        DeclKind::Class => (
            Descriptor::Type(declaration.name.clone()),
            SymbolKind::Class,
        ),
        DeclKind::Variable => (
            Descriptor::Term(declaration.name.clone()),
            SymbolKind::Variable,
        ),
        DeclKind::Parameter => (
            Descriptor::Term(declaration.name.clone()),
            SymbolKind::Parameter,
        ),
        DeclKind::Field => (
            Descriptor::Term(declaration.name.clone()),
            SymbolKind::Field,
        ),
    };
    descriptors.push(last);

    let local = inside_function || declaration.kind == DeclKind::Parameter;
    Identity {
        reach: if local {
            Reach::Document
        } else {
            Reach::Global
        },
        descriptors,
        kind,
        display_name: declaration.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<ProjectRelativePath> {
        list.iter()
            .map(|p| ProjectRelativePath::new(p).unwrap())
            .collect()
    }

    #[test]
    fn accepts_python_sources_only() {
        assert!(PythonPack.accepts(Path::new("src/app.py")));
        assert!(!PythonPack.accepts(Path::new("src/app.pyc")));
        assert!(!PythonPack.accepts(Path::new("app.ts")));
        assert!(!PythonPack.accepts(Path::new("py")));
    }

    #[test]
    fn the_import_root_is_the_directory_above_the_topmost_package() {
        let model = PythonModel::new(&paths(&[
            "app.py",
            "src/app.py",
            "src/services.py",
            "lib/pkg/__init__.py",
            "lib/pkg/mod.py",
            "lib/pkg/sub/__init__.py",
            "lib/pkg/sub/leaf.py",
            "__init__.py",
            "top/__init__.py",
        ]));
        assert_eq!(model.import_root("app.py"), "");
        assert_eq!(model.import_root("src/app.py"), "src");
        assert_eq!(model.import_root("lib/pkg/mod.py"), "lib");
        assert_eq!(model.import_root("lib/pkg/sub/leaf.py"), "lib");
        assert_eq!(model.import_root("lib/pkg/sub/__init__.py"), "lib");
        // A package at the project root has the project root as its import root.
        assert_eq!(model.import_root("top/__init__.py"), "");
    }

    #[test]
    fn module_paths_come_from_the_import_root() {
        let model = PythonModel::new(&paths(&["lib/pkg/__init__.py", "lib/pkg/sub/__init__.py"]));
        let parts = |p: &str| model.module_parts(p);
        assert_eq!(
            parts("lib/pkg/mod.py"),
            (vec!["lib".into()], vec!["pkg".into(), "mod".into()])
        );
        assert_eq!(
            parts("lib/pkg/sub/__init__.py"),
            (vec!["lib".into()], vec!["pkg".into(), "sub".into()])
        );
        assert_eq!(parts("x.py"), (vec![], vec!["x".into()]));
    }

    #[test]
    fn the_root_path_global_is_an_anchored_pattern() {
        let model = PythonModel::new(&paths(&["a.b/c.py"]));
        let globals = model.resolver_globals(&ProjectRelativePath::new("a.b/c.py").unwrap());
        assert_eq!(globals, [("ROOT_PATH".to_string(), "^a\\.b/".to_string())]);
        let top = model.resolver_globals(&ProjectRelativePath::new("x.py").unwrap());
        assert_eq!(top, [("ROOT_PATH".to_string(), String::new())]);
    }
}
