//! Check the limits of the search for each reference.

use std::fs;
use std::path::Path;
use std::process::Command;

use scippy_core::{IndexRequest, ReferenceOutcome, ResolveLimits, index_project_with_snapshot};
use scippy_metalayer::{Metalayer, PublishOptions};
use scippy_python::PythonPack;

const SOURCE: &str = "import os\n\n\nclass Base:\n    def run(self):\n        return os.getcwd()\n\n\nclass Child(Base):\n    def go(self):\n        return self.run()\n\n\ndef make():\n    return Child()\n\n\nmake().go()\n";

fn project(root: &Path) {
    fs::write(root.join("a.py"), SOURCE).unwrap();
    fs::write(root.join("pyproject.toml"), "[project]\nname = \"demo\"\n").unwrap();
}

/// Index the small project with `limits`. Return the kinds of bound in the outcomes, the
/// report counts, and the snapshot.
fn bounded(limits: ResolveLimits) -> (Vec<String>, usize, usize, scippy_core::AnalysisSnapshot) {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let out = tempfile::tempdir().unwrap();
    let mut request = IndexRequest::new(dir.path(), out.path().join("index.scip"));
    request.resolve_limits = limits;
    let (report, snapshot) = index_project_with_snapshot(&request, &[&PythonPack]).unwrap();
    let kinds = snapshot
        .references
        .iter()
        .filter_map(|r| match &r.outcome {
            ReferenceOutcome::Bounded(kind) => Some(kind.clone()),
            _ => None,
        })
        .collect();
    (
        kinds,
        report.references_bounded,
        report.bounded_by_kind.work,
        snapshot,
    )
}

#[test]
fn the_default_limits_bound_no_reference_of_a_small_project() {
    let (kinds, count, _, snapshot) = bounded(ResolveLimits::default());
    assert!(kinds.is_empty(), "{kinds:?}");
    assert_eq!(count, 0);
    assert!(snapshot.analysis_exact);
}

#[test]
fn a_tiny_work_limit_bounds_the_references_and_the_snapshot_still_publishes() {
    let mut limits = ResolveLimits::default();
    limits.max_work = 1;
    let (kinds, count, work, snapshot) = bounded(limits);
    assert!(!kinds.is_empty());
    assert!(kinds.iter().all(|k| k == "work"), "{kinds:?}");
    assert_eq!(count, work);
    assert!(count > 0);
    assert!(!snapshot.analysis_exact);
    assert!(snapshot.input_complete, "{:?}", snapshot.diagnostics);
    let scratch = tempfile::tempdir().unwrap();
    let mut layer = Metalayer::open(scratch.path().join("meta.sqlite")).unwrap();
    let published = layer
        .publish(&snapshot, &PublishOptions::default())
        .unwrap();
    assert_eq!(published.revision, 1);
    let view = layer.view().unwrap().unwrap();
    assert!(!view.revision().analysis_exact);
    assert!(view.revision().input_complete);
}

#[test]
fn the_limit_flags_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let out = dir.path().join("out.scip");
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_scippy"))
            .arg("index")
            .arg(dir.path())
            .arg("--output")
            .arg(&out)
            .args(extra)
            .output()
            .unwrap()
    };
    let loose = run(&[
        "--max-work",
        "200000",
        "--max-queue-len",
        "4000",
        "--max-arena-cells",
        "40000000",
    ]);
    assert!(loose.status.success(), "{loose:?}");
    let text = String::from_utf8(loose.stdout).unwrap();
    assert!(text.contains("references bounded: 0"), "{text}");
    assert!(text.contains("bounded by kind: work 0,"), "{text}");
    let tight = run(&["--max-work", "1"]);
    assert!(tight.status.success(), "{tight:?}");
    let text = String::from_utf8(tight.stdout).unwrap();
    assert!(!text.contains("references bounded: 0"), "{text}");
    assert!(!text.contains("bounded by kind: work 0,"), "{text}");
    let zero = run(&["--max-work", "0"]);
    assert!(!zero.status.success());
}
