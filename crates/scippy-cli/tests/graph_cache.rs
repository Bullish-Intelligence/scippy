//! Exercise the size and the reuse of the graph cache in full Scippy runs.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write_files(root: &Path, count: usize) {
    fs::create_dir_all(root).unwrap();
    for n in 0..count {
        let text =
            format!("def f{n}():\n    return {n}\n\nclass C{n}:\n    x = f{n}()\n\nf{n}()\n");
        fs::write(root.join(format!("m{n}.py")), text).unwrap();
    }
}

fn run(root: &Path, output: &Path, cache: Option<&Path>, max_bytes: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scippy"));
    command.args(["index", root.to_str().unwrap(), "--output"]);
    command.arg(output);
    if let Some(cache) = cache {
        command.arg("--graph-cache").arg(cache);
    }
    if let Some(max_bytes) = max_bytes {
        command.args(["--graph-cache-max-bytes", max_bytes]);
    }
    let out = command.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("warning"));
    out
}

fn graphs(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("file graphs: "))
        .unwrap()
        .to_owned()
}

#[test]
fn a_warm_run_equals_a_clean_run() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write_files(&root, 30);
    let cache = scratch.path().join("graphs.sqlite");
    let clean = scratch.path().join("clean.scip");
    run(&root, &clean, None, None);
    let cold = run(&root, &scratch.path().join("cold.scip"), Some(&cache), None);
    assert!(graphs(&cold).ends_with("30 built"), "{}", graphs(&cold));
    let warm = run(&root, &scratch.path().join("warm.scip"), Some(&cache), None);
    assert!(
        graphs(&warm).starts_with("30 cached, 0 built"),
        "{}",
        graphs(&warm)
    );
    assert_eq!(
        fs::read(&clean).unwrap(),
        fs::read(scratch.path().join("cold.scip")).unwrap()
    );
    assert_eq!(
        fs::read(&clean).unwrap(),
        fs::read(scratch.path().join("warm.scip")).unwrap()
    );
}

#[test]
fn the_cap_keeps_the_file_small_and_the_output_the_same() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write_files(&root, 300);
    let cache = scratch.path().join("graphs.sqlite");
    let clean = scratch.path().join("clean.scip");
    run(&root, &clean, None, None);
    run(&root, &scratch.path().join("big.scip"), Some(&cache), None);
    let uncapped = fs::metadata(&cache).unwrap().len();
    let cap = uncapped / 4;
    let capped = run(
        &root,
        &scratch.path().join("capped.scip"),
        Some(&cache),
        Some(&cap.to_string()),
    );
    // The run reads the whole cache. The cap applies after the run.
    assert!(
        graphs(&capped).starts_with("300 cached"),
        "{}",
        graphs(&capped)
    );
    let size = fs::metadata(&cache).unwrap().len();
    assert!(size <= cap, "the cache has {size} bytes for the cap {cap}");
    let after = run(
        &root,
        &scratch.path().join("after.scip"),
        Some(&cache),
        Some(&cap.to_string()),
    );
    assert!(
        !graphs(&after).starts_with("300 cached"),
        "{}",
        graphs(&after)
    );
    for name in ["big", "capped", "after"] {
        assert_eq!(
            fs::read(&clean).unwrap(),
            fs::read(scratch.path().join(format!("{name}.scip"))).unwrap(),
            "{name}"
        );
    }
    assert!(fs::metadata(&cache).unwrap().len() <= cap);
}
