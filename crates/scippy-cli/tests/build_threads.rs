//! `--build-threads` changes neither the index nor the trace, with the graph cache on or off.

mod common;

use std::fs;
use std::path::Path;

/// A project of modules that import each other and use classes, calls, and attributes.
fn write_project(root: &Path, count: usize) {
    fs::create_dir_all(root).unwrap();
    for n in 0..count {
        let other = (n * 5 + 1) % count;
        let text = format!(
            "from m{other} import C{other}, f{other}\n\n\
             def f{n}(x):\n    return C{other}().go(x) + f{other}(x)\n\n\
             class C{n}(C{other}):\n    value = f{n}\n\n    def go(self, y):\n        return self.value(y)\n\n\
             f{n}(1)\nC{n}().go(2)\n"
        );
        fs::write(root.join(format!("m{n}.py")), text).unwrap();
    }
    // A file with a syntax error. The run skips it at every thread count.
    fs::write(root.join("broken.py"), "def (:\n").unwrap();
}

fn run(
    root: &Path,
    work: &Path,
    name: &str,
    build_threads: &str,
    cache: Option<&Path>,
) -> (Vec<u8>, Vec<u8>) {
    let output = work.join(format!("{name}.scip"));
    let trace = work.join(format!("{name}.trace"));
    let mut command = common::scippy();
    command.args(["index", root.to_str().unwrap(), "--output"]);
    command.arg(&output);
    command.arg("--trace").arg(&trace);
    command.args(["--jobs", "4", "--build-threads", build_threads]);
    if let Some(cache) = cache {
        command.arg("--graph-cache").arg(cache);
    }
    let out = command.output().unwrap();
    assert!(
        common::finished(&out),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("warning: the graph cache"));
    (fs::read(&output).unwrap(), fs::read(&trace).unwrap())
}

#[test]
fn the_index_and_the_trace_do_not_depend_on_the_build_threads() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write_project(&root, 40);
    let work = scratch.path();
    let one = run(&root, work, "one", "1", None);
    assert!(!one.1.is_empty());
    for threads in ["2", "8"] {
        let many = run(&root, work, &format!("t{threads}"), threads, None);
        assert!(many == one, "{threads} build threads");
    }
}

#[test]
fn a_cold_and_a_warm_cache_run_equal_a_run_without_the_cache() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write_project(&root, 40);
    let work = scratch.path();
    let cache = work.join("graphs.sqlite");
    let clean = run(&root, work, "clean", "1", None);
    let cold = run(&root, work, "cold", "8", Some(&cache));
    assert!(cold == clean, "a cold cache");
    let warm = run(&root, work, "warm", "8", Some(&cache));
    assert!(warm == clean, "a warm cache");
    let serial = run(&root, work, "serial", "1", Some(&cache));
    assert!(serial == clean, "a serial run on a warm cache");
}
