//! End-to-end tests of the TypeScript pack through the whole pipeline.
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
use scippy_core::{IndexRequest, InputLimits, Severity};
use scippy_typescript::TypeScriptPack;

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

const PREFIX: &str = "scippy npm workspace . ";

fn index_at(root: &Path) -> Indexed {
    harness::index_at(&TypeScriptPack, root)
}

fn index_with(request: IndexRequest) -> Indexed {
    harness::index_with(&TypeScriptPack, request)
}

fn index(name: &str) -> Indexed {
    index_at(&fixture(name))
}

/// The index of a fixture, shared by all test processes. See [`harness::shared`].
fn shared(name: &str) -> Cached {
    harness::shared("e2e-typescript", &TypeScriptPack, name)
}

fn is_import(o: &Occurrence) -> bool {
    o.symbol_roles & SymbolRole::Import as i32 != 0
}

/// A symbol of the default package: the file, then the descriptors below the module.
fn sym(file: &str, tail: &str) -> String {
    format!("{PREFIX}`{file}`/{tail}")
}

// ---- the acceptance fixture of the concept document, section 3.2 ----

#[test]
fn an_aliased_import_gives_one_symbol_to_the_definition_the_import_and_the_call() {
    let indexed = index("alias");
    let process = sym("src/services.ts", "process().");

    let definition = indexed.find("src/services.ts", "function process", "process");
    assert_eq!(definition.len(), 1);
    assert_eq!(definition[0].symbol, process);
    assert!(is_definition(definition[0]));

    // The import occurrence: `process` in `import { process as run } from "./services.js"`.
    let import = indexed.find("src/app.ts", "{ process as", "process");
    assert_eq!(import.len(), 1);
    assert_eq!(import[0].symbol, process);
    assert!(is_import(import[0]));
    assert!(!is_definition(import[0]));

    // The alias name names the same symbol. The call site too.
    let alias = indexed.find("src/app.ts", "as run", "run");
    assert_eq!(alias.len(), 1);
    assert_eq!(alias[0].symbol, process);
    assert!(is_import(alias[0]));
    let call = indexed.find("src/app.ts", "= run(", "run");
    assert_eq!(call.len(), 1);
    assert_eq!(call[0].symbol, process);
    assert_eq!(call[0].symbol_roles, 0, "a plain reference");

    // The module specifier names the module of the file, with the `.js` extension in the import.
    assert_eq!(
        indexed.symbols_of("src/app.ts", "from \"./services.js\"", "\"./services.js\""),
        [sym("src/services.ts", "")]
    );

    // The symbol information belongs to services.ts.
    let services = indexed.document("src/services.ts").unwrap();
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
    assert_eq!(indexed.report.references_ambiguous, 0);
    assert_eq!(indexed.report.files_skipped, 0);
}

// ---- shadowing, unresolved, ambiguous ----

#[test]
fn a_shadowing_name_never_binds_to_the_outer_definition() {
    let indexed = shared("shadow");
    let outer = sym("outer.ts", "process().");
    assert_eq!(
        indexed.symbols_of("other.ts", "second = process(", "process"),
        [sym("other.ts", "process().")]
    );
    assert!(!indexed.has_symbol("other.ts", &outer));
    assert_eq!(
        indexed.symbols_of("outer.ts", "first = process(", "process"),
        [outer.as_str()]
    );
    // A local constant shadows the function. Both uses name the local, never the function.
    let local = indexed.symbols_of("outer.ts", "return process;", "process");
    assert_eq!(local.len(), 1);
    assert!(local[0].starts_with("local "), "{local:?}");
    let declaration = indexed.symbols_of("outer.ts", "const process", "process");
    assert_eq!(declaration, local);
}

#[test]
fn unresolved_names_get_no_target() {
    let indexed = index("unresolved");
    // A missing module, a node module, and an undeclared name get no occurrence.
    // The decoy exports the same names. The importer never reaches it.
    for (context, word) in [
        ("import { nothing }", "nothing"),
        ("import { readFile }", "readFile"),
        ("\nnothing();", "nothing"),
        ("\nreadFile();", "readFile"),
        ("undeclared();", "undeclared"),
        ("\"./nowhere\"", "\"./nowhere\""),
        ("\"node:fs\"", "\"node:fs\""),
    ] {
        assert!(
            indexed.find("missing.ts", context, word).is_empty(),
            "{context}"
        );
    }
    assert!(!indexed.has_symbol("missing.ts", &sym("decoy.ts", "nothing().")));
    assert!(indexed.report.references_unresolved >= 5);
    assert_eq!(indexed.report.references_external, 0);
}

#[test]
fn ambiguous_names_get_no_target() {
    // Two script files declare `dup`. A third file calls it.
    let indexed = index("ambiguous");
    assert!(indexed.find("c.ts", "dup()", "dup").is_empty());
    assert_eq!(indexed.report.references_ambiguous, 1);
    // The declarations have distinct symbols, because the module is the file.
    assert!(indexed.has_symbol("a.ts", &sym("a.ts", "dup().")));
    assert!(indexed.has_symbol("b.ts", &sym("b.ts", "dup().")));
}

// ---- module forms ----

#[test]
fn imports_resolve_with_js_extensions_directories_and_names_that_end_in_ts_or_js() {
    let indexed = shared("js_extension");
    let answer = sym("lib/util.ts", "answer.");
    let helper = sym("lib/index.ts", "helper().");
    for context in ["{ answer } from", "{ answer as again }"] {
        assert_eq!(
            indexed.symbols_of("main.ts", context, "answer"),
            [answer.as_str()]
        );
    }
    assert_eq!(
        indexed.symbols_of("main.ts", "{ helper } from", "helper"),
        [helper.as_str()]
    );
    // The module specifiers: the same module, with or without the extension.
    assert_eq!(
        indexed.symbols_of("main.ts", "\"./lib/index.js\"", "\"./lib/index.js\""),
        [sym("lib/index.ts", "")]
    );
    assert_eq!(
        indexed.symbols_of("main.ts", "\"./lib\"", "\"./lib\""),
        [sym("lib/index.ts", "")]
    );
    // The names `constants` and `widgets` end in `ts`, and `jsx` ends in `jsx`. No character is lost.
    assert_eq!(
        indexed.symbols_of("ends.ts", "{ limits } from", "limits"),
        [sym("lib/constants.ts", "limits.")]
    );
    assert_eq!(
        indexed.symbols_of("ends.ts", "{ jsx } from", "jsx"),
        [sym("lib/widgets.ts", "jsx.")]
    );
}

#[test]
fn barrels_pass_a_name_through_export_star_and_renamed_exports() {
    let indexed = shared("barrels");
    let alpha = sym("core/a.ts", "alpha.");
    let beta = sym("core/a.ts", "beta().");
    assert_eq!(
        indexed.symbols_of("use.ts", "{ alpha,", "alpha"),
        [alpha.as_str()]
    );
    // `export { beta as gamma }` renames. The import of `gamma` reaches `beta`.
    assert_eq!(
        indexed.symbols_of("use.ts", ", gamma,", "gamma"),
        [beta.as_str()]
    );
    assert_eq!(
        indexed.symbols_of("use.ts", "gamma()", "gamma"),
        [beta.as_str()]
    );
    // `export * as core` makes a namespace. `core.beta()` reaches the function.
    assert_eq!(
        indexed.symbols_of("use.ts", "core.beta", "beta"),
        [beta.as_str()]
    );
    assert_eq!(
        indexed.symbols_of("use.ts", "[alpha, gamma(), core", "core"),
        [sym("core/index.ts", "")]
    );
}

#[test]
fn default_exports_resolve_to_the_exported_declaration() {
    let indexed = shared("defaults");
    assert_eq!(
        indexed.symbols_of("use.ts", "import Widget", "Widget"),
        [sym("widget.ts", "Widget#")]
    );
    assert_eq!(
        indexed.symbols_of("use.ts", "new Widget", "Widget"),
        [sym("widget.ts", "Widget#")]
    );
    // `export default config` names `config`. `export default { ... }` declares `default`.
    assert_eq!(
        indexed.symbols_of("use.ts", "const c = cfg", "cfg"),
        [sym("config.ts", "config.")]
    );
    assert_eq!(
        indexed.symbols_of("use.ts", "const l = lit", "lit"),
        [sym("literal.ts", "default.")]
    );
    // The import statement itself has no occurrence that covers its whole text.
    let document = indexed.document("use.ts").unwrap();
    assert!(
        document.occurrences.iter().all(|o| matches!(
            &o.typed_range,
            Some(scip::types::occurrence::Typed_range::SingleLineRange(r)) if r.end_character - r.start_character < 30
        )),
        "an occurrence covers a whole statement"
    );
    // Known gap of the binding rules: a method call on `new Widget()` of a default import.
    assert!(
        indexed
            .find("use.ts", "new Widget().render", "render")
            .is_empty()
    );
}

#[test]
fn a_namespace_import_names_the_module() {
    let indexed = shared("namespace");
    assert_eq!(
        indexed.symbols_of("use.ts", "* as geo", "geo"),
        [sym("geo.ts", "")]
    );
    assert_eq!(
        indexed.symbols_of("use.ts", "geo.area", "area"),
        [sym("geo.ts", "area().")]
    );
    assert_eq!(
        indexed.symbols_of("use.ts", "[geo.area", "geo"),
        [sym("geo.ts", "")]
    );
}

#[test]
fn a_type_only_import_resolves_types() {
    let indexed = shared("type_only");
    assert_eq!(
        indexed.symbols_of("use.ts", "import type { Shape", "Shape"),
        [sym("types.ts", "Shape#")]
    );
    assert_eq!(
        indexed.symbols_of("use.ts", "{ type Impl }", "Impl"),
        [sym("types.ts", "Impl#")]
    );
    assert_eq!(
        indexed.symbols_of("use.ts", "s.area", "area"),
        [sym("types.ts", "Shape#area().")]
    );
}

// ---- members ----

#[test]
fn class_members_resolve_through_this_and_through_a_value() {
    let indexed = shared("members");
    let count = sym("counter.ts", "Counter#count.");
    let step = sym("counter.ts", "Counter#step.");
    // A field.
    assert_eq!(
        indexed.symbols_of("counter.ts", "this.count = step", "count"),
        [count.as_str()]
    );
    // A parameter property is a field of the class. The constructor sees it as a bare name.
    assert_eq!(
        indexed.symbols_of("counter.ts", "(private step", "step"),
        [step.as_str()]
    );
    assert_eq!(
        indexed.symbols_of("counter.ts", "= step;", "step"),
        [step.as_str()]
    );
    assert_eq!(
        indexed.symbols_of("counter.ts", "this.step +=", "step"),
        [step.as_str()]
    );
    // A static member.
    assert_eq!(
        indexed.symbols_of("counter.ts", "Counter.created +=", "created"),
        [sym("counter.ts", "Counter#created.")]
    );
    // A call through a value of the class, an accessor.
    assert_eq!(
        indexed.symbols_of("use.ts", "ctr.inc", "inc"),
        [sym("counter.ts", "Counter#inc().")]
    );
    assert_eq!(
        indexed.symbols_of("use.ts", "ctr.value", "value"),
        [sym("counter.ts", "Counter#value().")]
    );
}

#[test]
fn enums_and_namespaces_own_their_members() {
    let indexed = shared("members");
    assert_eq!(
        indexed.symbols_of("enums.ts", "Color.Green", "Green"),
        [sym("enums.ts", "Color#Green.")]
    );
    assert_eq!(
        indexed.symbols_of("enums.ts", "return PI", "PI"),
        [sym("enums.ts", "Geo/PI.")]
    );
}

#[test]
fn a_bare_name_never_names_a_class_member() {
    let indexed = shared("members");
    // `return area` in the method `area` of `Box` names the module constant.
    assert_eq!(
        indexed.symbols_of("bare.ts", "return area", "area"),
        [sym("bare.ts", "area.")]
    );
}

#[test]
fn an_implementing_method_gets_a_relationship_to_the_interface_method() {
    let indexed = shared("implements");
    let shapes = indexed.document("shapes.ts").unwrap();
    let relationships = |symbol: &str| -> Vec<(String, bool)> {
        shapes
            .symbols
            .iter()
            .find(|s| s.symbol == symbol)
            .unwrap()
            .relationships
            .iter()
            .map(|r| (r.symbol.clone(), r.is_implementation))
            .collect()
    };
    assert_eq!(
        relationships(&sym("shapes.ts", "Square#area().")),
        [(sym("shapes.ts", "Shape#area()."), true)]
    );
    assert_eq!(
        relationships(&sym("shapes.ts", "Square#describe().")),
        [(sym("shapes.ts", "Base#describe()."), true)]
    );
    // The classes themselves have none.
    assert!(relationships(&sym("shapes.ts", "Square#")).is_empty());
}

// ---- TSX ----

#[test]
fn tsx_and_ts_files_import_each_other() {
    let indexed = index("tsx");
    let button = sym("Button.tsx", "Button().");
    assert_eq!(
        indexed.symbols_of("app.tsx", "<Button", "Button"),
        [button.as_str()]
    );
    // A .ts file imports a .tsx file.
    assert_eq!(
        indexed.symbols_of("button.ts", "= Button", "Button"),
        [button.as_str()]
    );
    // A .tsx file imports a .ts file.
    assert_eq!(
        indexed.symbols_of("Button.tsx", "{shout(", "shout"),
        [sym("text.ts", "shout().")]
    );
    // A lowercase tag is an element of the platform. It never names the variable `button`.
    assert!(indexed.find("Button.tsx", "<button>", "button").is_empty());
    assert!(indexed.find("Button.tsx", "</button>", "button").is_empty());
    assert_eq!(indexed.report.files_indexed, 4);
}

// ---- syntax errors, repair, limits ----

#[test]
fn a_syntax_error_skips_one_file_and_the_others_still_index() {
    let indexed = index("syntax_error");
    assert!(indexed.document("bad.ts").is_none());
    assert_eq!(indexed.report.files_skipped, 1);
    let diagnostic = indexed
        .report
        .diagnostics
        .iter()
        .find(|d| d.code == "syntax-error")
        .expect("a syntax diagnostic");
    assert_eq!(diagnostic.file.as_deref(), Some("bad.ts"));
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(
        indexed.symbols_of("user.ts", "x = fine()", "fine"),
        [sym("good.ts", "fine().")]
    );
}

#[test]
fn syntax_that_the_grammar_rejects_is_repaired_and_reported() {
    let indexed = index("repair");
    assert_eq!(indexed.report.files_skipped, 0);
    assert_eq!(indexed.report.files_indexed, 3);
    let repaired: Vec<_> = indexed
        .report
        .diagnostics
        .iter()
        .filter(|d| d.code == "repaired-syntax")
        .filter_map(|d| d.file.as_deref())
        .collect();
    assert_eq!(repaired, ["barrel.ts", "types.ts"]);
    // The declarations of the repaired file resolve from another file.
    assert_eq!(
        indexed.symbols_of("use.ts", "Reader<number>", "Reader"),
        [sym("types.ts", "Reader#")]
    );
    assert_eq!(
        indexed.symbols_of("use.ts", "= call(1)", "call"),
        [sym("types.ts", "call.")]
    );
    // The text of the file is not changed.
    let text = fs::read_to_string(fixture("repair").join("types.ts")).unwrap();
    assert!(text.contains("Reader<out T>"));
}

#[test]
fn a_file_over_the_limit_is_skipped() {
    let mut request = IndexRequest::new(fixture("alias"), "");
    request.limits = InputLimits {
        max_file_bytes: 400,
        max_total_bytes: 1 << 20,
    };
    let indexed = index_with(request);
    assert!(
        indexed
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "file-too-large" && d.file.as_deref() == Some("src/app.ts"))
    );
    assert!(indexed.document("src/app.ts").is_none());
    assert!(indexed.document("src/services.ts").is_some());
}

#[test]
fn crlf_line_endings_keep_the_columns() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("a.ts"),
        "export function f(): number {\r\n  return 1;\r\n}\r\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.ts"),
        "import { f } from \"./a\";\r\n\r\nexport const x = f();\r\n",
    )
    .unwrap();
    let indexed = index_at(dir.path());
    assert_eq!(indexed.symbols_at("b.ts", 3, 17), [sym("a.ts", "f().")]);
    assert_eq!(indexed.symbols_at("b.ts", 1, 9), [sym("a.ts", "f().")]);
}

#[test]
fn import_cycles_end() {
    let indexed = shared("cycle");
    assert_eq!(
        indexed.symbols_of("a.ts", "return b()", "b"),
        [sym("b.ts", "b().")]
    );
    assert_eq!(
        indexed.symbols_of("b.ts", "return a()", "a"),
        [sym("a.ts", "a().")]
    );
}

#[test]
fn utf8_byte_columns_follow_non_ascii_text() {
    let indexed = shared("unicode");
    // `export const naïve = 日本;`: `日本` starts after `ï`, which has two bytes.
    let (line, column) = indexed.locate("use.ts", "naïve = 日本", "日本");
    assert_eq!(
        indexed.symbols_at("use.ts", line, column),
        [sym("lib.ts", "`日本`.")]
    );
    assert_eq!(column, "export const naïve = ".len() as i32);
}

// ---- package.json and tsconfig.json ----

#[test]
fn the_package_and_the_path_mappings_come_from_the_project_files() {
    let indexed = index("config");
    let prefix = "scippy npm @acme/app 1.2.3 ";
    let at = |context: &str, word: &str| -> Vec<String> {
        indexed
            .find("src/app/main.ts", context, word)
            .iter()
            .map(|o| o.symbol.clone())
            .collect()
    };
    // `paths` with a wildcard, `paths` without one, `baseUrl`, and the package name of the project.
    assert_eq!(
        at("{ feather } from \"@lib", "feather"),
        [format!("{prefix}`src/lib/f.ts`/feather().")]
    );
    assert_eq!(
        at("{ cfg }", "cfg"),
        [format!("{prefix}`src/shared/cfg.ts`/cfg.")]
    );
    assert_eq!(
        at("{ gadget }", "gadget"),
        [format!("{prefix}`src/shared/g.ts`/gadget.")]
    );
    assert_eq!(
        at("{ feather as self }", "feather"),
        [format!("{prefix}`src/lib/f.ts`/feather().")]
    );
    assert!(
        indexed
            .report
            .diagnostics
            .iter()
            .all(|d| d.code != "support-file-error"),
        "{:?}",
        indexed.report.diagnostics
    );
}

#[test]
fn a_request_can_replace_the_package_name() {
    let mut request = IndexRequest::new(fixture("alias"), "");
    request.package_name = Some("renamed".into());
    request.package_version = Some("9.9".into());
    let indexed = index_with(request);
    assert!(indexed.has_symbol(
        "src/services.ts",
        "scippy npm renamed 9.9 `src/services.ts`/process()."
    ));
}

#[test]
fn every_document_defines_its_module_at_the_start() {
    let indexed = shared("alias");
    for (path, module) in [
        ("src/app.ts", "src/app.ts"),
        ("src/services.ts", "src/services.ts"),
    ] {
        let at = indexed.at(path, 1, 0);
        let definition = at
            .iter()
            .find(|o| is_definition(o) && o.symbol == sym(module, ""))
            .unwrap_or_else(|| panic!("module of {path}"));
        assert_eq!(definition.symbol_roles, SymbolRole::Definition as i32);
    }
}

#[test]
fn same_names_in_two_files_get_distinct_symbols() {
    let indexed = shared("shadow");
    assert_ne!(sym("outer.ts", "process()."), sym("other.ts", "process()."));
    assert!(indexed.has_symbol("outer.ts", &sym("outer.ts", "process().")));
    assert!(indexed.has_symbol("other.ts", &sym("other.ts", "process().")));
}

// ---- determinism ----

#[test]
fn another_checkout_root_changes_only_the_root_uri() {
    let copy = tempfile::tempdir().unwrap();
    copy_tree(&fixture("barrels"), copy.path());
    let normalised = |mut indexed: Cached| {
        let metadata = indexed.index.metadata.as_mut().unwrap();
        metadata.project_root.clear();
        metadata.tool_info.as_mut().unwrap().version.clear();
        indexed.index.write_to_bytes().unwrap()
    };
    let here = shared("barrels");
    let there = index_at(copy.path());
    assert_ne!(
        here.index.metadata.project_root,
        there.index.metadata.project_root
    );
    assert_eq!(normalised(here), normalised(there.view));
}

#[test]
fn the_pack_leaves_python_files_alone() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    fs::write(dir.path().join("b.js"), "export const y = 1;\n").unwrap();
    fs::write(dir.path().join("c.ts"), "export const z = 1;\n").unwrap();
    let indexed = index_at(dir.path());
    assert_eq!(indexed.report.files_indexed, 1);
    assert!(indexed.document("c.ts").is_some());
}

// ---- the official scip CLI ----

const SCIP_FIXTURES: &[&str] = &[
    "alias",
    "shadow",
    "unresolved",
    "ambiguous",
    "js_extension",
    "barrels",
    "defaults",
    "namespace",
    "type_only",
    "members",
    "implements",
    "tsx",
    "syntax_error",
    "unicode",
    "repair",
    "config",
    "cycle",
];

#[test]
fn the_lint_tolerance_accepts_only_a_defined_relationship_target() {
    let indexed = shared("implements");
    let target = sym("shapes.ts", "Shape#area().");
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
            "//",
        ];
        // The syntax-error fixture has a file that is not in the index on purpose.
        // The config fixture has JSON files.
        if *name != "syntax_error" && *name != "config" {
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
        ("alias", &["src/services.ts", "src/app.ts"][..]),
        ("barrels", &["core/index.ts", "index.ts", "use.ts"][..]),
        ("defaults", &["use.ts"][..]),
        ("shadow", &["outer.ts", "other.ts"][..]),
        ("tsx", &["Button.tsx", "app.tsx"][..]),
        ("unicode", &["lib.ts", "use.ts"][..]),
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
                "//",
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
