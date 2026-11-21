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
use std::path::Path;

use protobuf::Message;
use scip::types::{Occurrence, SymbolRole};
use scippy_core::{IndexRequest, InputLimits, Severity, index_project};
use scippy_python::PythonPack;

#[path = "../../../test-support/copy_tree.rs"]
mod copy_tree;
#[path = "../../../test-support/pack_e2e.rs"]
mod harness;
#[path = "../../../test-support/shared_cache.rs"]
mod shared_cache;

use copy_tree::copy_tree;
use harness::{
    Cached, Indexed, check_golden, defined_symbols, fixture, is_definition, lint_problems, run,
    scip,
};

const PREFIX: &str = "scippy python workspace . ";

fn index_at(root: &Path) -> Indexed {
    harness::index_at(&PythonPack, root)
}

fn index_with(request: IndexRequest) -> Indexed {
    harness::index_with(&PythonPack, request)
}

fn index(name: &str) -> Indexed {
    index_at(&fixture(name))
}

/// The index of a fixture, shared by all test processes. See [`harness::shared`].
fn shared(name: &str) -> Cached {
    harness::shared("e2e-python", &PythonPack, name)
}

fn global(rest: &str) -> String {
    format!("{PREFIX}{rest}")
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
    let indexed = shared("shadow");
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
            "return process\n#          ^^^^^^^ reference local 0",
            "process"
        ),
        ["local 0"]
    );
    // A parameter shadows the function through precedence. The outer function is never the target.
    assert_eq!(
        indexed.symbols_of("outer.py", "(process)\n    return process", "process"),
        [global("outer/local_shadow().(process)")]
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
        [global("flow/Child#__init__().(self)")]
    );
    // `v` is the parameter. The field that it feeds is not a second target.
    assert_eq!(
        indexed.symbols_of("flow.py", "self.v = v", "v"),
        [global("flow/Child#__init__().(v)")]
    );
    // `x = make(); x` is the local `x`, not the function that made the value.
    assert_eq!(indexed.symbols_of("flow.py", "return x", "x"), ["local 0"]);
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
    let param_id = global("fields/State#__init__().(id)");
    // Without the policy, `id` reaches the parameter and the field `State#id`. It is the parameter.
    assert_eq!(
        indexed.symbols_of("fields.py", "print(id)", "id"),
        [param_id.as_str()]
    );
    assert_eq!(
        indexed.symbols_of("fields.py", "assert id,", "id"),
        [param_id.as_str()]
    );
    assert_eq!(
        indexed.symbols_of("fields.py", "id or \"main\"", "id"),
        [param_id.as_str()]
    );
    // `self.id` is a member reference. It still resolves to the field.
    assert_eq!(
        indexed.symbols_of("fields.py", "return self.id", "id"),
        [state_id]
    );
    assert_eq!(indexed.report.references_ambiguous, 0);
    // The engine now ranks the parameter above the field. The shadow filter takes these.
    assert!(indexed.report.multi_target.shadow_filtered >= 3);
    // A method does not see the class body. The module name `limit` comes after the class.
    // The engine now lets that module-level name shadow the class variable on its own,
    // so the pack has nothing to refuse here.
    assert_eq!(
        indexed.symbols_of("fields.py", "print(limit)", "limit"),
        [global("fields/limit.")]
    );
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
fn module_sites(indexed: &Cached, path: &str, symbol: &str) -> Vec<(i32, i32, i32, i32)> {
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
    let indexed = shared("modules");
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
fn implements(indexed: &Cached, path: &str, symbol: &str) -> Vec<String> {
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
    let indexed = shared("overrides");
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
    let indexed = shared("overrides_later");
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

// ---- stub modules ----

#[test]
fn a_typing_name_resolves_to_the_symbol_of_the_stub() {
    let indexed = index("stubs");
    let at = |context, word| indexed.symbols_of("use.py", context, word);
    // A stub class has a symbol in the module `typing`.
    assert_eq!(at("import Any,", "Any"), [stdlib("typing/Any#")]);
    assert_eq!(at("index: Dict[str, Any]", "Any"), [stdlib("typing/Any#")]);
    // The import of a name without a class has the stub variable as its target.
    assert_eq!(
        at("List, Optional", "Optional"),
        [stdlib("typing/Optional.")]
    );
    // The stub aliases `List` and `Dict` to the builtin classes. The symbol is the class.
    assert_eq!(at("Dict, List,", "Dict"), [builtin("dict#")]);
    assert_eq!(at("Dict, List,", "List"), [builtin("list#")]);
    assert_eq!(at("rows: List[int]", "List"), [builtin("list#")]);
    assert_eq!(at("index: Dict[str", "Dict"), [builtin("dict#")]);
    // The import binding and the stub definition make one target, with the import role.
    for (context, word) in [("Dict, List,", "List"), ("import Any,", "Any")] {
        let found = indexed.find("use.py", context, word);
        assert_eq!(found.len(), 1);
        assert!(found[0].symbol_roles & SymbolRole::Import as i32 != 0);
    }
    // A dotted use goes through the module of the stub.
    assert_eq!(at("import typing", "typing"), [stdlib("typing/")]);
    assert_eq!(at("-> typing.List", "typing"), [stdlib("typing/")]);
    assert_eq!(at("-> typing.List", "List"), [builtin("list#")]);
    assert_eq!(indexed.report.references_ambiguous, 0);
    assert_eq!(indexed.report.references_unresolved, 0);

    // The index lists the external symbols once each. A stub alias adds no `typing/List`.
    let externals: Vec<&str> = indexed
        .index
        .external_symbols
        .iter()
        .map(|e| e.symbol.as_str())
        .collect();
    for wanted in [
        stdlib("typing/"),
        stdlib("typing/Any#"),
        stdlib("typing/Optional."),
        builtin("list#"),
        builtin("dict#"),
    ] {
        assert_eq!(externals.iter().filter(|s| **s == wanted).count(), 1);
    }
    assert!(externals.iter().all(|s| !s.contains("typing/List")));
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
fn a_project_module_named_typing_wins_over_the_stub() {
    let scratch = tempfile::tempdir().unwrap();
    fs::write(scratch.path().join("typing.py"), "class Any:\n    pass\n").unwrap();
    fs::write(
        scratch.path().join("use.py"),
        "from typing import Any\n\nvalue: Any\n",
    )
    .unwrap();
    let indexed = index_at(scratch.path());
    assert_eq!(
        indexed.symbols_of("use.py", "value: Any", "Any"),
        [global("typing/Any#")]
    );
    assert!(indexed.index.external_symbols.is_empty());
}

// ---- parameters as global symbols ----

#[test]
fn a_parameter_of_a_global_function_has_a_global_symbol() {
    let indexed = shared("params");
    let at = |context, word| indexed.symbols_of("lib.py", context, word);
    let found = indexed.find("lib.py", "def make(width", "width");
    assert_eq!(found.len(), 1);
    assert!(is_definition(found[0]));
    assert_eq!(found[0].symbol, global("lib/make().(width)"));
    assert_eq!(at("Box(width)", "width"), [global("lib/make().(width)")]);
    // `self` and `cls` are parameters too.
    assert_eq!(
        at("(self, size, label", "self"),
        [global("lib/Box#__init__().(self)")]
    );
    assert_eq!(at("(cls, label)", "cls"), [global("lib/Box#unit().(cls)")]);
    assert_eq!(at("cls(label)", "cls"), [global("lib/Box#unit().(cls)")]);
    // A variable that rebinds a parameter is that parameter.
    let rebound = global("lib/Box#resize().(size)");
    assert_eq!(at("size = max", "size"), [rebound.as_str()]);
    assert_eq!(at("max(size", "size"), [rebound.as_str()]);
    assert_eq!(at("return size", "size"), [rebound.as_str()]);
    assert!(is_definition(
        indexed.find("lib.py", "size = max", "size")[0]
    ));
    // A function inside a function is out of reach of other documents. Its parameter is local.
    let inner = at("def inner(b", "b");
    assert!(inner[0].starts_with("local "), "{inner:?}");
    assert_eq!(at("return b", "b"), inner);
    // The parameter of the outer function stays global.
    assert_eq!(at("inner(a)", "a"), [global("lib/outer().(a)")]);
    // A parameter has the parameter kind.
    use scip::types::symbol_information::Kind;
    let info = indexed
        .document("lib.py")
        .unwrap()
        .symbols
        .iter()
        .find(|s| s.symbol == global("lib/make().(width)"))
        .unwrap();
    assert_eq!(info.kind.enum_value().unwrap(), Kind::Parameter);
    assert_eq!(info.display_name, "width");
}

#[test]
fn a_keyword_argument_names_the_parameter_in_another_document() {
    let indexed = index("params");
    let at = |context, word| indexed.symbols_of("use.py", context, word);
    assert_eq!(
        at("Box(size=1", "size"),
        [global("lib/Box#__init__().(size)")]
    );
    assert_eq!(
        at("label=\"a\"", "label"),
        [global("lib/Box#__init__().(label)")]
    );
    assert_eq!(
        at("resize(size=2", "size"),
        [global("lib/Box#resize().(size)")]
    );
    assert_eq!(
        at("force=True", "force"),
        [global("lib/Box#resize().(force)")]
    );
    assert_eq!(at("make(width", "width"), [global("lib/make().(width)")]);
    assert_eq!(at("height=2", "height"), [global("lib/make().(height)")]);
    assert_eq!(
        at("unit(label", "label"),
        [global("lib/Box#unit().(label)")]
    );
    // The default value of the parameter is a name that the callee holds. It is not a target.
    assert_eq!(
        at("build(width", "width"),
        [global("lib/Spec#build().(width)")]
    );
    assert_eq!(at("depth=2", "depth"), [global("lib/Spec#build().(depth)")]);
    assert_eq!(indexed.report.references_ambiguous, 0);
}

#[test]
fn a_keyword_argument_names_a_field_of_a_synthesized_init() {
    let indexed = shared("synth");
    let at = |context, word| indexed.symbols_of("use.py", context, word);
    assert_eq!(at("Point(x=1", "x"), [global("lib/Point#x.")]);
    assert_eq!(at("y=2", "y"), [global("lib/Point#y.")]);
    assert_eq!(at("Sig(name", "name"), [global("lib/Sig#name.")]);
    assert_eq!(at("args=[]", "args"), [global("lib/Sig#args.")]);
    // No keyword argument names `*args` or `**kwargs`.
    assert!(at("wrap(args", "args").is_empty());
}

#[test]
fn a_keyword_argument_names_a_field_of_a_model_class_or_a_typeddict() {
    let indexed = shared("synth");
    let at = |context, word| indexed.symbols_of("use.py", context, word);
    assert_eq!(at("Model(title", "title"), [global("lib/Model#title.")]);
    // A model class takes the fields of its model base classes.
    assert_eq!(
        at("Grandchild(title", "title"),
        [global("lib/Model#title.")]
    );
    assert_eq!(at("extra=1", "extra"), [global("lib/Child#extra.")]);
    assert_eq!(at("deep=True", "deep"), [global("lib/Grandchild#deep.")]);
    // A call of a generic class names the fields of the class.
    assert_eq!(at("Item[int](value", "value"), [global("lib/Item#value.")]);
    assert_eq!(at("Opts(quiet", "quiet"), [global("lib/Opts#quiet.")]);
    assert_eq!(at("MoreOpts(quiet", "quiet"), [global("lib/Opts#quiet.")]);
    assert_eq!(at("level=1", "level"), [global("lib/MoreOpts#level.")]);
    // A class that derives from a class that is no model class takes no field.
    assert!(at("NotModel(own", "own").is_empty());
}

#[test]
fn a_global_name_resolves_to_the_module_binding() {
    let indexed = shared("synth");
    let at = |context, word| indexed.symbols_of("use.py", context, word);
    let count = global("use/count.");
    assert_eq!(at("global count", "count"), [count.as_str()]);
    assert_eq!(at("count + 1", "count"), [count.as_str()]);
    assert_eq!(at("return count", "count"), [count.as_str()]);
    // The assignment is a reference to the module binding as well.
    let found = indexed.find("use.py", "    count = count", "count");
    assert!(found.iter().any(|o| o.symbol == count && !is_definition(o)));
}

// ---- type parameters, type aliases, assignment expressions, and many-dot imports ----

#[test]
fn a_type_parameter_binds_in_its_own_scope() {
    let indexed = shared("syntax");
    let at = |context, word| indexed.find("generic.py", context, word);
    let declared = at("class Box[T", "T");
    assert_eq!(declared.len(), 1);
    assert!(is_definition(declared[0]));
    let own = declared[0].symbol.clone();
    assert!(
        own.starts_with("local "),
        "a type parameter is local: {own}"
    );
    // The type parameter wins over the module variable `T` in the class, in a method, and in
    // a function with its own type parameter.
    assert_eq!(at("item: T", "T")[0].symbol, own);
    assert_eq!(at("-> T:\n        return self", "T")[0].symbol, own);
    let function = at("first[T", "T")[0].symbol.clone();
    assert_ne!(function, own);
    assert_eq!(at("list[T]", "T")[0].symbol, function);
    assert_eq!(at("-> T:\n    return items", "T")[0].symbol, function);
    let alias = at("Pair[T", "T")[0].symbol.clone();
    assert_eq!(at("tuple[T, T]", "T")[0].symbol, alias);
    // A name outside every type parameter list still resolves to the module variable.
    assert_eq!(at("n: T", "T")[0].symbol, global("generic/T."));
    // The bound is an ordinary reference.
    assert_eq!(
        at("T: BaseModel", "BaseModel")[0].symbol,
        global("generic/BaseModel#")
    );
}

#[test]
fn a_type_alias_is_a_definition() {
    let indexed = shared("syntax");
    let at = |context, word| indexed.find("generic.py", context, word);
    let defined = at("type Alias", "Alias");
    assert_eq!(defined.len(), 1);
    assert!(is_definition(defined[0]));
    assert_eq!(defined[0].symbol, global("generic/Alias#"));
    assert_eq!(at("a: Alias", "Alias")[0].symbol, global("generic/Alias#"));
    assert_eq!(at("p: Pair", "Pair")[0].symbol, global("generic/Pair#"));
}

#[test]
fn an_assignment_expression_binds_in_the_enclosing_function() {
    let indexed = shared("syntax");
    let at = |context, word| indexed.find("walrus.py", context, word);
    let defined = at("(found :=", "found");
    assert_eq!(defined.len(), 1);
    assert!(is_definition(defined[0]));
    assert_eq!(at("return found", "found")[0].symbol, defined[0].symbol);
    // A comprehension binds the name in the function around it.
    let kept = at("(y :=", "y");
    assert!(is_definition(kept[0]));
    assert_eq!(at("return kept, y", "y")[0].symbol, kept[0].symbol);
    assert_eq!(at("x + total", "total")[0].symbol, global("walrus/total."));
}

#[test]
fn a_match_statement_binds_captures_and_references_its_names() {
    let indexed = shared("syntax");
    let at = |context: &str, word: &str| indexed.find("match.py", context, word);
    let symbol = |context: &str, word: &str| {
        let found = at(context, word);
        assert!(!found.is_empty(), "no occurrence of {word} in {context}");
        found[0].symbol.clone()
    };
    // A case body and the subject resolve as normal code.
    assert_eq!(
        symbol("match helper(event", "helper"),
        global("match/helper().")
    );
    assert_eq!(
        symbol("return color", "color"),
        global("match/handle().(color)")
    );
    // A class pattern references the class. Its keyword names reference the fields.
    assert_eq!(symbol("case Point(x=0", "Point"), global("match/Point#"));
    assert_eq!(symbol("(x=0", "x"), global("match/Point#x."));
    assert_eq!(symbol(", y=", "y"), global("match/Point#y."));
    // A value pattern references the member.
    assert_eq!(symbol("case Color.RED", "Color"), global("match/Color#"));
    assert_eq!(symbol("Color.RED", "RED"), global("match/Color#RED."));
    // A capture, a splat, and an `as` name bind in the function. A guard and a body use them.
    let bound = |context, word| {
        let found = at(context, word);
        assert!(!found.is_empty(), "no occurrence of {word} in {context}");
        let occurrence = found[0];
        assert!(is_definition(occurrence), "{word} is a definition");
        occurrence.symbol.clone()
    };
    assert_eq!(symbol("if yy >", "yy"), bound("y=yy", "yy"));
    assert_eq!(symbol("helper(yy", "yy"), bound("y=yy", "yy"));
    assert_eq!(symbol("helper(first", "first"), bound("[first", "first"));
    assert_eq!(symbol("rest\n", "rest"), bound("*rest", "rest"));
    assert_eq!(symbol("return val,", "val"), bound("\"k\": val", "val"));
    assert_eq!(symbol("val, extra", "extra"), bound("**extra", "extra"));
    assert_eq!(symbol("return shape", "shape"), bound("as shape", "shape"));
    assert_eq!(
        symbol("return captured", "captured"),
        bound("case captured", "captured")
    );
    // A name that a case binds stays visible after the match statement.
    assert_eq!(
        symbol("return left, right", "left"),
        bound("[left, right]", "left")
    );
    // The wildcard binds nothing.
    assert!(at("case _:", "_").is_empty());
}

#[test]
fn binding_forms_bind_the_right_token() {
    // The tests check columns, not only lines: `x` and `a` sit on one line in a class pattern.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("forms.py"),
        "from dataclasses import dataclass


@dataclass
class P:
    x: int
    y: int


def pattern(p):
    match p:
        case P(x=a, y=y):
            return a + y
    return 0


def star(text):
    *words, last = text.split(' ')
    words = [w for w in words]
    return words, last


def star_loop(rows):
    for head, *tail in rows:
        print(tail)


def pair(factory):
    with factory() as (left, right):
        left.read(right)
    with factory() as [one, [two, *rest]]:
        print(one, two, rest)


def attempt(path):
    try:
        data = open(path)
    except OSError:
        data = None
    else:
        print(data)
    return data


def store(item, value):
    item.kind = value
",
    )
    .unwrap();
    let indexed = index_at(dir.path());
    let at = |context: &str, word: &str| indexed.find("forms.py", context, word);
    let one = |context: &str, word: &str| {
        let found = at(context, word);
        assert_eq!(found.len(), 1, "{word} in {context}: {found:?}");
        found[0].symbol.clone()
    };
    let defines = |context: &str, word: &str| {
        let found = at(context, word);
        assert!(
            found.iter().any(|o| is_definition(o)),
            "{word} in {context} is no definition"
        );
        found[0].symbol.clone()
    };
    // A class pattern: the key `x` references the field. The capture `a` is the definition.
    assert_eq!(one("P(x=a", "x"), global("forms/P#x."));
    assert_eq!(one(", y", "y"), global("forms/P#y."));
    assert!(!is_definition(at("P(x=a", "x")[0]));
    assert_eq!(one("return a +", "a"), defines("x=a", "a"));
    assert_eq!(one("+ y\n", "y"), defines("y=y)", "y"));
    assert_ne!(one("+ y\n", "y"), global("forms/P#y."));
    // A star target binds. The comprehension reads the star target, the later rebind is another one.
    assert_eq!(
        one("[w for w in words", "words"),
        defines("*words", "words")
    );
    assert_eq!(one("return words", "words"), defines("words = [w", "words"));
    assert_eq!(one("words, last", "last"), defines(", last =", "last"));
    assert_eq!(one("print(tail", "tail"), defines("*tail", "tail"));
    // A tuple or list target of `with` binds each name, also in a nested list and after a star.
    assert_eq!(one("left.read", "left"), defines("(left", "left"));
    assert_eq!(one("read(right", "right"), defines("right)", "right"));
    assert_eq!(one("print(one", "one"), defines("[one", "one"));
    assert_eq!(one(", two,", "two"), defines("[two", "two"));
    assert_eq!(one("two, rest", "rest"), defines("*rest", "rest"));
    // A name that the `try` body binds is visible in `else`.
    assert_eq!(one("print(data", "data"), defines("data = open", "data"));
    // A store to an attribute of an unknown receiver names no other symbol.
    assert!(
        at("item.kind", "kind")
            .iter()
            .all(|o| o.symbol != global("forms/store().(value)"))
    );
}

#[test]
fn a_relative_import_with_three_dots_resolves() {
    let indexed = shared("syntax");
    let at = |context, word| indexed.find("pkg/sub/deep/mod.py", context, word);
    let limit = global("pkg/top/LIMIT.");
    assert!(
        at("import LIMIT", "LIMIT")
            .iter()
            .any(|o| o.symbol == limit)
    );
    assert!(at("print(LIMIT", "LIMIT").iter().any(|o| o.symbol == limit));
}

// ---- attribute assignment targets ----

#[test]
fn an_assignment_to_a_self_attribute_is_one_definition() {
    let indexed = index("assign");
    // `self.size = 3` declares `Child#size`. The receiver also reaches `Base#size`.
    // The token is the declaration, so the reference does not turn ambiguous.
    let found = indexed.find("child.py", "self.size = 3", "size");
    assert_eq!(found.len(), 1);
    assert!(is_definition(found[0]));
    assert_eq!(found[0].symbol, global("child/Child#size."));
    // Two assignments in one class give one entity with two definitions.
    let found = indexed.find("base.py", "self.size = 1", "size");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].symbol, global("base/Base#size."));
    assert_eq!(indexed.report.references_ambiguous, 0);
}

#[test]
fn an_assignment_to_an_attribute_of_another_value_is_a_reference() {
    let indexed = shared("assign");
    let at = |context, word| indexed.find("child.py", context, word);
    // The receiver decides the target: the field of the class of the value.
    let child = at("item.size = 5", "size");
    assert_eq!(child.len(), 1);
    assert_eq!(child[0].symbol, global("child/Child#size."));
    assert!(!is_definition(child[0]));
    let base = at("other.size = 7", "size");
    assert_eq!(base.len(), 1);
    assert_eq!(base[0].symbol, global("base/Base#size."));
    assert!(!is_definition(base[0]));
    assert_eq!(
        at("item.extra = 6", "extra")[0].symbol,
        global("child/Child#extra.")
    );
    // An attribute that no class declares has no target.
    assert!(at("item.fresh = 8", "fresh").is_empty());
}

// ---- rules of the fork: flow, classmethods, annotations, lambdas ----

/// Index a project of the given files, in a new directory. Keep the directory while the test
/// reads the source files.
fn index_files(files: &[(&str, &str)]) -> (tempfile::TempDir, Indexed) {
    let scratch = tempfile::tempdir().unwrap();
    for (name, text) in files {
        fs::write(scratch.path().join(name), text).unwrap();
    }
    let indexed = index_at(scratch.path());
    (scratch, indexed)
}

#[test]
fn an_if_with_else_hides_the_binding_before_it() {
    let lib = "def f():\n    return 0\n\n\ndef g():\n    return 0\n";
    let text = "\
from lib import f, g

if cond:
    def f():
        return 1
else:
    def f():
        return 2

value = f()

if cond:
    def g():
        return 1

other = g()
";
    let (_scratch, indexed) = index_files(&[("lib.py", lib), ("use.py", text)]);
    // Both branches define `f`, so the import of `f` is not reachable after the statement.
    assert_eq!(
        indexed.symbols_of("use.py", "value = f()", "f"),
        [global("use/f().")]
    );
    // An `elif` chain with an `else`, and a branch that nests a loop with an `if`.
    // Every path through the statement binds the name.
    let more = "\
from lib import f, g

if cond:
    def f():
        return 1
elif other:
    def f():
        return 2
else:
    def f():
        return 3

chain = f()

if cond:
    def g():
        return 1
else:
    for item in items:
        if item:
            def g():
                return 2
    def g():
        return 3

nested = g()
";
    let (_more_scratch, more_indexed) = index_files(&[("lib.py", lib), ("use.py", more)]);
    assert_eq!(
        more_indexed.symbols_of("use.py", "chain = f()", "f"),
        [global("use/f().")]
    );
    assert_eq!(
        more_indexed.symbols_of("use.py", "nested = g()", "g"),
        [global("use/g().")]
    );
    // One branch only: the import stays reachable. The reference has two targets and none is
    // chosen.
    assert!(indexed.find("use.py", "other = g()", "g").is_empty());
    assert_eq!(indexed.report.references_ambiguous, 1);
}

#[test]
fn a_nested_function_and_a_trailing_comment_do_not_drop_a_path_at_a_join() {
    let lib = "def f():\n    return 0\n";
    let text = "\
from lib import f


def outer():
    def inner():
        return 1
    # a comment after the nested function


if cond:
    def f():
        return 1
    # a comment in the branch
else:
    def f():
        return 2
# a comment after the join

value = f()


def run(cond):
    if cond:
        def helper():
            return 1
        # a comment after the nested function
    else:
        def helper():
            return 2
    # a comment after the join
    return helper()
";
    let (_scratch, indexed) = index_files(&[("lib.py", lib), ("use.py", text)]);
    assert_eq!(
        indexed.symbols_of("use.py", "value = f()", "f"),
        [global("use/f().")]
    );
    let helper = indexed.symbols_of("use.py", "return helper()", "helper");
    assert_eq!(helper.len(), 1, "{helper:?}");
    assert!(helper[0].starts_with("local "), "{helper:?}");
    assert_eq!(indexed.report.references_ambiguous, 0);
}

#[test]
fn a_call_of_cls_in_a_classmethod_names_the_parameters_of_init() {
    let lib = "\
class Box:
    def __init__(self, size):
        self.size = size

    @classmethod
    def make(cls, n):
        return cls(size=n)
";
    let text = "\
from lib import Box


class Local(Box):
    @classmethod
    def build(cls, n):
        return cls(size=n)
";
    let (_scratch, indexed) = index_files(&[("lib.py", lib), ("use.py", text)]);
    assert_eq!(
        indexed.symbols_of("lib.py", "cls(size=n)", "size"),
        [global("lib/Box#__init__().(size)")]
    );
    assert_eq!(
        indexed.symbols_of("use.py", "cls(size=n)", "size"),
        [global("lib/Box#__init__().(size)")]
    );
    assert_eq!(indexed.report.references_ambiguous, 0);
}

#[test]
fn an_optional_annotation_gives_the_attributes_of_its_class() {
    let lib = "class Foo:\n    def bar(self):\n        return 1\n";
    let text = "\
from typing import Optional, Union

from lib import Foo


def use(a: Optional[Foo], b: Union[Foo, None], c: Foo | None, d: Optional[\"Foo\"]):
    a.bar()
    b.bar()
    c.bar()
    d.bar()
";
    let (_scratch, indexed) = index_files(&[("lib.py", lib), ("use.py", text)]);
    for context in ["a.bar()", "b.bar()", "c.bar()", "d.bar()"] {
        assert_eq!(
            indexed.symbols_of("use.py", context, "bar"),
            [global("lib/Foo#bar().")],
            "{context}"
        );
    }
    assert_eq!(indexed.report.references_ambiguous, 0);
}

#[test]
fn a_lambda_body_sees_the_parameters_of_the_enclosing_function() {
    let text = "\
def outer(a, b):
    c = 3
    return lambda z: z + a + b + c
";
    let (_scratch, indexed) = index_files(&[("use.py", text)]);
    assert_eq!(
        indexed.symbols_of("use.py", "z + a", "a"),
        [global("use/outer().(a)")]
    );
    assert_eq!(
        indexed.symbols_of("use.py", "+ b", "b"),
        [global("use/outer().(b)")]
    );
    let c = indexed.find("use.py", "c = 3", "c");
    assert_eq!(c.len(), 1);
    assert_eq!(
        indexed.symbols_of("use.py", "+ c", "c"),
        [c[0].symbol.as_str()]
    );
    assert_eq!(indexed.report.references_ambiguous, 0);
}

#[test]
fn a_lambda_parameter_is_a_local_symbol_of_its_own() {
    let text = "\
def outer(z, a):
    f = lambda z: z + a
    g = lambda z: z * 2
    return f, g
";
    let (_scratch, indexed) = index_files(&[("use.py", text)]);
    let outer_z = global("use/outer().(z)");
    // The parameter of `outer` keeps its global symbol.
    assert_eq!(indexed.find("use.py", "outer(z", "z")[0].symbol, outer_z);
    // The parameter of each lambda is a local symbol. Definition and use share it.
    let first = indexed.find("use.py", "lambda z", "z")[0].symbol.clone();
    assert!(first.starts_with("local "), "{first}");
    assert_eq!(indexed.symbols_of("use.py", "z + a", "z"), [first.as_str()]);
    // The two lambdas do not share one symbol. `outer` does not hold either one.
    let second = indexed.find("use.py", "lambda z: z * 2", "z")[0]
        .symbol
        .clone();
    assert!(second.starts_with("local "), "{second}");
    assert_ne!(first, second);
    assert_eq!(
        indexed.symbols_of("use.py", "z * 2", "z"),
        [second.as_str()]
    );
    // The body still sees a parameter of the enclosing function.
    assert_eq!(
        indexed.symbols_of("use.py", "z + a", "a"),
        [global("use/outer().(a)")]
    );
    assert_eq!(indexed.report.references_ambiguous, 0);
}

// ---- builtins as external symbols ----

/// The symbol of a builtin. The version is the typeshed commit of the stub header.
fn builtin(rest: &str) -> String {
    stdlib(&format!("builtins/{rest}"))
}

/// The symbol of a name in the standard library, such as `typing/Any#`.
/// The version is the typeshed commit of the stub header. The builtins and the stubs share it.
fn stdlib(rest: &str) -> String {
    let stub = tree_sitter_stack_graphs_python::STACK_GRAPHS_BUILTINS_SOURCE;
    let (_, tail) = stub.split_once("commit ").expect("a commit in the header");
    let hash: String = tail.chars().take_while(char::is_ascii_hexdigit).collect();
    assert_eq!(hash.len(), 40);
    format!(
        "scippy python python-stdlib typeshed-{} {rest}",
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
    let indexed = shared("builtins");
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
    let indexed = shared("unicode");
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

// ---- Python 3.14 syntax that the grammar rejects (repair_syntax) ----

/// The files of `py314` that got a `repaired-syntax` diagnostic, sorted.
fn repaired_files(indexed: &Indexed) -> Vec<&str> {
    let mut files: Vec<&str> = indexed
        .report
        .diagnostics
        .iter()
        .filter(|d| d.code == "repaired-syntax")
        .filter_map(|d| d.file.as_deref())
        .collect();
    files.sort_unstable();
    files
}

/// Check that each name in `names` is a plain reference to its class in `base.py`.
fn assert_exception_references(indexed: &Indexed, file: &str, context: &str, names: &[&str]) {
    for name in names {
        let found = indexed.find(file, &format!("{context}{name}"), name);
        assert_eq!(found.len(), 1, "{file}: {name}");
        assert_eq!(found[0].symbol, global(&format!("base/{name}#")));
        assert_eq!(
            found[0].symbol_roles, 0,
            "{file}: {name} is a plain reference"
        );
    }
}

#[test]
fn an_except_clause_without_brackets_and_with_three_names_is_repaired() {
    // R-1: the grammar reads `except A, B, C:` as an error.
    let indexed = index("py314");
    assert_eq!(indexed.report.files_skipped, 0);
    assert_eq!(indexed.report.files_indexed, 6);
    assert!(indexed.document("except_many.py").is_some());
    // The first clause names three classes, and the second clause names two.
    assert_exception_references(&indexed, "except_many.py", "except ", &["Boom"]);
    assert_exception_references(&indexed, "except_many.py", "except Boom, ", &["Bust"]);
    assert_exception_references(
        &indexed,
        "except_many.py",
        "except Boom, Bust, ",
        &["Crash"],
    );
    assert_exception_references(&indexed, "except_many.py", "except ", &["Fail"]);
    assert_exception_references(&indexed, "except_many.py", "except Fail, ", &["Bust"]);
    // The source file keeps its commas.
    let text = fs::read_to_string(fixture("py314").join("except_many.py")).unwrap();
    assert!(text.contains("except Boom, Bust, Crash:"));
}

#[test]
fn an_except_star_clause_without_brackets_is_repaired() {
    // R-1: the grammar reads `except* A, B:` as an error, even with two names.
    let indexed = index("py314");
    assert!(indexed.document("except_star.py").is_some());
    assert_exception_references(&indexed, "except_star.py", "except* ", &["Boom"]);
    assert_exception_references(&indexed, "except_star.py", "except* Boom, ", &["Bust"]);
    assert_exception_references(&indexed, "except_star.py", "except* ", &["Crash"]);
}

#[test]
fn a_clause_with_two_names_gives_two_references_and_no_binding() {
    // R-1 gap check: the grammar reads `except A, B:` as the Python 2 form `except A, name:`
    // and gives no error, so the core never calls the repair. The binding rules of the fork
    // have no rule for that form, so `B` is an expression and a reference. This is the right
    // result for Python 3.14. This test pins it. If the fork ever binds `B`, this test fails.
    let indexed = index("py314");
    assert!(indexed.document("except_two.py").is_some());
    assert_exception_references(&indexed, "except_two.py", "except ", &["Boom"]);
    assert_exception_references(&indexed, "except_two.py", "except Boom, ", &["Bust"]);
    // No definition occurrence in the function, and no repair for this file.
    let document = indexed.document("except_two.py").unwrap();
    let definitions: Vec<_> = document
        .occurrences
        .iter()
        .filter(|o| is_definition(o))
        .map(|o| o.symbol.as_str())
        .collect();
    assert_eq!(
        definitions,
        [
            global("except_two/").as_str(),
            global("except_two/run().").as_str()
        ]
    );
    assert!(!repaired_files(&indexed).contains(&"except_two.py"));
}

#[test]
fn a_template_string_is_read_as_an_f_string() {
    // R-2: the grammar 0.23.5 has no t-string.
    let indexed = index("py314");
    assert!(indexed.document("tstring.py").is_some());
    let name = global("tstring/name.");
    let width = global("tstring/width.");
    let at = |context, word| indexed.symbols_of("tstring.py", context, word);
    assert_eq!(at("t\"hello {name", "name"), [name.as_str()]);
    // A raw template, a call, a conversion, a nested field in the format specification, and `=`.
    assert_eq!(at("{render", "render"), [global("base/render().").as_str()]);
    assert_eq!(at("render(name", "name"), [name.as_str()]);
    assert_eq!(at(">{width", "width"), [width.as_str()]);
    assert_eq!(at("{width=", "width"), [width.as_str()]);
    // A template inside an f-string, and an f-string inside a template.
    assert_eq!(at("f\"inner {name", "name"), [name.as_str()]);
    assert_eq!(at("t\"deep {width", "width"), [width.as_str()]);
    // The prefix of the source stays.
    let text = fs::read_to_string(fixture("py314").join("tstring.py")).unwrap();
    assert!(text.contains("plain = t\"hello"));
}

#[test]
fn a_type_parameter_default_is_blanked_and_reported() {
    // R-3: no grammar version knows a default. The repair blanks it. The type parameter, its
    // bound, and every use of it stay. The names inside the default get no occurrence.
    let indexed = index("py314");
    assert!(indexed.document("typeparams.py").is_some());
    let at = |context, word| indexed.find("typeparams.py", context, word);
    let declared = at("class Box[T", "T");
    assert_eq!(declared.len(), 1);
    assert!(is_definition(declared[0]));
    let own = declared[0].symbol.clone();
    assert!(
        own.starts_with("local "),
        "a type parameter is local: {own}"
    );
    assert_eq!(at("item: T", "T")[0].symbol, own);
    assert_eq!(at("-> T:\n        return self", "T")[0].symbol, own);
    // The bound is a reference. The default is not read.
    assert_eq!(at("T: Bust", "Bust")[0].symbol, global("base/Bust#"));
    assert!(at("class Box[T = Boom", "Boom").is_empty());
    assert!(at("Bust = Boom", "Boom").is_empty());
    assert!(at("type Pair[T = Boom", "Boom").is_empty());
    // The alias and the function are declared.
    assert!(is_definition(at("type Pair", "Pair")[0]));
    assert_eq!(
        at("type Pair", "Pair")[0].symbol,
        global("typeparams/Pair#")
    );
    assert!(is_definition(at("def first", "first")[0]));
    // The report says that the pack blanked syntax.
    let diagnostic = indexed
        .report
        .diagnostics
        .iter()
        .find(|d| d.code == "repaired-syntax" && d.file.as_deref() == Some("typeparams.py"))
        .expect("a repaired-syntax diagnostic");
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert!(
        diagnostic.message.contains("blanked"),
        "{}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains("no occurrence"),
        "{}",
        diagnostic.message
    );
    // The source file keeps its defaults.
    let text = fs::read_to_string(fixture("py314").join("typeparams.py")).unwrap();
    assert!(text.contains("class Box[T = Boom]:"));
}

#[test]
fn each_repair_gives_one_diagnostic_and_a_clean_file_gives_none() {
    let indexed = index("py314");
    assert_eq!(
        repaired_files(&indexed),
        [
            "except_many.py",
            "except_star.py",
            "tstring.py",
            "typeparams.py"
        ]
    );
}

#[test]
fn a_repair_does_not_hide_a_real_syntax_error() {
    // R-1, R-2, R-3: a file that still fails after the repair is skipped, as before.
    let (_scratch, indexed) = index_files(&[
        (
            "a.py",
            "try:\n    pass\nexcept A, B, C:\n    pass\ndef broken(:\n    pass\n",
        ),
        // Python 3.14 rejects `as` without brackets, so the repair leaves the clause alone.
        ("b.py", "try:\n    pass\nexcept A, B, C as e:\n    pass\n"),
        ("good.py", "x = 1\n"),
    ]);
    assert!(indexed.document("a.py").is_none());
    assert!(indexed.document("b.py").is_none());
    assert!(indexed.document("good.py").is_some());
    assert_eq!(indexed.report.files_skipped, 2);
    assert!(
        !indexed
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "repaired-syntax")
    );
}

// ---- packages, roots, cycles ----

#[test]
fn packages_resolve_across_roots_and_cycles_end() {
    let indexed = shared("packages");
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
fn the_manifest_names_the_package_and_its_folder_sets_the_import_root() {
    let indexed = shared("src_layout");
    // `src/demo/ns` has no `__init__.py`. The package folder `src/demo` makes it a namespace
    // package below the import root `src`.
    let symbols = indexed.symbols_of("src/demo/main.py", "f()", "f");
    assert_eq!(symbols, ["scippy python Demo 2.0 src/demo/ns/mod/f()."]);
}

#[test]
fn a_package_shadows_a_module_of_the_same_name() {
    let indexed = index("collision");
    // `a/__init__.py` wins, as in the Python import system. `a.py` is shadowed. The run succeeds.
    assert!(indexed.document("a.py").is_none());
    assert!(indexed.document("a/__init__.py").is_some());
    assert_eq!(indexed.report.files_skipped, 0);
    assert_eq!(indexed.report.files_shadowed, 1);
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
    assert_eq!(indexed.report.files_skipped, 0);
    assert_eq!(indexed.report.files_shadowed, 1);
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
    copy_tree(&fixture("packages"), copy.path());
    let normalised = |mut indexed: Cached| {
        let metadata = indexed.index.metadata.as_mut().unwrap();
        metadata.project_root.clear();
        metadata.tool_info.as_mut().unwrap().version.clear();
        indexed.index.write_to_bytes().unwrap()
    };
    let here = shared("packages");
    let there = index_at(copy.path());
    assert_ne!(
        here.index.metadata.project_root,
        there.index.metadata.project_root
    );
    assert_eq!(normalised(here), normalised(there.view));
}

// ---- the official scip CLI ----

const SCIP_FIXTURES: &[&str] = &[
    "alias",
    "stubs",
    "params",
    "assign",
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

#[test]
fn the_lint_tolerance_accepts_only_a_defined_relationship_target() {
    let indexed = shared("overrides_later");
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
#[ignore = "slow: runs in opt:slow"]
fn every_relationship_target_has_symbol_information() {
    for name in SCIP_FIXTURES {
        let indexed = shared(name);
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
#[ignore = "slow: runs in opt:slow"]
fn scip_lint_accepts_every_fixture_index() {
    let Some(scip) = scip() else { return };
    for name in SCIP_FIXTURES {
        let indexed = shared(name);
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
#[ignore = "slow: runs in opt:slow"]
fn scip_test_accepts_the_annotated_sources() {
    let Some(scip) = scip() else { return };
    for name in SCIP_FIXTURES {
        let indexed = shared(name);
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

#[test]
#[ignore = "slow: runs in opt:slow"]
fn scip_snapshot_matches_the_golden_files() {
    let Some(scip) = scip() else { return };
    let scratch = tempfile::tempdir().unwrap();
    for (name, files) in [
        ("alias", &["src/services.py", "src/app.py"][..]),
        ("ambiguous", &["c.py"][..]),
        ("shadow", &["outer.py", "other.py"][..]),
        ("unicode", &["lib.py", "use.py"][..]),
    ] {
        let indexed = shared(name);
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

// ---- the work counts of the searches ----

fn search_work_of(name: &str, jobs: usize) -> scippy_core::SearchWork {
    let mut request = IndexRequest::new(fixture(name), "");
    request.jobs = jobs;
    index_with(request).report.search_work
}

#[test]
fn the_search_work_counts_are_present_consistent_and_the_same_for_each_run_and_job_count() {
    // The `packages` fixture has several shards, so `--jobs` changes the order of the searches.
    let one = search_work_of("packages", 1);
    let again = search_work_of("packages", 1);
    let four = search_work_of("packages", 4);
    assert_eq!(one, again, "two runs differ");
    assert_eq!(one, four, "the job count changes the counts");
    assert!(one.searches > 0);
    assert!(one.work_total > 0);
    assert!(one.work_max > 0);
    assert!(one.queue_length_max > 0);
    assert!(one.arena_cells_max > 0);
    assert!(one.work_max <= one.work_total);
    // The searches add up: the largest search is at least the mean.
    assert!(one.work_max * one.searches >= one.work_total);
    // One search runs for each reference node, and a location holds one or more nodes.
    let report = index("packages").report;
    let locations = report.references_resolved
        + report.references_unresolved
        + report.references_ambiguous
        + report.references_bounded;
    assert!(one.searches >= locations, "{} < {locations}", one.searches);
}

#[test]
fn a_project_with_no_reference_has_zero_search_work() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let work = index_at(dir.path()).report.search_work;
    assert_eq!(work, scippy_core::SearchWork::default());
}

#[test]
fn a_smaller_work_limit_gives_smaller_search_work_counts() {
    let free = search_work_of("alias", 1);
    let mut request = IndexRequest::new(fixture("alias"), "");
    request.resolve_limits.max_work = 1;
    let capped = index_with(request).report.search_work;
    assert_eq!(capped.searches, free.searches);
    assert!(capped.work_total < free.work_total, "{capped:?} {free:?}");
    assert!(capped.work_max < free.work_max, "{capped:?} {free:?}");
}

// ---- the search explosion of an annotated factory chain ----

/// The number of uses in `annotated_chain/tests/probe.py`.
const CHAIN_USES: usize = 20;

#[test]
#[ignore = "perf: runs in opt:slow"]
fn an_annotated_factory_chain_bounds_no_reference_and_each_member_resolves() {
    // The call of a function with `-> T` has one route for its value: the annotation.
    // Before the fork pin 3240d66, it had two: the annotation and each returned expression.
    // A chain of six factories then made each member reference run to the search limit.
    // The old pin gives 20 arena-bounded references here, and the run takes about 30 s.
    // The count of bounded references is deterministic. This test does not read the clock.
    let indexed = index("annotated_chain");
    assert_eq!(indexed.report.files_skipped, 0);
    assert_eq!(indexed.report.references_bounded, 0);
    assert_eq!(indexed.report.bounded_by_kind.queue_length, 0);
    assert_eq!(indexed.report.bounded_by_kind.arena_cells, 0);
    assert_eq!(indexed.report.bounded_by_kind.work, 0);
    assert_eq!(indexed.report.references_unresolved, 0);
    // The search work counts hold no time, so they bound the cost without a clock.
    // Measured with the fork pin 27e2435: 133 searches, total work 106,270, largest search
    // 2,259. The bounds leave about 25% of room for a small change of the rules. The old pin
    // cda555a gives total work 18,091,672 and a largest search of 663,650, so both bounds fail.
    let work = indexed.report.search_work;
    assert!(
        work.work_total <= 135_000,
        "total search work {} is over 135,000 (measured 106,270)",
        work.work_total
    );
    assert!(
        work.work_max <= 2_800,
        "the largest search work {} is over 2,800 (measured 2,259)",
        work.work_max
    );

    let probe = "tests/probe.py";
    for use_ in 0..CHAIN_USES {
        let context = format!("def use{use_}(d):\n    return wrap().resolve(d).node");
        for (word, target) in [
            ("wrap", "tests/probe/wrap()."),
            ("resolve", "src/pkg/_core/Address#resolve()."),
            ("node", "src/pkg/_core/Resolution#node."),
        ] {
            let found = indexed.find(probe, &context, word);
            assert_eq!(found.len(), 1, "use{use_}: {word}");
            assert_eq!(found[0].symbol, global(target), "use{use_}: {word}");
        }
    }
}
