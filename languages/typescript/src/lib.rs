//! TypeScript language pack.
//!
//! The pack supplies the grammars, the binding rules of the Stack Graphs TypeScript crate with a
//! few local fixes, a declaration query, and the TypeScript module and symbol policy.
//!
//! # Scope of the pack
//!
//! | Files | Status |
//! |---|---|
//! | `.ts`, `.tsx`, `.mts`, `.cts`, `.d.ts` | Indexed. `.tsx` has its own grammar and its own rules. |
//! | `.js`, `.jsx`, `.mjs`, `.cjs` | Not indexed. The Stack Graphs rules for JavaScript are a separate crate. |
//!
//! TypeScript and TSX files share one stack graph, so a `.ts` file can import a `.tsx` file.
//! The pack has two dialects, one for each grammar (`Analysis::extra_dialects`).
//! The module name of the binding rules is the file stem. The rules do not know `.d.ts`, `.mts`,
//! or `.cts`. A `.d.ts` file has the module name `x.d`, so no import reaches it.
//! An import of `./x.mjs` or `./x.cjs` reaches no `.mts` or `.cts` file.
//!
//! # Project layout
//!
//! The pack reads ordinary projects: relative imports, with or without the `.js` extension of a
//! `.ts` file, directory imports through `index.ts`, barrels (`export *`,
//! `export * as ns from`, `export { x as y } from`), default exports, namespace imports, and
//! type-only imports.
//!
//! `package.json` at the project root names the package (`name` and `version`) and lets the code
//! import the package by its own name. A project without it gets the package name `workspace`.
//! `tsconfig.json` at the project root gives `baseUrl`, `paths`, `rootDir`, and `rootDirs`. The
//! pack removes the comments and the trailing commas, and keeps only the options that the rules
//! use. It does not follow `extends`, a `tsconfig.json` in a subdirectory, a `package.json` in a
//! subdirectory, the `exports` map of a package, or installed dependencies.
//!
//! # Symbols
//!
//! The scheme is `scippy`, the manager is `npm`, and the package name and version come from
//! `package.json`. A symbol descriptor path starts with one namespace descriptor for the module.
//! The name of that descriptor is the project-relative path of the file with its extension,
//! as in `scip-typescript`. Two files never share a module, so two files never collide, and
//! `index.ts` needs no special case. Then come the declarations that enclose the declaration, then
//! the name.
//!
//! | Declaration | Descriptor | Reach |
//! |---|---|---|
//! | module | `` `src/a.ts`/ `` | global |
//! | function | `` `src/a.ts`/f(). `` | global |
//! | class, interface, enum, type alias | `` `src/a.ts`/Name# `` | global |
//! | namespace | `` `src/a.ts`/ns/ `` | global |
//! | module-level variable, `const f = () => {}` | `` `src/a.ts`/name. `` | global |
//! | method, accessor | `` `src/a.ts`/Class#name(). `` | global |
//! | property, parameter property | `` `src/a.ts`/Class#name. `` | global |
//! | enum member | `` `src/a.ts`/Enum#Member. `` | global |
//! | member of an object type that a type alias names | `` `src/a.ts`/Alias#name. `` | global |
//! | default export of an expression | `` `src/a.ts`/default. `` | global |
//! | parameter, type parameter, member of any other object type, anything inside a function or an anonymous scope | none | document local |
//!
//! A parameter is always local. No other document can name a parameter, because TypeScript has
//! no keyword arguments. A function overload, a getter and a setter of one name, and a class that
//! merges with an interface of the same name each get one symbol. A `static` member and an
//! instance member of one name share a symbol, too. A variable in a block at the module level
//! shares the symbol of a variable of the same name at the module level. Two `catch (e)` clauses
//! of one function share a local symbol. The query does not model block scope.
//!
//! # Anonymous scopes
//!
//! An arrow function, a function expression, a class expression, a method of an object literal,
//! a function type, a call signature, and an object type that no type alias names hold
//! declarations and have no name. The query marks them with `@scope`. The core names such a scope
//! `@<byte offset>`. Everything inside is local.
//!
//! # Names that a clause binds
//!
//! The rules define the names in `import * as ns from "m"`, `import d from "m"`,
//! `import { a as b } from "m"`, `export * as ns from "m"`, and `export default name`, and do not
//! connect a reference to them. The query pairs each name with the node that it imports
//! (`@alias.name` and `@alias.target`). The name takes the target of that node. For a default
//! import the rules anchor the reference at the whole statement, so the query hides that
//! reference (`@hidden`). The rules end a path through a default export at the whole statement of
//! `export default`. The query gives the statement as a second anchor (`@anchor`) to the exported
//! declaration, or declares a variable `default` for an expression.
//!
//! # Syntax that the grammar rejects
//!
//! `tree-sitter-typescript` 0.23.2 rejects a variance annotation (`<out T>`), `export type *`, and
//! a call signature that follows a member of an interface without a `;`. `repair` blanks them
//! out with spaces and `;`, and keeps all offsets (`Analysis::repair_syntax`). The core raises the
//! warning `repaired-syntax`. A file that still fails is skipped.
//!
//! # Local fixes of the binding rules
//!
//! `rules_source` edits the text of the rules. Each fix is a bug of the rules, recorded for the
//! fork. Remove it when the fork has it.
//!
//! - The pattern that strips `.js` from a module path matches any character before `js`, `ts`,
//!   `jsx`, and `tsx`, so `./constants` resolves as `./consta`.
//! - The constraint of a type parameter has no lexical scope: `Env` in `<E extends Env>` never
//!   resolves.
//! - An arrow function and a function expression have no type parameters in scope.
//! - The static members of a class are never reachable.
//! - A parameter property (`constructor(private x: T)`) is no member of the class.
//!
//! # Known gaps of the binding rules
//!
//! A method call on an instance of a default-imported class (`new Widget().render()`), mapped and
//! conditional types (`[K in keyof T]`, `infer U`), a bare reference to an earlier member inside
//! an enum, and a lowercase JSX tag bound to a variable. The pack hides the last one. A member of
//! a value whose type needs inference stays unresolved: the pack does no type inference.
//!
//! # External symbols
//!
//! The pack loads a trimmed `lib.d.ts` (`builtins.ts` of the rules crate) so that the resolver can
//! follow values through `Array`, `Promise`, and the other standard types.
//! A reference that ends in the builtins, in `node_modules`, or in an ambient module has no
//! target in the output. The index has no external symbols for TypeScript. The pack leaves such
//! a reference unresolved, as the design allows (section 8.5), because a faithful external symbol
//! needs the package name and the version of each dependency, and the pack does not read them.
//!
//! # Plain names, members, and enum members
//!
//! The rules of TypeScript scope that the binding rules do not enforce, and that the model
//! applies to the ends of a reference:
//!
//! - A bare name never names a member of a class or an interface. A member needs a receiver.
//!   The exception is a parameter property: a bare `x` in the constructor names the parameter,
//!   which is also the field.
//! - A bare name names an enum member only inside the enum.
//! - A member reference, such as the `x` in `a.x`, never names a parameter, a type parameter, or
//!   a name that a function body binds.

use std::path::Path;

use scippy_core::{
    Analysis, BindingRules, Builtins, DeclKind, Descriptor, Dialect, ExtractionSpec, Identity,
    LanguagePack, Package, ProjectModel, ProjectRelativePath, Reach, ReferenceKind, Scope,
    SupportFile, SymbolKind,
};
use tree_sitter_stack_graphs::FileAnalyzer;
use tree_sitter_stack_graphs_typescript as rules;

pub struct TypeScriptPack;

const QUERY: &str = include_str!("../queries/declarations.scm");
/// Patterns for JSX. Only the TSX grammar has the nodes.
const TSX_QUERY: &str = include_str!("../queries/jsx.scm");

/// The extensions that the pack indexes. `.d.ts` has the extension `ts`.
const EXTENSIONS: [&str; 4] = ["ts", "tsx", "mts", "cts"];

/// The dialect number of TSX. The primary dialect is TypeScript.
const TSX: usize = 1;

/// The rules of the Stack Graphs TypeScript crate, with the local fixes that the module header lists.
///
/// The fixes edit the text of the rules: one replaces a line, two add a stanza.
/// Remove a fix when the fork has the same fix.
fn rules_source(dialect: &'static str, source: &'static str) -> &'static str {
    use std::sync::OnceLock;
    static TS: OnceLock<String> = OnceLock::new();
    static TSX: OnceLock<String> = OnceLock::new();
    let cell = if dialect == "tsx" { &TSX } else { &TS };
    cell.get_or_init(|| fix_rules(source))
}

/// The module name of an import loses its extension with the pattern `\.(js|ts|jsx|tsx)$`.
/// The rules file writes it as `"\.(js|ts|jsx|tsx)$"`. The rules language reads `\.` as `.`,
/// so the pattern matches any character before `js`, `ts`, `jsx`, or `tsx`. The import `./constants`
/// then loses `nts`, and `../../jsx` loses `/jsx`. Double the backslash.
fn fix_rules(source: &str) -> String {
    let mut fixed = source.replace(r#""\.(js|ts|jsx|tsx)$""#, r#""\\.(js|ts|jsx|tsx)$""#);
    // The constraint of a type parameter has no lexical scope in the rules, so `Env` in
    // `<E extends Env>` never resolves. The default type `= Env` has the edge. Add it.
    fixed.push_str(CONSTRAINT_SCOPE);
    // The rules give a function declaration and a method their type parameters. They give an
    // arrow function and a function expression none, so `E` in `<E>(x: E): E => x` never
    // resolves.
    fixed.push_str(FUNCTION_EXPRESSION_TYPE_PARAMETERS);
    // The rules create the scope of the static members and never fill it. `C.created` and
    // `C.make()` never resolve. A parameter property, such as `constructor(private x: T)`, is a
    // member of the class. The rules define only the parameter. `this.x` never resolves.
    fixed.push_str(STATIC_AND_PARAMETER_MEMBERS);
    fixed
}

/// A stanza for the rules language. It fills the scope of the static members, and makes the
/// parameter properties of a constructor members of the class.
const STATIC_AND_PARAMETER_MEMBERS: &str = "
[
  (public_field_definition \"static\")
  (method_definition \"static\")
]@mem {
  edge @mem.static_members -> @mem.member
}

(method_definition
  parameters:(formal_parameters
    (required_parameter (accessibility_modifier) pattern:(identifier)@name)@param)
)@mem {
  node @param.member
  node @param.member_def
  attr (@param.member) pop_symbol = \".\"
  edge @mem.type_members -> @param.member
  edge @param.member -> @param.member_def
  attr (@param.member_def) node_definition = @name, syntax_type = \"field\"
  edge @param.member_def -> @name.expr_def__typeof
}
";

/// A stanza for the rules language. It makes the type parameters of an arrow function, a function
/// expression, or a generator function visible in its parameters, its return type, and its body.
const FUNCTION_EXPRESSION_TYPE_PARAMETERS: &str = "
[
  (function_expression type_parameters:(_)@type_params)
  (arrow_function      type_parameters:(_)@type_params)
  (generator_function  type_parameters:(_)@type_params)
]@fun {
  edge @type_params.lexical_scope -> @fun.lexical_scope
  edge @fun.lexical_scope -> @type_params.defs
  attr (@fun.lexical_scope -> @type_params.defs) precedence = 1
}
";

/// A stanza for the rules language. It gives the constraint of a type parameter the scope
/// of the type parameter, as the rules do for its default type.
const CONSTRAINT_SCOPE: &str = "
(type_parameter
  constraint:(_)@constraint
)@type_param {
  edge @constraint.lexical_scope -> @type_param.lexical_scope
}
";

impl LanguagePack for TypeScriptPack {
    fn id(&self) -> &'static str {
        "typescript"
    }

    fn scip_language(&self) -> &'static str {
        "TypeScript"
    }

    fn accepts(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.contains(&e))
    }

    fn analysis(&self) -> Option<&dyn Analysis> {
        Some(self)
    }
}

impl Analysis for TypeScriptPack {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }

    fn binding_rules(&self) -> BindingRules {
        BindingRules {
            tsg_path: rules::STACK_GRAPHS_TSG_PATH,
            tsg_source: rules_source("typescript", rules::STACK_GRAPHS_TSG_TS_SOURCE),
            builtins: Some(Builtins {
                path: rules::STACK_GRAPHS_BUILTINS_PATH,
                source: rules::STACK_GRAPHS_BUILTINS_SOURCE,
            }),
            assets: Vec::new(),
            shadow_aware_similar_paths: false,
        }
    }

    fn extra_dialects(&self) -> Vec<Dialect> {
        vec![Dialect {
            grammar: tree_sitter_typescript::LANGUAGE_TSX.into(),
            tsg_path: rules::STACK_GRAPHS_TSG_PATH,
            tsg_source: rules_source("tsx", rules::STACK_GRAPHS_TSG_TSX_SOURCE),
            query: TSX_QUERY,
        }]
    }

    fn dialect_of(&self, path: &ProjectRelativePath) -> usize {
        if path.as_str().ends_with(".tsx") {
            TSX
        } else {
            0
        }
    }

    fn extraction_spec(&self) -> ExtractionSpec {
        ExtractionSpec { query: QUERY }
    }

    fn default_package(&self) -> Package {
        Package {
            manager: "npm".into(),
            name: "workspace".into(),
            version: String::new(),
        }
    }

    fn repair_syntax(&self, source: &str) -> Option<String> {
        repair(source)
    }

    fn support_files(&self, root: &Path, _files: &[ProjectRelativePath]) -> Vec<SupportFile> {
        let mut out = Vec::new();
        for (name, analyzer) in [
            (
                "package.json",
                Box::new(rules::npm_package::NpmPackageAnalyzer {}) as Box<dyn FileAnalyzer>,
            ),
            (
                "tsconfig.json",
                Box::new(rules::tsconfig::TsConfigAnalyzer {}) as Box<dyn FileAnalyzer>,
            ),
        ] {
            let Ok(text) = std::fs::read_to_string(root.join(name)) else {
                continue;
            };
            let Ok(path) = ProjectRelativePath::new(name) else {
                continue;
            };
            let source = if name == "tsconfig.json" {
                // A file that does not parse gets no support. The core would only warn.
                let Some(source) = reduce_tsconfig(&text) else {
                    continue;
                };
                source
            } else {
                text
            };
            out.push(SupportFile {
                path,
                source,
                analyzer,
            });
        }
        out
    }

    fn project_model(&self, root: &Path, _files: &[ProjectRelativePath]) -> Box<dyn ProjectModel> {
        Box::new(TypeScriptModel {
            package: read_package(root),
        })
    }
}

/// Blank out syntax that `tree-sitter-typescript` 0.23.2 rejects and TypeScript accepts.
///
/// The repair keeps the length of the text and every line break. It handles three constructs:
///
/// - a variance annotation on a type parameter: `<in T, out U>`. It blanks `in` and `out`.
/// - a type-only export of everything: `export type * from "x"`. It blanks `type`.
/// - a call signature that starts a line after another member of an interface, without a `;`.
///   The grammar reads the `<` as a continuation of the line before. The repair writes `;` over
///   the character before the `<`.
///
/// The result is `None` when no construct matches.
pub fn repair(source: &str) -> Option<String> {
    use regex::Regex;
    let mut bytes: Vec<u8> = source.as_bytes().to_vec();
    let blank = |bytes: &mut Vec<u8>, from: usize, to: usize| bytes[from..to].fill(b' ');
    let mut changed = false;

    // `in` and `out` before a type parameter name. Repeat for `in out T`.
    // Comments can sit between the bracket or the comma and the modifier.
    let modifier = Regex::new(
        r"(?s)[<,](?:\s|//[^\n]*|/\*.*?\*/)*((?:in|out)\s+(?:(?:in|out)\s+)?)[A-Za-z_$]",
    )
    .ok()?;
    let word = Regex::new(r"\b(?:in|out)\b").ok()?;
    for m in modifier.captures_iter(source) {
        let group = m.get(1)?;
        for w in word.find_iter(group.as_str()) {
            blank(
                &mut bytes,
                group.start() + w.start(),
                group.start() + w.end(),
            );
            changed = true;
        }
    }

    // `export type * from "x"` and `export type * as ns from "x"`.
    let export_all = Regex::new(r"\bexport\s+(type)\s*\*").ok()?;
    for m in export_all.captures_iter(source) {
        let group = m.get(1)?;
        blank(&mut bytes, group.start(), group.end());
        changed = true;
    }

    // A line that starts with `<`, after a line that ends a member.
    let call_signature = Regex::new(r"(?m)^[ \t]+<").ok()?;
    for m in call_signature.find_iter(source) {
        let before = &source[..m.start()];
        let previous = before
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| {
                !l.is_empty() && !l.starts_with("//") && !l.starts_with('*') && !l.starts_with("/*")
            })
            .and_then(|l| l.chars().last());
        let ends_member = previous.is_some_and(|c| {
            c.is_alphanumeric() || matches!(c, '>' | ']' | ')' | '_' | '$' | '\'' | '"' | '`')
        });
        if ends_member {
            // The match ends with the `<`. The character before it is a space or a tab.
            let at = m.end() - 2;
            bytes[at] = b';';
            changed = true;
        }
    }

    if !changed {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Make `tsconfig.json` text strict JSON: drop the comments and the trailing commas.
/// The TypeScript compiler accepts both, and the analyzer of the binding rules accepts neither.
/// The function never changes text inside a string.
fn clean_json(text: &str) -> String {
    let chars: Vec<char> = text.trim_start_matches('\u{feff}').chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                out.push(c);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        out.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    if chars[i - 1] == '"' {
                        break;
                    }
                }
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
            }
            ',' => {
                // A comma before a closing bracket is a trailing comma.
                let next = chars[i + 1..]
                    .iter()
                    .position(|c| !c.is_whitespace())
                    .map(|p| chars[i + 1 + p]);
                if !matches!(next, Some('}' | ']')) {
                    out.push(c);
                }
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Reduce `tsconfig.json` to the options that the binding rules use.
///
/// The analyzer of the rules reads the file with a crate that has a fixed list of values for
/// options such as `moduleResolution`. It rejects `Bundler`, `NodeNext`, and others that are
/// newer than the crate. The rules only use `baseUrl`, `paths`, `rootDir`, `rootDirs`,
/// `composite`, `allowJs`, `files`, `include`, and `exclude`. The reduced text has only those,
/// and only with the types that the analyzer expects. The result is `None` when the text is not
/// JSON. The pack does not follow `extends`.
fn reduce_tsconfig(text: &str) -> Option<String> {
    use serde_json::{Map, Value};
    let json: Value = serde_json::from_str(&clean_json(text)).ok()?;
    let object = json.as_object()?;
    let strings = |v: &Value| {
        v.as_array()
            .filter(|a| a.iter().all(Value::is_string))
            .cloned()
            .map(Value::Array)
    };
    let mut out = Map::new();
    for key in ["files", "include", "exclude"] {
        if let Some(list) = object.get(key).and_then(strings) {
            out.insert(key.to_string(), list);
        }
    }
    let mut options = Map::new();
    if let Some(given) = object.get("compilerOptions").and_then(Value::as_object) {
        for key in ["baseUrl", "rootDir"] {
            if let Some(v) = given.get(key).filter(|v| v.is_string()) {
                options.insert(key.to_string(), v.clone());
            }
        }
        for key in ["composite", "allowJs"] {
            if let Some(v) = given.get(key).filter(|v| v.is_boolean()) {
                options.insert(key.to_string(), v.clone());
            }
        }
        if let Some(list) = given.get("rootDirs").and_then(strings) {
            options.insert("rootDirs".to_string(), list);
        }
        if let Some(paths) = given.get("paths").and_then(Value::as_object) {
            let ok: Map<String, Value> = paths
                .iter()
                .filter_map(|(k, v)| strings(v).map(|v| (k.clone(), v)))
                .collect();
            options.insert("paths".to_string(), Value::Object(ok));
        }
    }
    out.insert("compilerOptions".to_string(), Value::Object(options));
    Some(Value::Object(out).to_string())
}

/// The package that `package.json` at the project root names.
/// A missing file, bad JSON, or a missing name gives `None`.
fn read_package(root: &Path) -> Option<Package> {
    let text = std::fs::read_to_string(root.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = json.get("name")?.as_str().filter(|n| !n.is_empty())?;
    let version = json
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    Some(Package {
        manager: "npm".into(),
        name: name.to_string(),
        version: version.to_string(),
    })
}

struct TypeScriptModel {
    package: Option<Package>,
}

/// An anonymous scope has a name that starts with `@`, followed by a byte offset.
fn is_anonymous(scope: &Scope) -> bool {
    scope.name.starts_with('@')
}

/// The file name without its extensions: `a.d.ts` gives `a`.
fn module_name(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.split('.').next().unwrap_or(file);
    if stem == "index" {
        // An index file is the module of its directory.
        if let Some((dir, _)) = path.rsplit_once('/') {
            return dir.rsplit('/').next().unwrap_or(dir).to_string();
        }
    }
    stem.to_string()
}

/// The scopes that own a declaration. A parameter property is a field of the class. Its extent
/// lies inside the constructor, so the constructor leaves the chain.
fn owners<'a>(scopes: &'a [Scope], declaration: &Scope) -> &'a [Scope] {
    match scopes {
        [rest @ .., class, constructor]
            if declaration.kind == DeclKind::Field
                && class.kind.is_class_like()
                && constructor.kind == DeclKind::Function
                && constructor.name == "constructor" =>
        {
            &scopes[..rest.len() + 1]
        }
        _ => scopes,
    }
}

impl ProjectModel for TypeScriptModel {
    fn package(&self) -> Option<Package> {
        self.package.clone()
    }

    fn resolver_globals(&self, _path: &ProjectRelativePath) -> Vec<(String, String)> {
        Vec::new()
    }

    fn module_identity(&self, path: &ProjectRelativePath) -> Option<Identity> {
        Some(Identity {
            reach: Reach::Global,
            display_name: module_name(path.as_str()),
            descriptors: vec![Descriptor::Namespace(path.as_str().to_string())],
            kind: SymbolKind::Module,
        })
    }

    fn identity(
        &self,
        path: &ProjectRelativePath,
        scopes: &[Scope],
        declaration: &Scope,
        _siblings: &[Scope],
    ) -> Identity {
        let scopes = owners(scopes, declaration);
        let mut descriptors = vec![Descriptor::Namespace(path.as_str().to_string())];
        for owner in scopes {
            descriptors.push(match owner.kind {
                DeclKind::Class | DeclKind::Interface | DeclKind::Enum | DeclKind::TypeAlias => {
                    Descriptor::Type(owner.name.clone())
                }
                DeclKind::Namespace => Descriptor::Namespace(owner.name.clone()),
                _ => Descriptor::Method {
                    name: owner.name.clone(),
                    disambiguator: String::new(),
                },
            });
        }
        let owner = scopes.last();
        let in_class = owner.is_some_and(|s| s.kind.is_class_like());
        let in_enum = owner.is_some_and(|s| s.kind == DeclKind::Enum);
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
            DeclKind::Interface => (
                Descriptor::Type(declaration.name.clone()),
                SymbolKind::Interface,
            ),
            DeclKind::Enum => (Descriptor::Type(declaration.name.clone()), SymbolKind::Enum),
            DeclKind::TypeAlias => (
                Descriptor::Type(declaration.name.clone()),
                SymbolKind::TypeAlias,
            ),
            DeclKind::Namespace => (
                Descriptor::Namespace(declaration.name.clone()),
                SymbolKind::Namespace,
            ),
            // The kinds of documents that this pack never extracts.
            DeclKind::Variable | DeclKind::Section | DeclKind::Anchor | DeclKind::Alias => (
                Descriptor::Term(declaration.name.clone()),
                SymbolKind::Variable,
            ),
            DeclKind::Parameter => (
                Descriptor::Term(declaration.name.clone()),
                SymbolKind::Parameter,
            ),
            DeclKind::TypeParameter => (
                Descriptor::Term(declaration.name.clone()),
                SymbolKind::TypeParameter,
            ),
            DeclKind::Field => (
                Descriptor::Term(declaration.name.clone()),
                if in_enum {
                    SymbolKind::EnumMember
                } else if in_class {
                    SymbolKind::Property
                } else {
                    SymbolKind::Field
                },
            ),
            DeclKind::Module => (
                Descriptor::Namespace(declaration.name.clone()),
                SymbolKind::Module,
            ),
        };
        descriptors.push(last);

        // Only the declarations that sit at the module level, in a namespace, or in a class-like
        // can be addressed from another document. A parameter and a type parameter never can.
        let inside_function = scopes
            .iter()
            .any(|s| s.kind == DeclKind::Function || is_anonymous(s));
        let local = inside_function
            || matches!(
                declaration.kind,
                DeclKind::Parameter | DeclKind::TypeParameter
            );
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

    fn accepts_target(
        &self,
        reference: ReferenceKind,
        from: &[Scope],
        scopes: &[Scope],
        declaration: &Scope,
    ) -> bool {
        let scopes = owners(scopes, declaration);
        let owner = scopes.last();
        if reference == ReferenceKind::Member {
            // A name that a function body binds is not a member of any value.
            // A parameter and a type parameter are never members.
            // An anonymous scope can be the scope of an object type, and the members of an
            // object type are members. The query cannot tell it from an arrow function.
            let in_function =
                owner.is_some_and(|s| s.kind == DeclKind::Function && !is_anonymous(s));
            return !matches!(
                declaration.kind,
                DeclKind::Parameter | DeclKind::TypeParameter
            ) && (!in_function || declaration.kind == DeclKind::Field);
        }
        let is_member = matches!(declaration.kind, DeclKind::Field | DeclKind::Function);
        // A member of a class or an interface needs a receiver.
        if is_member && owner.is_some_and(|s| s.kind.is_class_like()) {
            // A parameter property is a parameter and a field at one identifier.
            // The constructor sees it as a bare name.
            return declaration.kind == DeclKind::Field
                && from.starts_with(scopes)
                && from
                    .get(scopes.len())
                    .is_some_and(|s| s.name == "constructor");
        }
        // An enum member is visible in the enum only.
        if owner.is_some_and(|s| s.kind == DeclKind::Enum) {
            return from.starts_with(scopes);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(p: &str) -> ProjectRelativePath {
        ProjectRelativePath::new(p).unwrap()
    }

    fn scope(kind: DeclKind, name: &str) -> Scope {
        Scope {
            kind,
            name: name.to_string(),
        }
    }

    fn model() -> TypeScriptModel {
        TypeScriptModel { package: None }
    }

    fn names(identity: &Identity) -> Vec<String> {
        identity
            .descriptors
            .iter()
            .map(|d| match d {
                Descriptor::Namespace(n) => format!("{n}/"),
                Descriptor::Type(n) => format!("{n}#"),
                Descriptor::Term(n) => format!("{n}."),
                Descriptor::Method { name, .. } => format!("{name}()."),
                Descriptor::Parameter(n) => format!("({n})"),
            })
            .collect()
    }

    #[test]
    fn accepts_typescript_sources_only() {
        for yes in ["src/app.ts", "src/App.tsx", "a.d.ts", "a.mts", "a.cts"] {
            assert!(TypeScriptPack.accepts(Path::new(yes)), "{yes}");
        }
        for no in ["src/app.js", "a.jsx", "a.mjs", "app.py", "ts", "a.tsx.bak"] {
            assert!(!TypeScriptPack.accepts(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn tsx_files_use_the_tsx_dialect() {
        assert_eq!(TypeScriptPack.dialect_of(&path("a/B.tsx")), TSX);
        assert_eq!(TypeScriptPack.dialect_of(&path("a/b.ts")), 0);
        assert_eq!(TypeScriptPack.extra_dialects().len(), 1);
    }

    #[test]
    fn a_module_symbol_is_the_file_path() {
        let found = model().module_identity(&path("src/services.ts")).unwrap();
        assert_eq!(names(&found), ["src/services.ts/"]);
        assert_eq!(found.display_name, "services");
        // Two files of one directory stem never collide.
        let a = model().module_identity(&path("a/index.ts")).unwrap();
        let b = model().module_identity(&path("a.ts")).unwrap();
        assert_ne!(names(&a), names(&b));
        assert_eq!(a.display_name, "a");
        assert_eq!(
            model()
                .module_identity(&path("index.ts"))
                .unwrap()
                .display_name,
            "index"
        );
        assert_eq!(module_name("types/x.d.ts"), "x");
    }

    #[test]
    fn declarations_get_the_descriptors_of_their_owners() {
        let m = model();
        let p = path("src/a.ts");
        let id = |scopes: &[Scope], d: Scope| m.identity(&p, scopes, &d, &[]);
        let class = scope(DeclKind::Class, "C");
        let method = id(
            std::slice::from_ref(&class),
            scope(DeclKind::Function, "run"),
        );
        assert_eq!(names(&method), ["src/a.ts/", "C#", "run()."]);
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.reach, Reach::Global);
        let field = id(&[class], scope(DeclKind::Field, "x"));
        assert_eq!(names(&field), ["src/a.ts/", "C#", "x."]);
        assert_eq!(field.kind, SymbolKind::Property);
        let member = id(&[scope(DeclKind::Enum, "E")], scope(DeclKind::Field, "Red"));
        assert_eq!(member.kind, SymbolKind::EnumMember);
        let nested = id(
            &[scope(DeclKind::Namespace, "ns")],
            scope(DeclKind::Function, "f"),
        );
        assert_eq!(names(&nested), ["src/a.ts/", "ns/", "f()."]);
        assert_eq!(nested.reach, Reach::Global);
        let top = id(&[], scope(DeclKind::Interface, "I"));
        assert_eq!(names(&top), ["src/a.ts/", "I#"]);
        assert_eq!(top.kind, SymbolKind::Interface);
    }

    #[test]
    fn parameters_and_names_inside_functions_are_local() {
        let m = model();
        let p = path("a.ts");
        let function = scope(DeclKind::Function, "f");
        let anonymous = scope(DeclKind::Function, "@120");
        let id = |scopes: &[Scope], d: Scope| m.identity(&p, scopes, &d, &[]).reach;
        assert_eq!(
            id(
                std::slice::from_ref(&function),
                scope(DeclKind::Variable, "x")
            ),
            Reach::Document
        );
        assert_eq!(
            id(&[function], scope(DeclKind::Parameter, "x")),
            Reach::Document
        );
        assert_eq!(
            id(&[anonymous], scope(DeclKind::Variable, "x")),
            Reach::Document
        );
        assert_eq!(id(&[], scope(DeclKind::Parameter, "x")), Reach::Document);
        assert_eq!(
            id(&[], scope(DeclKind::TypeParameter, "T")),
            Reach::Document
        );
        assert_eq!(id(&[], scope(DeclKind::Variable, "x")), Reach::Global);
    }

    #[test]
    fn a_bare_name_never_names_a_member() {
        let m = model();
        let class = vec![scope(DeclKind::Class, "C")];
        let method = vec![scope(DeclKind::Class, "C"), scope(DeclKind::Function, "m")];
        let ctor = vec![
            scope(DeclKind::Class, "C"),
            scope(DeclKind::Function, "constructor"),
        ];
        let field = scope(DeclKind::Field, "x");
        let plain = ReferenceKind::Plain;
        assert!(!m.accepts_target(plain, &class, &class, &field));
        assert!(!m.accepts_target(plain, &method, &class, &field));
        assert!(!m.accepts_target(plain, &method, &class, &scope(DeclKind::Function, "m")));
        // A parameter property is visible in the constructor.
        assert!(m.accepts_target(plain, &ctor, &class, &field));
        // A member reference passes.
        assert!(m.accepts_target(ReferenceKind::Member, &method, &class, &field));
        // A class at the module level passes.
        assert!(m.accepts_target(plain, &method, &[], &scope(DeclKind::Class, "C")));
    }

    #[test]
    fn a_parameter_property_belongs_to_the_class() {
        let m = model();
        let p = path("a.ts");
        let chain = [
            scope(DeclKind::Class, "C"),
            scope(DeclKind::Function, "constructor"),
        ];
        let property = scope(DeclKind::Field, "x");
        let found = m.identity(&p, &chain, &property, &[]);
        assert_eq!(names(&found), ["a.ts/", "C#", "x."]);
        assert_eq!(found.reach, Reach::Global);
        // The constructor sees it as a bare name. A method does not.
        assert!(m.accepts_target(ReferenceKind::Plain, &chain, &chain, &property));
        let method = [scope(DeclKind::Class, "C"), scope(DeclKind::Function, "m")];
        assert!(!m.accepts_target(ReferenceKind::Plain, &method, &chain, &property));
        // A variable in the constructor stays local.
        let local = m.identity(&p, &chain, &scope(DeclKind::Variable, "v"), &[]);
        assert_eq!(local.reach, Reach::Document);
    }

    #[test]
    fn an_enum_member_is_visible_in_the_enum_only() {
        let m = model();
        let en = vec![scope(DeclKind::Enum, "E")];
        let member = scope(DeclKind::Field, "A");
        assert!(m.accepts_target(ReferenceKind::Plain, &en, &en, &member));
        assert!(!m.accepts_target(ReferenceKind::Plain, &[], &en, &member));
        assert!(m.accepts_target(ReferenceKind::Member, &[], &en, &member));
    }

    #[test]
    fn a_member_reference_never_names_a_local() {
        let m = model();
        let function = vec![scope(DeclKind::Function, "f")];
        let member = ReferenceKind::Member;
        let accepts =
            |scopes: &[Scope], kind| m.accepts_target(member, &[], scopes, &scope(kind, "x"));
        assert!(!accepts(&function, DeclKind::Variable));
        assert!(!accepts(&function, DeclKind::Parameter));
        assert!(!accepts(&[], DeclKind::Parameter));
        assert!(!accepts(&[], DeclKind::TypeParameter));
        assert!(accepts(&[], DeclKind::Variable));
        assert!(accepts(&function, DeclKind::Field));
        let namespace = vec![scope(DeclKind::Namespace, "ns")];
        assert!(accepts(&namespace, DeclKind::Function));
    }

    #[test]
    fn the_repair_blanks_the_syntax_that_the_grammar_rejects() {
        let source = "interface C<\n  /** note */\n  out Z,\n  // line\n  out Q\n> {}\ninterface A<in T, out U> {\n  <X>(a: X): U\n  <Y>(a: Y): T\n}\nexport type * from \"./x\"\ntype B<\n  out V = 1,\n  in out W\n> = V\n";
        let fixed = repair(source).unwrap();
        assert_eq!(fixed.len(), source.len());
        assert_eq!(fixed.matches('\n').count(), source.matches('\n').count());
        assert!(!fixed.contains("out U") && !fixed.contains("in T"));
        assert!(!fixed.contains("out Z") && !fixed.contains("out Q"));
        assert!(fixed.contains("export      * from"));
        assert!(fixed.contains("\n ;<Y>"));
        assert!(!fixed.contains("out V") && !fixed.contains("out W") && !fixed.contains("in out"));
        // The first signature follows `{`, so it keeps its text.
        assert!(fixed.contains("{\n  <X>"));
        // Code with none of the constructs is not changed. `in` and `out` stay as words.
        assert!(repair("for (const k in out) { x = a < b }\n").is_none());
        // The fixed text parses.
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        assert!(parser.parse(source, None).unwrap().root_node().has_error());
        assert!(!parser.parse(&fixed, None).unwrap().root_node().has_error());
    }

    #[test]
    fn the_extension_pattern_of_the_rules_matches_a_dot() {
        let fixed = fix_rules(rules::STACK_GRAPHS_TSG_TS_SOURCE);
        assert_ne!(
            fixed,
            rules::STACK_GRAPHS_TSG_TS_SOURCE,
            "the fix found its line"
        );
        assert!(fixed.contains(r#""\\.(js|ts|jsx|tsx)$""#));
        // The fixed rules still compile.
        assert!(
            tree_sitter_stack_graphs::StackGraphLanguage::from_source(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "fixed.tsg".into(),
                &fixed
            )
            .is_ok()
        );
    }

    #[test]
    fn clean_json_drops_comments_and_trailing_commas() {
        let text = "\u{feff}{\n // line\n \"a\": [1, 2,], /* block */\n \"b\": \"x // not a comment, ]\",\n}\n";
        let cleaned = clean_json(text);
        let value: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(value["a"], serde_json::json!([1, 2]));
        assert_eq!(value["b"], "x // not a comment, ]");
        // An escaped quote does not end the string.
        let value: serde_json::Value =
            serde_json::from_str(&clean_json(r#"{"k": "a\"//b", /* c */ "n": 1}"#)).unwrap();
        assert_eq!(value["k"], "a\"//b");
        assert_eq!(value["n"], 1);
    }

    #[test]
    fn a_tsconfig_keeps_the_options_that_the_rules_use() {
        let text = r#"{
            // comment
            "extends": "./base.json",
            "compilerOptions": {
                "moduleResolution": "Bundler", "target": "ES2022", "strict": true,
                "baseUrl": "src", "composite": true, "paths": {"@/*": ["./*"], "bad": 1},
                "rootDirs": ["a", "b"], "types": [],
            },
            "include": ["src/**/*"], "exclude": 7,
        }"#;
        let reduced: serde_json::Value =
            serde_json::from_str(&reduce_tsconfig(text).unwrap()).unwrap();
        let options = &reduced["compilerOptions"];
        assert_eq!(options["baseUrl"], "src");
        assert_eq!(options["composite"], true);
        assert_eq!(options["paths"], serde_json::json!({"@/*": ["./*"]}));
        assert_eq!(options["rootDirs"], serde_json::json!(["a", "b"]));
        assert!(options.get("moduleResolution").is_none());
        assert!(options.get("strict").is_none());
        assert_eq!(reduced["include"], serde_json::json!(["src/**/*"]));
        assert!(reduced.get("exclude").is_none());
        assert!(reduced.get("extends").is_none());
        assert!(reduce_tsconfig("{ not json").is_none());
    }

    #[test]
    fn the_package_comes_from_package_json() {
        let dir = std::env::temp_dir().join(format!("scippy-ts-pkg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_package(&dir).is_none());
        std::fs::write(
            dir.join("package.json"),
            r#"{"name": "@acme/widgets", "version": "1.2.3"}"#,
        )
        .unwrap();
        let package = read_package(&dir).unwrap();
        assert_eq!(package.manager, "npm");
        assert_eq!(package.name, "@acme/widgets");
        assert_eq!(package.version, "1.2.3");
        std::fs::write(dir.join("package.json"), r#"{"version": "1"}"#).unwrap();
        assert!(read_package(&dir).is_none());
        std::fs::write(dir.join("package.json"), "{ not json").unwrap();
        assert!(read_package(&dir).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
