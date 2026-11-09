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
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use protobuf::Message;
use scip::types::{Index, Occurrence, SymbolRole};
use scippy_core::{IndexReport, IndexRequest, InputLimits, Severity, index_project};
use scippy_typescript::TypeScriptPack;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");
const PREFIX: &str = "scippy npm workspace . ";

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
    let report = index_project(&request, &[&TypeScriptPack]).unwrap();
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

fn is_definition(o: &Occurrence) -> bool {
    o.symbol_roles & SymbolRole::Definition as i32 != 0
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
    let indexed = index("shadow");
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
    let indexed = index("js_extension");
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
    let indexed = index("barrels");
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
    let indexed = index("defaults");
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
    let indexed = index("namespace");
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
    let indexed = index("type_only");
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
    let indexed = index("members");
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
    let indexed = index("members");
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
    let indexed = index("members");
    // `return area` in the method `area` of `Box` names the module constant.
    assert_eq!(
        indexed.symbols_of("bare.ts", "return area", "area"),
        [sym("bare.ts", "area.")]
    );
}

#[test]
fn an_implementing_method_gets_a_relationship_to_the_interface_method() {
    let indexed = index("implements");
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
    let indexed = index("cycle");
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
    let indexed = index("unicode");
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
    let indexed = index("alias");
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
    let indexed = index("shadow");
    assert_ne!(sym("outer.ts", "process()."), sym("other.ts", "process()."));
    assert!(indexed.has_symbol("outer.ts", &sym("outer.ts", "process().")));
    assert!(indexed.has_symbol("other.ts", &sym("other.ts", "process().")));
}

// ---- determinism ----

#[test]
fn another_checkout_root_changes_only_the_root_uri() {
    let copy = tempfile::tempdir().unwrap();
    copy_dir(&fixture("barrels"), copy.path());
    let normalised = |mut indexed: Indexed| {
        let metadata = indexed.index.metadata.as_mut().unwrap();
        metadata.project_root.clear();
        metadata.tool_info.as_mut().unwrap().version.clear();
        indexed.index.write_to_bytes().unwrap()
    };
    let here = index("barrels");
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
    let indexed = index("implements");
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
        ("alias", &["src/services.ts", "src/app.ts"][..]),
        ("barrels", &["core/index.ts", "index.ts", "use.ts"][..]),
        ("defaults", &["use.ts"][..]),
        ("shadow", &["outer.ts", "other.ts"][..]),
        ("tsx", &["Button.tsx", "app.tsx"][..]),
        ("unicode", &["lib.ts", "use.ts"][..]),
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
