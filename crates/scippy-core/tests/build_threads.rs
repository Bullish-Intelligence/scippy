//! The threads that build the graphs of a shard change neither the output, nor the report, nor
//! the graph cache. A file that fails to build fails the same way at every thread count.

use std::fs;
use std::path::{Path, PathBuf};

use scippy_core::{
    Analysis, BindingRules, Descriptor, Dialect, ExtractionSpec, Identity, IndexReport,
    IndexRequest, LanguagePack, Manifest, Package, ProjectModel, ProjectRelativePath, Reach, Scope,
    SymbolKind, index_project,
};

/// Each function definition defines its name at the root. Each call refers to a name at the root.
/// A `pass` statement makes a node that the graph loader rejects, so a file with `pass` fails to
/// build.
const TSG: &str = r#"
global FILE_PATH
global ROOT_NODE
(function_definition name: (identifier) @n) {
  node def
  attr (def) type = "pop_symbol", symbol = (source-text @n), is_definition, source_node = @n
  edge ROOT_NODE -> def
}
(call function: (identifier) @n) {
  node ref
  attr (ref) type = "push_symbol", symbol = (source-text @n), is_reference, source_node = @n
  edge ref -> ROOT_NODE
}
(pass_statement) @_p {
  node bad
  attr (bad) type = "push_symbol"
}
"#;

/// The rules of the second dialect do not compile. A file of that dialect fails to build.
const BAD_TSG: &str = "(module) @m { this is not a rule";

struct Model;

impl ProjectModel for Model {
    fn resolver_globals(&self, _: &ProjectRelativePath) -> Vec<(String, String)> {
        Vec::new()
    }
    fn identity(
        &self,
        path: &ProjectRelativePath,
        _: &[Scope],
        d: &Scope,
        _: &[Scope],
    ) -> Identity {
        Identity {
            reach: Reach::Global,
            descriptors: vec![
                Descriptor::Namespace(path.as_str().split('.').next().unwrap().to_string()),
                Descriptor::Term(d.name.clone()),
            ],
            kind: SymbolKind::Variable,
            display_name: d.name.clone(),
        }
    }
}

/// A pack with two dialects. A `.alt` file has the second dialect, with rules that do not compile.
struct Pack;

impl Analysis for Pack {
    fn graph_cache_key(&self) -> Option<String> {
        Some("build-threads-test-v1".to_string())
    }
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }
    fn binding_rules(&self) -> BindingRules {
        BindingRules {
            tsg_path: "good.tsg",
            tsg_source: TSG,
            builtins: None,
            assets: Vec::new(),
            shadow_aware_similar_paths: false,
            prebuilt_assets: None,
        }
    }
    fn extraction_spec(&self) -> ExtractionSpec {
        ExtractionSpec {
            query: "(function_definition name: (identifier) @name) @definition.function",
        }
    }
    fn default_package(&self) -> Package {
        Package {
            manager: "test".into(),
            name: "threads".into(),
            version: "1".into(),
        }
    }
    fn project_model(
        &self,
        _: &Path,
        _: &[ProjectRelativePath],
        _: &[Manifest],
    ) -> Box<dyn ProjectModel> {
        Box::new(Model)
    }
    fn extra_dialects(&self) -> Vec<Dialect> {
        vec![Dialect {
            grammar: tree_sitter_python::LANGUAGE.into(),
            tsg_path: "bad.tsg",
            tsg_source: BAD_TSG,
            query: "",
        }]
    }
    fn dialect_of(&self, path: &ProjectRelativePath) -> usize {
        usize::from(path.as_str().ends_with(".alt"))
    }
    fn repair_syntax(&self, source: &str) -> Option<String> {
        // A test hook: a file with this word makes a worker panic while it parses the file.
        assert!(!source.contains("PANIC"), "the pack panics on purpose");
        None
    }
}

impl LanguagePack for Pack {
    fn id(&self) -> &'static str {
        "threads"
    }
    fn scip_language(&self) -> &'static str {
        "Threads"
    }
    fn accepts(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "tpy" || e == "alt")
    }
    fn analysis(&self) -> Option<&dyn Analysis> {
        Some(self)
    }
}

/// Write a project. The calls cross the files, so the resolver joins them over the root.
/// Some files fail to build: two have a `pass`, two have the second dialect, and one has a
/// syntax error.
fn write_project(root: &Path, files: usize) {
    fs::create_dir_all(root).unwrap();
    for n in 0..files {
        let text = format!(
            "def d{n}():\n    return {n}\n\nd{next}()\nd{n}()\nd{far}()\n",
            next = (n + 1) % files,
            far = (n * 7 + 3) % files
        );
        fs::write(root.join(format!("m{n:02}.tpy")), text).unwrap();
    }
    fs::write(root.join("p1.tpy"), "def p1():\n    pass\n").unwrap();
    fs::write(root.join("p2.tpy"), "def p2():\n    pass\n\nd1()\n").unwrap();
    fs::write(root.join("a1.alt"), "def a1():\n    return 1\n").unwrap();
    fs::write(root.join("a2.alt"), "def a2():\n    return 2\n\nd2()\n").unwrap();
    fs::write(root.join("s1.tpy"), "def (:\n").unwrap();
}

/// Everything that a run must keep the same at every thread count.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    scip: Vec<u8>,
    summary: String,
    diagnostics: Vec<String>,
    lost: Vec<String>,
}

struct Run {
    outcome: Outcome,
    report: IndexReport,
}

fn run(
    root: &Path,
    work: &Path,
    name: &str,
    build_threads: Option<usize>,
    cache: Option<&PathBuf>,
) -> Run {
    let output = work.join(format!("{name}.scip"));
    let mut request = IndexRequest::new(root, &output);
    request.jobs = 4;
    request.resolve_threads = Some(1);
    request.build_threads = build_threads;
    request.graph_cache = cache.cloned();
    let report = index_project(&request, &[&Pack]).unwrap();
    let outcome = Outcome {
        scip: fs::read(&output).unwrap(),
        summary: format!(
            "{}/{}/{}/{}/{}/{}",
            report.files_discovered,
            report.files_indexed,
            report.files_skipped,
            report.references_resolved,
            report.references_unresolved,
            report.declarations
        ),
        diagnostics: report
            .diagnostics
            .iter()
            .map(|d| format!("{d:?}"))
            .collect(),
        lost: report.lost_files.iter().map(|l| format!("{l:?}")).collect(),
    };
    Run { outcome, report }
}

fn project(files: usize) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    write_project(&root, files);
    let work = dir.path().join("work");
    fs::create_dir_all(&work).unwrap();
    (dir, root, work)
}

#[test]
fn the_output_and_the_report_do_not_depend_on_the_build_threads() {
    let (_dir, root, work) = project(40);
    let one = run(&root, &work, "one", Some(1), None);
    // The references resolve, so the comparison has bindings in it.
    assert!(one.report.references_resolved > 40, "{}", one.report);
    for threads in [2, 3, 8, 64] {
        let many = run(&root, &work, &format!("t{threads}"), Some(threads), None);
        assert_eq!(many.outcome, one.outcome, "{threads} build threads");
    }
    let default = run(&root, &work, "default", None, None);
    assert_eq!(default.outcome, one.outcome, "the default build threads");
}

#[test]
fn a_file_that_fails_to_build_fails_the_same_way_at_every_thread_count() {
    let (_dir, root, work) = project(30);
    let serial = run(&root, &work, "serial", Some(1), None);
    let parallel = run(&root, &work, "parallel", Some(8), None);
    let codes = |run: &Run| -> Vec<(String, &'static str, String)> {
        run.report
            .diagnostics
            .iter()
            .map(|d| {
                (
                    d.file.clone().unwrap_or_default(),
                    d.code,
                    d.message.clone(),
                )
            })
            .collect()
    };
    // Five files fail: two on the rules (`pass`), two on the second dialect, one on the syntax.
    let binds: Vec<_> = codes(&serial)
        .into_iter()
        .filter(|(_, code, _)| *code == "bind-error")
        .collect();
    assert_eq!(binds.len(), 4, "{binds:?}");
    assert!(
        binds.iter().any(|(file, _, message)| file == "a1.alt"
            && message.contains("the binding rules do not compile")),
        "{binds:?}"
    );
    assert_eq!(serial.report.files_skipped, 5);
    assert_eq!(codes(&parallel), codes(&serial));
    assert_eq!(parallel.outcome, serial.outcome);
    // No file drops without a diagnostic and a lost-file line.
    assert_eq!(parallel.report.lost_files.len(), 5);
}

#[test]
fn a_panic_in_a_worker_ends_the_run_and_writes_no_file() {
    let (_dir, root, work) = project(20);
    fs::write(root.join("z.tpy"), "def (: PANIC\n").unwrap();
    for threads in [1, 4] {
        let output = work.join(format!("panic{threads}.scip"));
        let mut request = IndexRequest::new(&root, &output);
        request.jobs = 4;
        request.build_threads = Some(threads);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            index_project(&request, &[&Pack])
        }));
        assert!(result.is_err(), "{threads} build threads: the run passed");
        assert!(
            !output.exists(),
            "{threads} build threads: the run wrote a file"
        );
    }
}

#[test]
fn cold_and_warm_cache_runs_equal_a_run_without_the_cache() {
    let (_dir, root, work) = project(40);
    let cache = work.join("graphs.sqlite");
    let clean = run(&root, &work, "clean", Some(1), None);
    let good = clean.report.files_indexed;
    let cold = run(&root, &work, "cold", Some(8), Some(&cache));
    assert_eq!(cold.outcome, clean.outcome, "a cold cache");
    assert_eq!(cold.report.graph_cache_hits, 0);
    assert!(cold.report.graph_cache_misses >= good);
    let warm = run(&root, &work, "warm", Some(8), Some(&cache));
    assert_eq!(warm.outcome, clean.outcome, "a warm cache");
    // Every file that built the first time comes from the cache now.
    assert_eq!(warm.report.graph_cache_hits, good);
    // A serial run reads the rows that the parallel run wrote, and the other way round.
    let serial_warm = run(&root, &work, "serial-warm", Some(1), Some(&cache));
    assert_eq!(
        serial_warm.outcome, clean.outcome,
        "a serial run on a warm cache"
    );
    assert_eq!(serial_warm.report.graph_cache_hits, good);
}

#[test]
fn a_cache_with_some_stale_rows_gives_the_same_output() {
    let (_dir, root, work) = project(40);
    let cache = work.join("graphs.sqlite");
    run(&root, &work, "fill", Some(8), Some(&cache));
    // Change some files: they miss, and the others hit.
    for n in [0, 5, 6, 39] {
        fs::write(
            root.join(format!("m{n:02}.tpy")),
            format!("def d{n}():\n    return 0\n\nd{}()\nd{n}()\n", (n + 2) % 40),
        )
        .unwrap();
    }
    let clean = run(&root, &work, "clean", Some(1), None);
    let mixed = run(&root, &work, "mixed", Some(8), Some(&cache));
    assert_eq!(mixed.outcome, clean.outcome);
    assert!(mixed.report.graph_cache_hits > 0);
    assert!(mixed.report.graph_cache_misses >= 4);
    let again = run(&root, &work, "again", Some(3), Some(&cache));
    assert_eq!(again.outcome, clean.outcome);
    // Only the four files that fail to build miss: the cache never stores them.
    assert_eq!(again.report.graph_cache_misses, 4);
}

/// A project with more files than the window of the lookup, and a head file that is slower to
/// build than all the others. The workers finish the later files first, and the merge waits for
/// the head file. The lookup must stop at the window, and it must still reach the head file.
fn write_slow_head_project(root: &Path, files: usize) {
    write_project(root, files);
    let mut head = String::new();
    for n in 0..1500 {
        head.push_str(&format!("def h{n}():\n    return d0()\n\nh{n}()\n"));
    }
    // A name that sorts before every other file, so this file is the first job.
    fs::write(root.join("0head.tpy"), head).unwrap();
}

#[test]
fn a_slow_head_file_and_more_files_than_the_window_give_the_serial_output() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    // 100 files are more than the window of 2 threads (16 files) and of 8 threads (32 files).
    write_slow_head_project(&root, 100);
    let work = dir.path().join("work");
    fs::create_dir_all(&work).unwrap();
    let cache = work.join("graphs.sqlite");
    let serial = run(&root, &work, "serial", Some(1), None);
    assert!(serial.report.references_resolved > 100, "{}", serial.report);
    for threads in [2, 8] {
        let many = run(&root, &work, &format!("t{threads}"), Some(threads), None);
        assert_eq!(many.outcome, serial.outcome, "{threads} build threads");
    }
    // A cold cache and then a warm cache, where the lookup meets hits behind the window.
    let cold = run(&root, &work, "cold", Some(3), Some(&cache));
    assert_eq!(cold.outcome, serial.outcome, "a cold cache");
    let warm = run(&root, &work, "warm", Some(3), Some(&cache));
    assert_eq!(warm.outcome, serial.outcome, "a warm cache");
    assert_eq!(warm.report.graph_cache_hits, serial.report.files_indexed);
}

/// More files than the hit window of the lookup (`HIT_WINDOW`, 256). The window of the lookup
/// ahead of the merge is `4 * threads` files. It is larger than 256 files only above 64 build
/// threads, so the hit window is the one that binds. A warm cache gives a hit for each good file.
/// The lookup stops at 256 held hits, goes on after each merge, and gives the serial output.
#[test]
fn more_files_than_the_hit_window_give_the_serial_output() {
    let (_dir, root, work) = project(300);
    let cache = work.join("graphs.sqlite");
    let serial = run(&root, &work, "serial", Some(1), None);
    assert!(serial.report.references_resolved > 300, "{}", serial.report);
    // The hits are more than the hit window, so the window binds on a warm cache.
    assert!(serial.report.files_indexed > 256, "{}", serial.report);
    // 100 threads run ahead 400 files. A cold cache misses on all files.
    let cold = run(&root, &work, "cold", Some(100), Some(&cache));
    assert_eq!(cold.outcome, serial.outcome, "a cold cache");
    assert_eq!(cold.report.graph_cache_hits, 0);
    let warm = run(&root, &work, "warm", Some(100), Some(&cache));
    assert_eq!(warm.outcome, serial.outcome, "a warm cache");
    assert_eq!(warm.report.graph_cache_hits, serial.report.files_indexed);
    // A thread count under the window gives the same output on the same cache.
    let few = run(&root, &work, "few", Some(3), Some(&cache));
    assert_eq!(few.outcome, serial.outcome, "3 build threads");
    assert_eq!(few.report.graph_cache_hits, serial.report.files_indexed);
}
