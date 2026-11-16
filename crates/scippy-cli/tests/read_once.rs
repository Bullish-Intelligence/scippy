//! Check that a run reads each input file once, and that a file that changes in the run is stale.

#[path = "../../../test-support/copy_tree.rs"]
mod copy_tree;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use scippy_core::{IndexRequest, ReadHook, ReferenceOutcome, index_project_with_snapshot};
use scippy_loci::LociPack;
use scippy_metalayer::{Metalayer, PublishOptions};
use scippy_python::PythonPack;

use copy_tree::copy_tree;

const EDITED: &str = "src/base/types.py";

/// Copy the fixture with three Python roots that depend on each other. Return the vault folder.
/// The root `base` is a dependency of `lib`, and `lib` is a dependency of `app`.
fn vault(scratch: &Path) -> PathBuf {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../languages/loci/tests/fixtures/deps");
    copy_tree(&fixture, scratch);
    let root = scratch.join("vault");
    let roots_file = root.join("roots.toml");
    let roots = fs::read_to_string(&roots_file)
        .unwrap()
        .replace("@ROOTS@", scratch.join("roots").to_str().unwrap());
    fs::write(roots_file, roots).unwrap();
    root
}

/// A hook that counts the reads of each file. After the first read of a file whose path ends
/// with `edit`, it replaces the file on disk, as an editor does.
fn counting(edit: &'static str) -> (ReadHook, Arc<Mutex<HashMap<PathBuf, usize>>>) {
    let reads: Arc<Mutex<HashMap<PathBuf, usize>>> = Arc::default();
    let seen = Arc::clone(&reads);
    let hook = ReadHook::new(move |path| {
        *seen.lock().unwrap().entry(path.to_owned()).or_default() += 1;
        if path.ends_with(edit) {
            fs::write(
                path,
                "class Base:\n    def halt(self):\n        return None\n",
            )
            .unwrap();
        }
    });
    (hook, reads)
}

#[test]
fn a_dependency_file_of_several_roots_is_read_once_and_a_change_after_the_read_is_stale() {
    let scratch = tempfile::tempdir().unwrap();
    let root = vault(scratch.path());
    let (hook, reads) = counting(EDITED);
    let mut request = IndexRequest::new(&root, scratch.path().join("index.scip"));
    request.jobs = 4;
    request.read_hook = Some(hook);
    let (report, snapshot) =
        index_project_with_snapshot(&request, &[&PythonPack, &LociPack]).unwrap();

    // Each file is read once. The shards of `lib` and `app` load `base/src/base/types.py` as a
    // dependency, and they share the first read.
    let reads = reads.lock().unwrap();
    assert!(reads.len() > 4, "{reads:?}");
    assert!(reads.keys().any(|p| p.ends_with(EDITED)));
    assert!(reads.values().all(|&n| n == 1), "{reads:?}");

    // Every stage saw the first content: the call `engine.stop()` in `app` resolves to the
    // method `stop` of the old class, and the document keeps the old text.
    let document = snapshot
        .documents
        .iter()
        .find(|d| d.path == format!("base/{EDITED}"))
        .expect("the edited file keeps its document");
    assert!(document.source.contains("def stop"), "{}", document.source);
    assert!(document.stale);
    let main = snapshot
        .documents
        .iter()
        .find(|d| d.path == "app/src/app/main.py")
        .unwrap();
    let start = main.source.find("engine.stop()").unwrap() + "engine.".len();
    let outcome = &snapshot
        .references
        .iter()
        .find(|r| r.path == main.path && r.start == start as u32)
        .unwrap()
        .outcome;
    assert!(
        matches!(outcome, ReferenceOutcome::Resolved(s) if s.contains("Base#stop")),
        "{outcome:?}"
    );

    // The file is stale, not lost. The SCIP file holds it, and the input stays complete.
    assert_eq!(report.stale_files, [format!("base/{EDITED}")]);
    assert!(snapshot.lost_files.is_empty(), "{:?}", snapshot.lost_files);
    assert!(snapshot.input_complete, "{:?}", snapshot.diagnostics);
    assert!(snapshot.scip.windows(8).any(|w| w == b"types.py"));
    assert_eq!(
        snapshot.documents.iter().filter(|d| d.stale).count(),
        1,
        "only the edited file is stale"
    );

    // The metalayer store keeps the file as indexed and stale.
    let mut layer = Metalayer::open(scratch.path().join("layer.sqlite")).unwrap();
    layer
        .publish(&snapshot, &PublishOptions::default())
        .unwrap();
    let view = layer.view().unwrap().unwrap();
    assert!(view.revision().input_complete);
    let files = view.files(None).unwrap();
    let stale: Vec<_> = files.iter().filter(|f| f.stale).collect();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].path, format!("base/{EDITED}"));
    assert_eq!(stale[0].reason, None);
}

#[test]
fn an_unchanged_project_has_no_stale_file() {
    let scratch = tempfile::tempdir().unwrap();
    let root = vault(scratch.path());
    let request = IndexRequest::new(&root, scratch.path().join("index.scip"));
    let (report, snapshot) =
        index_project_with_snapshot(&request, &[&PythonPack, &LociPack]).unwrap();
    assert!(report.stale_files.is_empty());
    assert!(snapshot.documents.iter().all(|d| !d.stale));
    assert!(snapshot.input_complete);
}
