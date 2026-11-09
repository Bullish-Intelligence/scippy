//! End-to-end tests of the Python pack through the whole pipeline.
//!
//! Each fixture is a small project under `tests/fixtures`. The tests index it,
//! parse the index, and check the occurrences against hand-written expectations.
//! Positive expectations live in the fixture sources as `scip test` assertions.
//! `scip test` has no negative assertion, so the tests check absent targets in Rust.
//!
//! The scip CLI steps skip with a message when `scip` is not on PATH.
//! Set `SCIPPY_REQUIRE_SCIP=1` to fail instead of skip.
//! Set `UPDATE_GOLDEN=1` to rewrite the golden snapshots.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use protobuf::Message;
use scip::types::{Index, Occurrence, SymbolRole};
use scippy_core::{IndexReport, IndexRequest, InputLimits, Severity, index_project};
use scippy_python::PythonPack;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");
const PREFIX: &str = "scippy python workspace . ";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

struct Indexed {
    root: PathBuf,
    _scratch: tempfile::TempDir,
    path: PathBuf,
    report: IndexReport,
    index: Index,
}

fn index_at(root: &Path) -> Indexed {
    index_with(IndexRequest::new(root, ""))
}

fn index_with(mut request: IndexRequest) -> Indexed {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("index.scip");
    request.output = path.clone();
    let report = index_project(&request, &[&PythonPack]).unwrap();
    let index = Index::parse_from_bytes(&fs::read(&path).unwrap()).unwrap();
    Indexed {
        root: request.root,
        _scratch: scratch,
        path,
        report,
        index,
    }
}

fn index(name: &str) -> Indexed {
    index_at(&fixture(name))
}

impl Indexed {
    fn document(&self, path: &str) -> Option<&scip::types::Document> {
        self.index
            .documents
            .iter()
            .find(|d| d.relative_path == path)
    }

    /// The occurrences of the document that start on `line` (1-based) at `column`.
    fn at(&self, path: &str, line: i32, column: i32) -> Vec<&Occurrence> {
        self.document(path)
            .unwrap_or_else(|| panic!("no document {path}"))
            .occurrences
            .iter()
            .filter(|o| match &o.typed_range {
                Some(scip::types::occurrence::Typed_range::SingleLineRange(r)) => {
                    r.line == line - 1 && r.start_character == column
                }
                _ => false,
            })
            .collect()
    }

    fn symbols_at(&self, path: &str, line: i32, column: i32) -> Vec<&str> {
        self.at(path, line, column)
            .iter()
            .map(|o| o.symbol.as_str())
            .collect()
    }

    /// The occurrences of `word` inside the first `context` of the source file.
    /// The fixture files hold assertion comments, so the tests find lines by text.
    fn find(&self, path: &str, context: &str, word: &str) -> Vec<&Occurrence> {
        let (line, column) = self.locate(path, context, word);
        self.at(path, line, column)
    }

    fn symbols_of(&self, path: &str, context: &str, word: &str) -> Vec<&str> {
        self.find(path, context, word)
            .iter()
            .map(|o| o.symbol.as_str())
            .collect()
    }

    /// The 1-based line and the byte column of `word` in the first `context`.
    fn locate(&self, path: &str, context: &str, word: &str) -> (i32, i32) {
        let text = fs::read_to_string(self.root.join(path)).unwrap();
        let at = text.find(context).expect("context") + context.rfind(word).expect("word");
        let line = text[..at].matches('\n').count() as i32 + 1;
        let column = at - text[..at].rfind('\n').map_or(0, |n| n + 1);
        (line, column as i32)
    }

    fn has_symbol(&self, path: &str, symbol: &str) -> bool {
        self.document(path)
            .unwrap()
            .occurrences
            .iter()
            .any(|o| o.symbol == symbol)
    }
}

fn global(rest: &str) -> String {
    format!("{PREFIX}{rest}")
}

fn is_definition(o: &Occurrence) -> bool {
    o.symbol_roles & SymbolRole::Definition as i32 != 0
}

fn is_import(o: &Occurrence) -> bool {
    o.symbol_roles & SymbolRole::Import as i32 != 0
}

// ---- the acceptance fixture of the concept document, section 3.2 ----

#[test]
fn an_aliased_import_gives_one_symbol_to_the_definition_the_import_and_the_call() {
    let indexed = index("alias");
    let process = global("src/services/process().");

    // The definition in services.py.
    let definition = indexed.find("src/services.py", "def process", "process");
    assert_eq!(definition.len(), 1);
    assert_eq!(definition[0].symbol, process);
    assert!(is_definition(definition[0]));

    // The import occurrence: `process` in `from services import process as run`.
    let import = indexed.find("src/app.py", "import process as run", "process");
    assert_eq!(import.len(), 1);
    assert_eq!(import[0].symbol, process);
    assert!(is_import(import[0]));
    assert!(!is_definition(import[0]));

    // The call site: `run("hello")`.
    let call = indexed.find("src/app.py", "= run(", "run");
    assert_eq!(call.len(), 1);
    assert_eq!(call[0].symbol, process);
    assert_eq!(call[0].symbol_roles, 0, "a plain reference");

    // The symbol information belongs to services.py.
    let services = indexed.document("src/services.py").unwrap();
    let info = services
        .symbols
        .iter()
        .find(|s| s.symbol == process)
        .expect("symbol information");
    assert_eq!(info.display_name, "process");
    assert_eq!(
        info.kind.enum_value().unwrap(),
        scip::types::symbol_information::Kind::Function
    );
    // The alias name `run` in the import statement gets no occurrence of its own.
    assert!(indexed.find("src/app.py", "as run", "run").is_empty());
    // The alias policy: the call had two raw ends, and it is not ambiguous.
    assert_eq!(indexed.report.references_ambiguous, 0);
    assert!(indexed.report.multi_target.alias_collapsed >= 1);
}

// ---- shadowing, unresolved, ambiguous ----

#[test]
fn a_shadowing_name_never_binds_to_the_outer_definition() {
    let indexed = index("shadow");
    let outer = global("outer/process().");
    // `process` in other.py resolves to other.py, never to outer.py.
    assert_eq!(
        indexed.symbols_of("other.py", "return process(3)", "process"),
        [global("other/process().")]
    );
    assert!(!indexed.has_symbol("other.py", &outer));
    // The module-level call in outer.py reaches the outer function.
    assert_eq!(
        indexed.symbols_of("outer.py", "return process(2)", "process"),
        [outer.as_str()]
    );
    // A local variable shadows the function: the engine drops the shadowed path.
    assert_eq!(
        indexed.symbols_of(
            "outer.py",
            "return process\n#          ^^^^^^^ reference local 2",
            "process"
        ),
        ["local 2"]
    );
    // A parameter shadows the function through precedence. The outer function is never the target.
    assert_eq!(
        indexed.symbols_of(
            "outer.py",
            "return process\n#          ^^^^^^^ reference local 1",
            "process"
        ),
        ["local 1"]
    );
}

#[test]
fn unresolved_names_get_no_target() {
    let indexed = index("unresolved");
    // `thing` comes from a module that does not exist. A decoy defines the same name.
    assert!(indexed.find("missing.py", "= thing(1)", "thing").is_empty());
    assert!(!indexed.has_symbol("missing.py", &global("decoy/thing().")));
    // A name that nobody defines.
    assert!(
        indexed
            .find("missing.py", "unknown_name(2)", "unknown_name")
            .is_empty()
    );
    assert!(indexed.report.references_unresolved >= 2);
}

#[test]
fn ambiguous_names_get_no_target() {
    let indexed = index("ambiguous");
    // `f` is imported from a in one branch and from b in the other.
    assert!(indexed.find("c.py", "value = f()", "f").is_empty());
    assert_eq!(indexed.report.references_ambiguous, 1);
    assert_eq!(indexed.report.multi_target.ambiguous, 1);
    // Each import site still resolves to its own definition.
    assert_eq!(
        indexed.symbols_of("c.py", "from a import f", "f"),
        [global("a/f().")]
    );
}

// ---- the nearest binding ----

#[test]
fn a_plain_name_prefers_its_nearest_binding() {
    let indexed = index("flow");
    // `self` is the parameter. The class that the value comes from is not a second target.
    assert_eq!(
        indexed.symbols_of("flow.py", "self.v = v", "self"),
        ["local 1"]
    );
    // `v` is the parameter. The field that it feeds is not a second target.
    assert_eq!(
        indexed.symbols_of("flow.py", "self.v = v", "v"),
        ["local 2"]
    );
    // `x = make(); x` is the local `x`, not the function that made the value.
    assert_eq!(indexed.symbols_of("flow.py", "return x", "x"), ["local 4"]);
    // `self.v` is the field, not the parameter that the field takes its value from.
    assert_eq!(
        indexed.symbols_of("flow.py", "self.w = self.v", "v"),
        [global("flow/Child#v.")]
    );
    assert_eq!(
        indexed.symbols_of("flow.py", "return self.w", "w"),
        [global("flow/Child#w.")]
    );
    // The pack refuses the parameter and the local that a member reference reaches by value
    // flow. The nearest-binding rule has its own unit tests in the core.
    let stats = &indexed.report.multi_target;
    assert!(stats.nearest_binding + stats.pack_refused >= 2);
}

#[test]
fn a_real_ambiguity_survives_the_nearest_binding_rule() {
    let indexed = index("flow");
    // `f` is imported from lib in one branch and defined in flow.py in the other.
    // Each binding is direct, so both stay and the reference has no target.
    assert!(indexed.find("flow.py", "value = f()", "f").is_empty());
    assert_eq!(indexed.report.references_ambiguous, 1);
    // The conditional import of the `ambiguous` fixture stays ambiguous too.
    let indexed = index("ambiguous");
    assert_eq!(indexed.report.references_ambiguous, 1);
}

// ---- the pack policy: plain names and class scope ----

#[test]
fn a_plain_name_never_targets_an_instance_field() {
    let indexed = index("fields");
    let state_id = global("fields/State#id.");
    // Without the policy, `id` reaches the parameter and the field `State#id`. It is the parameter.
    assert_eq!(
        indexed.symbols_of("fields.py", "print(id)", "id"),
        ["local 1"]
    );
    assert_eq!(
        indexed.symbols_of("fields.py", "assert id,", "id"),
        ["local 1"]
    );
    assert_eq!(
        indexed.symbols_of("fields.py", "id or \"main\"", "id"),
        ["local 1"]
    );
    // `self.id` is a member reference. It still resolves to the field.
    assert_eq!(
        indexed.symbols_of("fields.py", "return self.id", "id"),
        [state_id]
    );
    assert_eq!(indexed.report.references_ambiguous, 0);
    // The engine now ranks the parameter above the field. The shadow filter takes these.
    assert!(indexed.report.multi_target.shadow_filtered >= 3);
    // A method does not see the class body. The pack refuses `State#limit` and keeps the module name.
    assert_eq!(
        indexed.symbols_of("fields.py", "print(limit)", "limit"),
        [global("fields/limit.")]
    );
    assert!(indexed.report.multi_target.pack_refused >= 1);
}

#[test]
fn a_class_body_name_resolves_in_the_class_body_and_not_in_a_method() {
    let indexed = index("fields");
    // The class body binds `count`. The body reads it as a plain name.
    assert_eq!(
        indexed.symbols_of("fields.py", "label = count", "count"),
        [global("fields/State#count.")]
    );
    // A method does not see the class body. `print(count)` has no target in Python.
    assert!(
        indexed
            .find("fields.py", "print(count)", "count")
            .is_empty()
    );
    assert_eq!(
        indexed.report.unresolved_reasons.get("not-targetable"),
        Some(&1)
    );
}

#[test]
fn a_member_reference_never_names_a_local_variable() {
    let indexed = index("members");
    let field = global("members/Options#cache_dir.");
    // `cache_dir = self.options.cache_dir`: the value flows into the local `cache_dir`.
    // The member reference still names the field, with no second end.
    assert_eq!(
        indexed.symbols_of("members.py", "= self.options.cache_dir", "cache_dir"),
        [field.as_str()]
    );
    assert_eq!(indexed.report.references_ambiguous, 0);
    // The local variable keeps its own reference.
    let local = indexed.symbols_of("members.py", "        cache_dir = self", "cache_dir");
    assert_eq!(local.len(), 1);
    assert!(local[0].starts_with("local "), "{local:?}");
    assert_eq!(
        indexed.symbols_of("members.py", "return cache_dir", "cache_dir"),
        local
    );
    // A class that a function holds has a real attribute. The member reference reaches it.
    let depth = indexed.symbols_of("members.py", "return item.depth", "depth");
    assert_eq!(depth.len(), 1, "{depth:?}");
    assert!(depth[0].starts_with("local "), "{depth:?}");
    let cache = indexed.symbols_of("members.py", "item.cache_dir", "cache_dir");
    assert_eq!(cache.len(), 1, "{cache:?}");
    assert!(cache[0].starts_with("local "), "{cache:?}");
}

// ---- modules ----

/// The occurrences of a document that carry `symbol`, as `(line, start, end, roles)`.
fn module_sites(indexed: &Indexed, path: &str, symbol: &str) -> Vec<(i32, i32, i32, i32)> {
    indexed
        .document(path)
        .unwrap()
        .occurrences
        .iter()
        .filter(|o| o.symbol == symbol)
        .map(|o| match &o.typed_range {
            Some(scip::types::occurrence::Typed_range::SingleLineRange(r)) => {
                (r.line, r.start_character, r.end_character, o.symbol_roles)
            }
            other => panic!("unexpected range {other:?}"),
        })
        .collect()
}

#[test]
fn every_document_defines_its_module_at_the_start() {
    let indexed = index("modules");
    let definition = SymbolRole::Definition as i32;
    for (path, module) in [
        ("app.py", "app/"),
        ("lib/__init__.py", "lib/"),
        ("lib/util.py", "lib/util/"),
        ("lib/sub/__init__.py", "lib/sub/"),
        ("lib/sub/leaf.py", "lib/sub/leaf/"),
    ] {
        // An empty range at line 0, column 0, with the definition role.
        let sites = module_sites(&indexed, path, &global(module));
        assert!(sites.contains(&(0, 0, 0, definition)), "{path}: {sites:?}");
        let info = indexed
            .document(path)
            .unwrap()
            .symbols
            .iter()
            .find(|s| s.symbol == global(module))
            .unwrap_or_else(|| panic!("{path}: no symbol information"));
        assert_eq!(
            info.kind.enum_value().unwrap(),
            scip::types::symbol_information::Kind::Module
        );
        assert_eq!(
            info.display_name,
            module.trim_end_matches('/').rsplit('/').next().unwrap()
        );
    }
    // An empty module file has a definition too.
    assert_eq!(
        indexed
            .document("lib/sub/__init__.py")
            .unwrap()
            .occurrences
            .len(),
        1
    );
}

#[test]
fn module_names_in_imports_and_in_attribute_chains_get_occurrences() {
    let indexed = index("modules");
    let (import, none) = (SymbolRole::Import as i32, 0);
    // The site of `word` in the first `context` of a file, as `(line, start, end)` with 0-based lines.
    let site = |path: &str, context: &str, word: &str| {
        let (line, column) = indexed.locate(path, context, word);
        (line - 1, column, column + word.len() as i32)
    };
    let with = |(line, start, end): (i32, i32, i32), role| (line, start, end, role);
    let util = module_sites(&indexed, "app.py", &global("lib/util/"));
    // `import lib.util as u`: both names are import occurrences. The alias gets none.
    assert!(util.contains(&with(
        site("app.py", "import lib.util as u", "util"),
        import
    )));
    assert!(indexed.find("app.py", "as u", "u").is_empty());
    // `from lib import util`: the name `util` is a module.
    assert!(util.contains(&with(
        site("app.py", "from lib import util", "util"),
        import
    )));
    // `lib.util.VALUE`: the module name is a plain reference, and so is the alias in `u.VALUE`.
    assert!(util.contains(&with(site("app.py", "a = lib.util.VALUE", "util"), none)));
    assert!(util.contains(&with(site("app.py", "b = u.VALUE", "u"), none)));
    // The relative import `from . import util` in the package file.
    assert_eq!(
        module_sites(&indexed, "lib/__init__.py", &global("lib/util/")),
        [with(
            site("lib/__init__.py", "from . import util", "util"),
            import
        )]
    );
    // Each name of a dotted import path names its own module.
    for (module, word) in [("lib/sub/", "sub"), ("lib/sub/leaf/", "leaf")] {
        let sites = module_sites(&indexed, "deep.py", &global(module));
        assert_eq!(
            sites[0],
            with(site("deep.py", "import lib.sub.leaf", word), import)
        );
    }
    assert_eq!(indexed.report.references_ambiguous, 0);
}

#[test]
fn a_module_with_leading_blank_lines_still_defines_at_the_start() {
    let dir = tempfile::tempdir().unwrap();
    // The module node of the first file starts after the blank lines. The definition does not.
    fs::write(dir.path().join("m.py"), "\n\n\nx = 1\n").unwrap();
    fs::write(dir.path().join("n.py"), "\n\nimport m\ny = m.x\n").unwrap();
    fs::write(dir.path().join("empty.py"), "").unwrap();
    let indexed = index_at(dir.path());
    let definition = SymbolRole::Definition as i32;
    assert!(module_sites(&indexed, "m.py", &global("m/")).contains(&(0, 0, 0, definition)));
    assert!(module_sites(&indexed, "empty.py", &global("empty/")).contains(&(0, 0, 0, definition)));
    assert_eq!(indexed.symbols_of("n.py", "import m", "m"), [global("m/")]);
    assert_eq!(indexed.symbols_of("n.py", "y = m.x", "m"), [global("m/")]);
    assert_eq!(indexed.symbols_of("n.py", "y = m.x", "x"), [global("m/x.")]);
}

#[test]
fn a_package_file_at_the_project_root_has_no_module_symbol() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("__init__.py"), "x = 1\n").unwrap();
    let indexed = index_at(dir.path());
    let document = indexed.document("__init__.py").unwrap();
    assert_eq!(document.occurrences.len(), 1, "only the variable `x`");
    assert_eq!(document.symbols.len(), 1);
}

#[test]
fn a_module_does_not_make_a_name_ambiguous() {
    // `from pkg import helper` crosses the module `pkg`. The module is not a value.
    let indexed = index("packages");
    assert_eq!(
        indexed.symbols_of("lib/main.py", "x = helper()", "helper"),
        [global("lib/pkg/core/helper().")]
    );
    assert_eq!(
        indexed.symbols_of("lib/main.py", "from pkg import", "pkg"),
        [global("lib/pkg/")]
    );
    assert_eq!(
        indexed.symbols_of("lib/main.py", "from pkg.core import", "core"),
        [global("lib/pkg/core/")]
    );
    assert_eq!(indexed.report.references_ambiguous, 0);
}

// ---- override relationships ----

/// The `is_implementation` targets of a symbol, from its symbol information.
fn implements(indexed: &Indexed, path: &str, symbol: &str) -> Vec<String> {
    let info = indexed
        .document(path)
        .unwrap()
        .symbols
        .iter()
        .find(|s| s.symbol == symbol)
        .unwrap_or_else(|| panic!("no symbol information for {symbol}"));
    for relationship in &info.relationships {
        assert!(relationship.is_implementation);
        assert!(!relationship.is_reference);
        assert!(!relationship.is_type_definition);
        assert!(!relationship.is_definition);
    }
    info.relationships
        .iter()
        .map(|r| r.symbol.clone())
        .collect()
}

#[test]
fn an_override_implements_the_method_that_it_shadows() {
    let indexed = index("overrides");
    let at = |symbol: &str| implements(&indexed, "child.py", &global(symbol));
    // The base class comes through an alias in the import: `Child(Root)`.
    assert_eq!(at("child/Child#run()."), [global("base/Base#run().")]);
    // The nearest class that has the method wins. `Grand.run` shadows `Child.run`.
    assert_eq!(at("child/Grand#run()."), [global("child/Child#run().")]);
    // `Child` has no `stop`, so `Grand.stop` shadows the method of `Base`, two levels up.
    assert_eq!(at("child/Grand#stop()."), [global("base/Base#stop().")]);
    // A qualified base class: `Qualified(base.Base)`.
    assert_eq!(at("child/Qualified#keep()."), [global("base/Base#keep().")]);
    // Two base classes give one relationship on each branch, in symbol order.
    assert_eq!(
        at("child/Several#spin()."),
        [global("base/Left#spin()."), global("base/Right#spin().")]
    );
    // A method that no base class has gets none.
    assert!(at("child/Child#extra().").is_empty());
    // The same name in an unrelated class, in a builtin base class, and in an unresolved base.
    assert!(at("child/Unrelated#run().").is_empty());
    assert!(at("child/Failure#__str__().").is_empty());
    assert!(at("child/Missing#run().").is_empty());
    // The base methods carry no relationship.
    assert!(implements(&indexed, "base.py", &global("base/Base#run().")).is_empty());
}

#[test]
fn an_override_may_implement_a_method_of_a_later_document() {
    // `a_child.py` sorts before `z_base.py`, and the base class lives in `z_base.py`.
    let indexed = index("overrides_later");
    let paths: Vec<&str> = indexed
        .index
        .documents
        .iter()
        .map(|d| d.relative_path.as_str())
        .collect();
    assert_eq!(paths, ["a_child.py", "z_base.py"]);
    assert_eq!(
        implements(&indexed, "a_child.py", &global("a_child/Child#run().")),
        [global("z_base/Base#run().")]
    );
    let base = indexed.document("z_base.py").unwrap();
    assert!(
        base.symbols
            .iter()
            .any(|s| s.symbol == global("z_base/Base#run()."))
    );
}

// ---- builtins as external symbols ----

/// The symbol of a builtin. The version is the typeshed commit of the stub header.
fn builtin(rest: &str) -> String {
    let stub = tree_sitter_stack_graphs_python::STACK_GRAPHS_BUILTINS_SOURCE;
    let (_, tail) = stub.split_once("commit ").expect("a commit in the header");
    let hash: String = tail.chars().take_while(char::is_ascii_hexdigit).collect();
    assert_eq!(hash.len(), 40);
    format!(
        "scippy python python-stdlib typeshed-{} builtins/{rest}",
        &hash[..12]
    )
}

#[test]
fn builtins_resolve_to_external_symbols() {
    let indexed = index("builtins");
    let at = |context, word| indexed.symbols_of("use.py", context, word);
    assert_eq!(at("n = len(items)", "len"), [builtin("len().")]);
    assert_eq!(
        at("isinstance(n, int)", "isinstance"),
        [builtin("isinstance().")]
    );
    assert_eq!(at("isinstance(n, int)", "int"), [builtin("int#")]);
    assert_eq!(at("label = str(n)", "str"), [builtin("str#")]);
    assert_eq!(
        at("class Failure(ValueError)", "ValueError"),
        [builtin("ValueError#")]
    );
    // A builtin method through its class.
    assert_eq!(at("str.join(", "str"), [builtin("str#")]);
    assert_eq!(at("str.join(", "join"), [builtin("str#join().")]);
    // The references have the reference role.
    for o in indexed.find("use.py", "n = len(items)", "len") {
        assert_eq!(o.symbol_roles, 0);
    }
    assert_eq!(indexed.report.references_external, 7);
    assert_eq!(indexed.report.references_unresolved, 0);

    // The index lists each referenced external symbol once, with kind and display name.
    let externals = &indexed.index.external_symbols;
    let find = |symbol: &str| {
        externals
            .iter()
            .find(|e| e.symbol == symbol)
            .unwrap_or_else(|| panic!("no external symbol {symbol}"))
    };
    use scip::types::symbol_information::Kind;
    let len = find(&builtin("len()."));
    assert_eq!(len.display_name, "len");
    assert_eq!(len.kind.enum_value().unwrap(), Kind::Function);
    let join = find(&builtin("str#join()."));
    assert_eq!(join.display_name, "join");
    assert_eq!(join.kind.enum_value().unwrap(), Kind::Method);
    assert_eq!(
        find(&builtin("ValueError#")).kind.enum_value().unwrap(),
        Kind::Class
    );
    // A builtin that no reference names is not listed. The list has no duplicates.
    assert!(externals.iter().all(|e| e.symbol != builtin("print().")));
    let mut names: Vec<&str> = externals.iter().map(|e| e.symbol.as_str()).collect();
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count);
    assert_eq!(count, 6);
    // No document holds the symbols of the standard library.
    assert!(
        indexed
            .document("use.py")
            .unwrap()
            .symbols
            .iter()
            .all(|s| !s.symbol.contains("python-stdlib"))
    );
}

#[test]
fn a_local_definition_shadows_the_builtin() {
    let indexed = index("builtins");
    assert_eq!(
        indexed.symbols_of("own.py", "size = len([])", "len"),
        [global("own/len().")]
    );
    // The builtin `len` is still listed, because use.py references it.
    assert!(
        indexed
            .index
            .external_symbols
            .iter()
            .any(|e| e.symbol == builtin("len()."))
    );
    // A project that only shadows the builtin lists no external symbol of `len`.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("own.py"),
        "def len(x):\n    return 0\n\n\nsize = len([])\n",
    )
    .unwrap();
    let only = index_at(dir.path());
    assert!(only.index.external_symbols.is_empty());
    assert_eq!(
        only.symbols_of("own.py", "size = len([])", "len"),
        [global("own/len().")]
    );
}

// ---- positions ----

#[test]
fn columns_count_utf8_bytes() {
    let indexed = index("unicode");
    // `from lib import grüße, 日本`: `grüße` starts at byte 16 and is 7 bytes long.
    let o = indexed.at("use.py", 1, 16);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].symbol, global("lib/`grüße`()."));
    // `日本` follows `grüße` and `, `, so it starts at byte 25.
    assert_eq!(indexed.symbols_at("use.py", 1, 25), [global("lib/`日本`.")]);
    // `naïve = 日本`: `naïve` is 6 bytes long, so `日本` starts at byte 9.
    assert_eq!(
        indexed.symbols_of("use.py", "naïve = 日本", "日本"),
        [global("lib/`日本`.")]
    );
    assert_eq!(indexed.locate("use.py", "naïve = 日本", "日本").1, 9);
    // The call `grüße("é")` on a line with more non-ASCII text after it.
    assert_eq!(
        indexed.symbols_of("use.py", "x = grüße(", "grüße"),
        [global("lib/`grüße`().")]
    );
}

#[test]
fn crlf_line_endings_keep_the_columns() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.py"), "def f():\r\n    return 1\r\n").unwrap();
    fs::write(
        dir.path().join("b.py"),
        "from a import f\r\n\r\nx = f()\r\n",
    )
    .unwrap();
    let indexed = index_at(dir.path());
    assert_eq!(indexed.symbols_at("b.py", 1, 14), [global("a/f().")]);
    assert_eq!(indexed.symbols_at("b.py", 3, 4), [global("a/f().")]);
    assert_eq!(indexed.symbols_at("a.py", 1, 4), [global("a/f().")]);
}

// ---- syntax errors ----

#[test]
fn a_syntax_error_skips_one_file_and_the_others_still_index() {
    let indexed = index("syntax_error");
    assert!(indexed.document("bad.py").is_none());
    assert!(indexed.document("good.py").is_some());
    assert_eq!(indexed.report.files_skipped, 1);
    assert_eq!(indexed.report.files_indexed, 1);
    let diagnostic = indexed
        .report
        .diagnostics
        .iter()
        .find(|d| d.code == "syntax-error")
        .expect("a syntax-error diagnostic");
    assert_eq!(diagnostic.file.as_deref(), Some("bad.py"));
    assert!(diagnostic.range.is_some());
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(
        indexed.symbols_of("good.py", "value = fine()", "fine"),
        [global("good/fine().")]
    );
}

// ---- packages, roots, cycles ----

#[test]
fn packages_resolve_across_roots_and_cycles_end() {
    let indexed = index("packages");
    // `from pkg import helper` goes through the re-export in pkg/__init__.py.
    assert_eq!(
        indexed.symbols_of("lib/main.py", "x = helper()", "helper"),
        [global("lib/pkg/core/helper().")]
    );
    // The relative import `from ..core import helper`.
    assert_eq!(
        indexed.symbols_of("lib/pkg/sub/leaf.py", "answer = helper()", "helper"),
        [global("lib/pkg/core/helper().")]
    );
    // Two modules in different roots define `helper`. The symbols differ.
    let lib = indexed.find("lib/pkg/core.py", "def helper", "helper");
    let other = indexed.find("other/core.py", "def helper", "helper");
    assert_ne!(lib[0].symbol, other[0].symbol);
    // The import cycle between a.py and b.py resolves and the run ends.
    assert_eq!(
        indexed.symbols_of("lib/pkg/a.py", "return from_b()", "from_b"),
        [global("lib/pkg/b/from_b().")]
    );
    // A call of a method through a class instance resolves to the method.
    assert_eq!(
        indexed.symbols_of("lib/main.py", "box.get()", "get"),
        [global("lib/pkg/core/Box#get().")]
    );
}

// ---- a module and a package of one name ----

#[test]
fn a_package_shadows_a_module_of_the_same_name() {
    let indexed = index("collision");
    // `a/__init__.py` wins, as in the Python import system. `a.py` is skipped. The run succeeds.
    assert!(indexed.document("a.py").is_none());
    assert!(indexed.document("a/__init__.py").is_some());
    assert_eq!(indexed.report.files_skipped, 1);
    assert_eq!(indexed.report.files_indexed, 2);
    let warning = indexed
        .report
        .diagnostics
        .iter()
        .find(|d| d.code == "shadowed-file")
        .expect("a shadowed-file diagnostic");
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(warning.file.as_deref(), Some("a.py"));
    assert!(warning.message.contains("a/__init__.py"), "{warning}");
    // The import reaches the package.
    assert_eq!(
        indexed.symbols_of("use.py", "from a import f", "f"),
        [global("a/f().")]
    );
    assert_eq!(
        indexed.symbols_of("use.py", "value = f()", "f"),
        [global("a/f().")]
    );
    // No symbol of the shadowed module exists.
    assert!(!indexed.has_symbol("use.py", &global("a/only_in_module().")));
}

#[test]
fn a_shadowed_module_below_an_import_root_gets_the_same_policy() {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in [
        ("src/util.py", "def helper():\n    return 1\n"),
        ("src/util/__init__.py", "def helper():\n    return 2\n"),
        ("src/util/extra.py", "def more():\n    return 3\n"),
        (
            "src/main.py",
            "from util import helper\nfrom util.extra import more\n",
        ),
        // No package of this name. The module stays.
        ("src/solo.py", "def one():\n    return 4\n"),
    ] {
        let full = dir.path().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }
    let indexed = index_at(dir.path());
    assert!(indexed.document("src/util.py").is_none());
    assert!(indexed.document("src/solo.py").is_some());
    assert_eq!(indexed.report.files_skipped, 1);
    assert_eq!(
        indexed.symbols_of("src/main.py", "import helper", "helper"),
        [global("src/util/helper().")]
    );
    assert_eq!(
        indexed.symbols_of("src/main.py", "import more", "more"),
        [global("src/util/extra/more().")]
    );
}

// ---- input limits ----

#[test]
fn the_file_limit_skips_a_large_file_with_a_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("small.py"), "x = 1\n").unwrap();
    fs::write(
        dir.path().join("big.py"),
        format!("x = 1\n{}", "#".repeat(100)),
    )
    .unwrap();
    let mut request = IndexRequest::new(dir.path(), "");
    request.limits = InputLimits {
        max_file_bytes: 50,
        max_total_bytes: 1000,
    };
    let indexed = index_with(request);
    assert!(indexed.document("small.py").is_some());
    assert!(indexed.document("big.py").is_none());
    assert!(
        indexed
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "file-too-large" && d.file.as_deref() == Some("big.py"))
    );
}

#[test]
fn the_total_limit_stops_the_run_with_an_error_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.py", "b.py", "c.py"] {
        fs::write(dir.path().join(name), "x = 1\n").unwrap();
    }
    let mut request = IndexRequest::new(dir.path(), "");
    let limits = InputLimits {
        max_file_bytes: 100,
        max_total_bytes: 14,
    };
    request.limits = limits;
    let indexed = index_with(request);
    assert_eq!(indexed.report.files_indexed, 2);
    let error = indexed
        .report
        .diagnostics
        .iter()
        .find(|d| d.code == "input-limit-reached")
        .expect("a limit diagnostic");
    assert_eq!(error.severity, Severity::Error);
    assert_eq!(error.file.as_deref(), Some("c.py"));

    // Strict mode turns the error into a failure.
    let mut strict = IndexRequest::new(dir.path(), dir.path().join("strict.scip"));
    strict.limits = limits;
    strict.strict = true;
    assert!(index_project(&strict, &[&PythonPack]).is_err());
}

// ---- determinism ----

#[test]
fn another_checkout_root_changes_only_the_root_uri() {
    let copy = tempfile::tempdir().unwrap();
    copy_dir(&fixture("packages"), copy.path());
    let normalised = |mut indexed: Indexed| {
        let metadata = indexed.index.metadata.as_mut().unwrap();
        metadata.project_root.clear();
        metadata.tool_info.as_mut().unwrap().version.clear();
        indexed.index.write_to_bytes().unwrap()
    };
    let here = index("packages");
    let there = index_at(copy.path());
    assert_ne!(
        here.index.metadata.project_root,
        there.index.metadata.project_root
    );
    assert_eq!(normalised(here), normalised(there));
}

fn copy_dir(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

// ---- the official scip CLI ----

fn scip() -> Option<PathBuf> {
    let found = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join("scip"))
        .find(|candidate| candidate.is_file());
    if found.is_none() {
        assert!(
            std::env::var_os("SCIPPY_REQUIRE_SCIP").is_none(),
            "the scip CLI is not on PATH and SCIPPY_REQUIRE_SCIP is set"
        );
        eprintln!(
            "SKIPPED: the scip CLI is not on PATH. Install it from https://github.com/scip-code/scip/releases"
        );
    }
    found
}

fn run(scip: &Path, cwd: &Path, args: &[&str]) -> Output {
    let output = Command::new(scip)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    eprintln!(
        "$ scip {} ({})\n{}{}",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

const SCIP_FIXTURES: &[&str] = &[
    "alias",
    "builtins",
    "flow",
    "fields",
    "unicode",
    "packages",
    "members",
    "modules",
    "overrides",
    "overrides_later",
    "ambiguous",
    "shadow",
    "unresolved",
    "syntax_error",
];

/// Every symbol that the index defines: in a document or in the external symbols.
fn defined_symbols(index: &Index) -> std::collections::HashSet<&str> {
    index
        .documents
        .iter()
        .flat_map(|d| d.symbols.iter())
        .chain(index.external_symbols.iter())
        .map(|info| info.symbol.as_str())
        .collect()
}

/// The target of a relationship that `scip lint` 0.10.0 reports as missing.
///
/// Returns `None` for any other line.
///
/// The CLI reports this message for a target that a later document defines.
/// It checks the relationships of each document in the same loop that fills its
/// symbol table, and it visits the documents in the random order of a Go map.
/// The count of these lines changes from run to run. The `scip-python` index gets them too:
/// 548, 775 and 1172 lines in three runs on the mypy corpus.
/// See https://github.com/scip-code/scip/blob/v0.10.0/cmd/scip/lint.go#L71-L86
/// The comment at line 79 asks for a second pass, but the code makes none.
fn lint_missing_relationship_target(line: &str) -> Option<&str> {
    // The CLI writes a log timestamp before the message: `2026/11/09 10:06:05 `.
    let at = line.find("error: symbol '")?;
    let stamp = &line[..at];
    if !stamp.is_empty()
        && !(stamp.len() == 20
            && stamp
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '/' | ':' | ' ')))
    {
        return None;
    }
    let rest = &line[at + "error: symbol '".len()..];
    let rest = rest.split_once("' (#1) (in symbols for file ")?.1;
    let rest = rest.split_once(") has a relationship to '")?.1;
    let (target, tail) = rest.split_once("' (#2), ")?;
    (tail == "but couldn't find #2 in external symbols or some other document").then_some(target)
}

/// The lines of `scip lint` output that matter.
///
/// Tolerates only the message above, and only when this index does define the target.
fn lint_problems(text: &str, index: &Index) -> Vec<String> {
    let defined = defined_symbols(index);
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| {
            !lint_missing_relationship_target(line).is_some_and(|target| defined.contains(target))
        })
        .map(str::to_string)
        .collect()
}

#[test]
fn the_lint_tolerance_accepts_only_a_defined_relationship_target() {
    let indexed = index("overrides_later");
    let target = global("z_base/Base#run().");
    let line = |target: &str| {
        format!(
            "error: symbol 'x' (#1) (in symbols for file a_child.py) has a relationship to '{target}' (#2), \
             but couldn't find #2 in external symbols or some other document"
        )
    };
    assert!(lint_problems(&line(&target), &indexed.index).is_empty());
    // A target that the index does not define stays a problem.
    assert_eq!(lint_problems(&line("nowhere"), &indexed.index).len(), 1);
    // Any other message stays a problem.
    assert_eq!(
        lint_problems("error: found empty symbol in index", &indexed.index).len(),
        1
    );
    // The CLI log timestamp is allowed. Other text before the message is not.
    let stamped = format!("2026/11/09 10:06:05 {}", line(&target));
    assert!(lint_problems(&stamped, &indexed.index).is_empty());
    let other = format!("warning: {}", line(&target));
    assert_eq!(lint_problems(&other, &indexed.index).len(), 1);
}

#[test]
fn every_relationship_target_has_symbol_information() {
    for name in SCIP_FIXTURES {
        let indexed = index(name);
        let defined = defined_symbols(&indexed.index);
        for document in &indexed.index.documents {
            for info in &document.symbols {
                for relationship in &info.relationships {
                    assert!(
                        defined.contains(relationship.symbol.as_str()),
                        "{name}: {} has a relationship to {}, which no document defines",
                        info.symbol,
                        relationship.symbol
                    );
                }
            }
        }
    }
}

#[test]
fn scip_lint_accepts_every_fixture_index() {
    let Some(scip) = scip() else { return };
    for name in SCIP_FIXTURES {
        let indexed = index(name);
        let output = run(
            &scip,
            &fixture(name),
            &["lint", indexed.path.to_str().unwrap()],
        );
        let text = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr);
        let problems = lint_problems(&text, &indexed.index);
        assert!(problems.is_empty(), "{name}: {problems:?}");
    }
}

#[test]
fn scip_test_accepts_the_annotated_sources() {
    let Some(scip) = scip() else { return };
    for name in SCIP_FIXTURES {
        let indexed = index(name);
        let mut args = vec![
            "test",
            "--from",
            indexed.path.to_str().unwrap(),
            "--comment-syntax",
            "#",
        ];
        // The syntax-error fixture has a file that is not in the index on purpose.
        if *name != "syntax_error" {
            args.push("--check-documents");
        }
        // `scip test` reads the directory from its first argument.
        args.push(".");
        let output = run(&scip, &fixture(name), &args);
        assert!(output.status.success(), "{name}");
    }
}

fn check_golden(name: &str, actual: &str) {
    let path = Path::new(GOLDEN).join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden file {}", path.display()));
    assert_eq!(
        actual, expected,
        "golden file {name} differs. Run with UPDATE_GOLDEN=1 to update it."
    );
}

#[test]
fn scip_snapshot_matches_the_golden_files() {
    let Some(scip) = scip() else { return };
    let scratch = tempfile::tempdir().unwrap();
    for (name, files) in [
        ("alias", &["src/services.py", "src/app.py"][..]),
        ("ambiguous", &["c.py"][..]),
        ("shadow", &["outer.py", "other.py"][..]),
        ("unicode", &["lib.py", "use.py"][..]),
    ] {
        let indexed = index(name);
        let out = scratch.path().join(name);
        let output = run(
            &scip,
            &fixture(name),
            &[
                "snapshot",
                "--from",
                indexed.path.to_str().unwrap(),
                "--to",
                out.to_str().unwrap(),
                "--comment-syntax",
                "#",
                // The default `--strict=true` fails on every index in scip 0.10.0.
                "--strict=false",
            ],
        );
        assert!(output.status.success(), "{name}");
        for file in files {
            let actual = fs::read_to_string(out.join(file)).unwrap();
            check_golden(&format!("{name}/{file}"), &actual);
        }
    }
}
