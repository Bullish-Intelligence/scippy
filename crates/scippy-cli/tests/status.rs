//! Check the status of each file, the `--strict` flag, and the publish of a lossy run.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;

use scippy_core::{
    AnalysisSnapshot, IndexReport, IndexRequest, LanguagePack, ReferenceOutcome,
    index_project_with_snapshot,
};
use scippy_loci::LociPack;
use scippy_metalayer::{FileStatus, Metalayer, Outcome, PublishOptions, ReferenceQuery};
use scippy_python::PythonPack;
use scippy_typescript::TypeScriptPack;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn snapshot(
    root: &Path,
    configure: impl FnOnce(&mut IndexRequest),
) -> (IndexReport, AnalysisSnapshot) {
    let out = tempfile::tempdir().unwrap();
    let mut request = IndexRequest::new(root, out.path().join("index.scip"));
    configure(&mut request);
    index_project_with_snapshot(&request, &[&PythonPack, &TypeScriptPack, &LociPack]).unwrap()
}

/// The rows as (path, status, reason).
fn rows(snapshot: &AnalysisSnapshot) -> Vec<(&str, &str, &str)> {
    snapshot
        .lost_files
        .iter()
        .map(|f| (f.path.as_str(), f.status.as_str(), f.reason.as_str()))
        .collect()
}

#[test]
fn every_loss_has_a_file_row_and_the_counts_agree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    write(&root, "good.py", "def f():\n    return 1\n");
    write(&root, "bad.py", "def broken(:\n");
    write(&root, "a.py", "def f():\n    return 1\n");
    write(&root, "a/__init__.py", "def f():\n    return 2\n");
    write(&root, "big.py", &format!("x = '{}'\n", "y".repeat(400)));
    fs::write(root.join("latin.py"), [0xff, 0xfe, b'\n']).unwrap();
    std::os::unix::fs::symlink("good.py", root.join("link.py")).unwrap();
    write(&root, "locked.py", "x = 1\n");
    let locked = root.join("locked.py");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let unreadable = fs::read(&locked).is_err();
    let (report, snapshot) = snapshot(&root, |r| r.limits.max_file_bytes = 200);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();

    let mut expected = vec![
        ("a.py", "shadowed", "shadowed-file"),
        ("bad.py", "failed", "syntax-error"),
        ("big.py", "skipped", "file-too-large"),
        ("latin.py", "skipped", "invalid-utf8"),
        ("link.py", "skipped", "symlink-skipped"),
    ];
    if unreadable {
        expected.push(("locked.py", "failed", "unreadable-file"));
    }
    expected.sort_unstable();
    assert_eq!(rows(&snapshot), expected);
    assert!(!snapshot.input_complete);
    // Every skipped or failed file has a row, and a shadowed file counts apart.
    let lossy = snapshot
        .lost_files
        .iter()
        .filter(|f| f.status != "shadowed")
        .count();
    assert_eq!(report.files_skipped + report.discovery_skipped, lossy);
    assert_eq!(report.files_shadowed, 1);

    // The database holds the same rows, and the input is incomplete.
    let mut layer = Metalayer::open(dir.path().join("layer.sqlite")).unwrap();
    layer
        .publish(&snapshot, &PublishOptions::default())
        .unwrap();
    let view = layer.view().unwrap().unwrap();
    assert!(!view.revision().input_complete);
    let status = |s| -> Vec<String> {
        view.files(Some(s))
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect()
    };
    assert_eq!(status(FileStatus::Shadowed), ["a.py"]);
    assert_eq!(status(FileStatus::Indexed), ["a/__init__.py", "good.py"]);
    assert!(status(FileStatus::Failed).contains(&"bad.py".to_owned()));
    assert!(status(FileStatus::Skipped).contains(&"link.py".to_owned()));
}

#[test]
fn a_shadowed_file_leaves_the_input_complete() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    write(&root, "a.py", "def f():\n    return 1\n");
    write(&root, "a/__init__.py", "def f():\n    return 2\n");
    let (report, snapshot) = snapshot(&root, |_| {});
    assert_eq!(rows(&snapshot), [("a.py", "shadowed", "shadowed-file")]);
    assert_eq!((report.files_skipped, report.files_shadowed), (0, 1));
    assert!(snapshot.input_complete, "{:?}", snapshot.diagnostics);
    let mut layer = Metalayer::open(dir.path().join("layer.sqlite")).unwrap();
    layer
        .publish(&snapshot, &PublishOptions::default())
        .unwrap();
    assert!(layer.view().unwrap().unwrap().revision().input_complete);
}

#[test]
fn the_files_after_the_total_limit_have_rows() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    for name in ["a.py", "b.py", "c.py"] {
        write(&root, name, &format!("x = '{}'\n", "y".repeat(20)));
    }
    let (report, snapshot) = snapshot(&root, |r| r.limits.max_total_bytes = 40);
    assert_eq!(
        rows(&snapshot),
        [
            ("b.py", "skipped", "input-limit-reached"),
            ("c.py", "skipped", "input-limit-reached")
        ]
    );
    assert_eq!(report.discovery_skipped, 2);
    assert!(!snapshot.input_complete);
}

fn scippy(args: &[&str], paths: &[(&str, &Path)]) -> Output {
    let mut command = common::scippy();
    command.arg("index").args(args);
    for (flag, path) in paths {
        command.arg(flag).arg(path);
    }
    command.output().unwrap()
}

#[test]
fn a_syntax_error_publishes_and_strict_rejects_it_and_keeps_every_file() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "good.py", "def f():\n    return 1\n\nf()\n");
    let database = scratch.path().join("layer.sqlite");
    let output = scratch.path().join("index.scip");
    let paths: [(&str, &Path); 2] = [("-o", &output), ("--metalayer", &database)];
    let index = |strict: bool| {
        let mut args = vec![root.to_str().unwrap()];
        if strict {
            args.push("--strict");
        }
        scippy(&args, &paths)
    };
    assert!(index(true).status.success());
    let kept_scip = fs::read(&output).unwrap();
    let revision = |db: &Path| {
        let mut layer = Metalayer::open_read_only(db).unwrap();
        let view = layer.view().unwrap().unwrap();
        (
            view.revision().id,
            view.revision().input_complete,
            view.revision().strict,
        )
    };
    assert_eq!(revision(&database), (1, true, true));
    let kept_db = fs::read(&database).unwrap();

    write(&root, "bad.py", "def broken(:\n");
    // A strict run rejects the lost input. The database and the SCIP file stay as they were.
    let strict = index(true);
    assert!(!strict.status.success());
    let stderr = String::from_utf8_lossy(&strict.stderr);
    assert!(
        stderr.contains("syntax-error") && stderr.contains("incomplete"),
        "{stderr}"
    );
    assert_eq!(fs::read(&output).unwrap(), kept_scip);
    assert_eq!(fs::read(&database).unwrap(), kept_db);
    assert_eq!(revision(&database), (1, true, true));

    // A run without `--strict` publishes the lost input and exits with success.
    let lax = index(false);
    assert!(
        lax.status.success(),
        "{}",
        String::from_utf8_lossy(&lax.stderr)
    );
    assert!(String::from_utf8_lossy(&lax.stderr).contains("syntax-error"));
    assert_eq!(revision(&database), (2, false, false));
    let mut layer = Metalayer::open_read_only(&database).unwrap();
    let view = layer.view().unwrap().unwrap();
    let failed = view.files(Some(FileStatus::Failed)).unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(
        (failed[0].path.as_str(), failed[0].reason.as_deref()),
        ("bad.py", Some("syntax-error"))
    );
}

#[test]
fn a_strict_run_with_lost_input_opens_no_database_and_writes_no_file() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "bad.py", "def broken(:\n");
    let database = scratch.path().join("layer.sqlite");
    let output = scratch.path().join("index.scip");
    let paths: [(&str, &Path); 2] = [("-o", &output), ("--metalayer", &database)];
    let strict = scippy(&[root.to_str().unwrap(), "--strict"], &paths);
    assert!(!strict.status.success());
    assert!(!database.exists());
    assert!(!output.exists());
}

#[test]
fn a_strict_run_without_metalayer_rejects_lost_input_and_keeps_the_output() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    write(&root, "good.py", "def f():\n    return 1\n\nf()\n");
    let output = scratch.path().join("index.scip");
    let paths: [(&str, &Path); 1] = [("-o", &output)];
    let root_arg = root.to_str().unwrap();
    assert!(scippy(&[root_arg, "--strict"], &paths).status.success());
    let kept = fs::read(&output).unwrap();

    // A syntax error is lost input. The strict run fails and leaves the file as it was.
    write(&root, "bad.py", "def broken(:\n");
    let strict = scippy(&[root_arg, "--strict"], &paths);
    assert!(!strict.status.success());
    let stderr = String::from_utf8_lossy(&strict.stderr);
    assert!(
        stderr.contains("syntax-error") && stderr.contains("incomplete"),
        "{stderr}"
    );
    assert_eq!(fs::read(&output).unwrap(), kept);

    // Without `--strict`, the same run exits with success and writes the file.
    fs::remove_file(&output).unwrap();
    let lax = scippy(&[root_arg], &paths);
    assert!(
        lax.status.success(),
        "{}",
        String::from_utf8_lossy(&lax.stderr)
    );
    assert!(String::from_utf8_lossy(&lax.stderr).contains("syntax-error"));
    assert!(output.exists());
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            copy_tree(&entry.path(), &target);
        } else if kind.is_file() {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
#[ignore = "slow: runs in opt:slow"]
fn the_scippy_sources_publish() {
    let repository: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("scippy");
    for part in ["crates", "languages"] {
        copy_tree(&repository.join(part), &root.join(part));
    }
    let database = scratch.path().join("layer.sqlite");
    let output = scratch.path().join("index.scip");
    let paths: [(&str, &Path); 2] = [("-o", &output), ("--metalayer", &database)];
    let run = scippy(&[root.to_str().unwrap()], &paths);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let db = rusqlite::Connection::open(&database).unwrap();
    let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count("SELECT count(*) FROM pragma_foreign_key_check"), 0);
    assert!(count("SELECT count(*) FROM file WHERE status = 'indexed'") > 10);
    assert!(count("SELECT count(*) FROM reference") > 100);
    assert!(count("SELECT count(*) FROM file_reference") > 10);
    assert_eq!(count("SELECT count(*) FROM revision"), 1);
}

#[test]
fn two_roots_with_one_name_publish_an_incomplete_run() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    let roots = dir.path().join("roots");
    write(
        &vault,
        ".loci/vault.toml",
        "schema = 1\nid = \"x\"\nroots_file = \"roots.toml\"\n",
    );
    write(&vault, "Home.md", "# Home\n");
    let mut table = String::new();
    for name in ["app", "one", "two"] {
        let path = roots.join(name);
        table += &format!(
            "[[root]]\nname = \"{name}\"\npath = \"{}\"\nlister = \"walk\"\n\n",
            path.display()
        );
    }
    write(&vault, "roots.toml", &table);
    // The roots `one` and `two` both name their package `lib`. The root `app` needs `lib`.
    write(
        &roots.join("one"),
        "pyproject.toml",
        "[project]\nname = \"lib\"\n",
    );
    write(&roots.join("one"), "lib.py", "x = 1\n");
    write(
        &roots.join("two"),
        "pyproject.toml",
        "[project]\nname = \"lib\"\n",
    );
    write(&roots.join("two"), "lib.py", "x = 2\n");
    write(
        &roots.join("app"),
        "pyproject.toml",
        "[project]\nname = \"app\"\ndependencies = [\"lib\"]\n",
    );
    write(&roots.join("app"), "main.py", "y = 1\n");
    let (_, snapshot) = snapshot(&vault, |_| {});
    let found: Vec<_> = snapshot
        .diagnostics
        .iter()
        .filter(|d| d.code == "root-name-collision")
        .collect();
    assert_eq!(found.len(), 1, "{:?}", snapshot.diagnostics);
    assert!(
        found[0].message.contains("one, two"),
        "{}",
        found[0].message
    );
    // The collision loses a link, not a file, so no file has a row.
    assert!(rows(&snapshot).is_empty(), "{:?}", rows(&snapshot));
    assert!(!snapshot.input_complete);

    // The run still publishes, and the revision says the input is incomplete.
    let mut layer = Metalayer::open(dir.path().join("layer.sqlite")).unwrap();
    layer
        .publish(&snapshot, &PublishOptions::default())
        .unwrap();
    assert!(!layer.view().unwrap().unwrap().revision().input_complete);
}

/// A vault with the note limit of 100 bytes. `Big.md` is over the limit. `files` are more notes.
fn limited_vault(root: &Path, files: &[(&str, &str)]) {
    write(
        root,
        ".loci/vault.toml",
        "schema = 1\nid = \"x\"\n[policy]\nnote_max_bytes = 100\n",
    );
    for (path, text) in files {
        write(root, path, text);
    }
    write(
        root,
        "Big.md",
        &format!("# Big\n\n## H\n\n{}\n", "word ".repeat(60)),
    );
}

fn limited_snapshot(root: &Path) -> AnalysisSnapshot {
    // The command line takes the file limit from the pack. Do the same here.
    let limits = LociPack.input_limits(root).unwrap();
    snapshot(root, |r| r.limits.max_file_bytes = limits.max_file_bytes).1
}

/// The links as (state, anchor state), in the order of the source.
fn link_states(snapshot: &AnalysisSnapshot) -> Vec<(&str, &str)> {
    snapshot
        .edges
        .iter()
        .filter(|e| e.kind == "link")
        .map(|e| {
            (
                e.state.as_deref().unwrap(),
                e.anchor_state.as_deref().unwrap(),
            )
        })
        .collect()
}

/// The outcomes of the references of one note, in the order of the source.
fn outcomes<'a>(snapshot: &'a AnalysisSnapshot, path: &str) -> Vec<&'a ReferenceOutcome> {
    snapshot
        .references
        .iter()
        .filter(|r| r.path == path)
        .map(|r| &r.outcome)
        .collect()
}

fn target_skipped() -> ReferenceOutcome {
    ReferenceOutcome::Unresolved("target-skipped".into())
}

#[test]
fn a_note_over_the_note_limit_is_skipped_and_its_link_is_a_skipped_target() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    limited_vault(&root, &[("Small.md", "See [[Big]].\n")]);
    let snapshot = limited_snapshot(&root);
    let found: Vec<_> = snapshot
        .diagnostics
        .iter()
        .filter(|d| d.code == "file-too-large")
        .collect();
    assert_eq!(found.len(), 1, "{:?}", snapshot.diagnostics);
    assert_eq!(found[0].file.as_deref(), Some("Big.md"));
    assert!(!snapshot.input_complete);
    assert_eq!(rows(&snapshot), [("Big.md", "skipped", "file-too-large")]);
    // The link to the skipped note is not a link to a note that does not exist.
    assert_eq!(link_states(&snapshot), [("skipped-target", "none")]);
    assert_eq!(outcomes(&snapshot, "Small.md"), [&target_skipped()]);
    // The skipped note gets no document and no stub.
    assert!(snapshot.documents.iter().all(|d| d.path != "Big.md"));

    // The run publishes, and the metalayer holds the skipped status and the new values.
    let mut layer = Metalayer::open(dir.path().join("layer.sqlite")).unwrap();
    layer
        .publish(&snapshot, &PublishOptions::default())
        .unwrap();
    let view = layer.view().unwrap().unwrap();
    assert!(!view.revision().input_complete);
    let skipped = view.files(Some(FileStatus::Skipped)).unwrap();
    assert_eq!(skipped.len(), 1);
    assert_eq!(
        (skipped[0].path.as_str(), skipped[0].reason.as_deref()),
        ("Big.md", Some("file-too-large"))
    );
    let edges = view.edges(None).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].state.as_deref(), Some("skipped-target"));
    let references = view.references(ReferenceQuery::default()).unwrap();
    assert_eq!(references.len(), 1);
    assert_eq!(
        references[0].outcome,
        Outcome::Unresolved("target-skipped".into())
    );
}

#[test]
fn an_anchor_link_to_a_skipped_note_has_an_unchecked_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    limited_vault(&root, &[("Small.md", "See [[Big#H]] and [[Big#^b]].\n")]);
    let snapshot = limited_snapshot(&root);
    // The note is not read, so its anchors are unknown. The link is not `missing`.
    assert_eq!(
        link_states(&snapshot),
        [
            ("skipped-target", "unchecked"),
            ("skipped-target", "unchecked")
        ]
    );
    assert_eq!(
        outcomes(&snapshot, "Small.md"),
        [&target_skipped(), &target_skipped()]
    );
}

#[test]
fn an_alias_of_a_skipped_note_stays_missing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    limited_vault(&root, &[("Small.md", "See [[Nick]].\n")]);
    // The alias is in the front matter of the skipped note. Scippy does not read it.
    write(
        &root,
        "Big.md",
        &format!("---\naliases: [Nick]\n---\n{}\n", "word ".repeat(60)),
    );
    let snapshot = limited_snapshot(&root);
    assert_eq!(rows(&snapshot), [("Big.md", "skipped", "file-too-large")]);
    assert_eq!(link_states(&snapshot), [("missing", "none")]);
    assert_eq!(
        outcomes(&snapshot, "Small.md"),
        [&ReferenceOutcome::Unresolved("no-path".into())]
    );
}

#[test]
fn a_skipped_note_takes_part_in_name_resolution_as_an_indexed_note() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    limited_vault(
        &root,
        &[
            // The shortest path of `Big` is `Big.md` at the root. A folder path is explicit.
            ("Small.md", "[[Big]] [[Notes/Big]] [[Dup]] [[Dup]]\n"),
            ("Notes/Big.md", "# Big\n"),
            ("Notes/Dup.md", "# Dup\n"),
            ("Other/Dup.md", "# Dup\n"),
        ],
    );
    let snapshot = limited_snapshot(&root);
    // `[[Big]]` reaches the skipped note at the root. `[[Notes/Big]]` reaches the indexed note.
    // `[[Dup]]` is a tie of two indexed notes, as before.
    let states = link_states(&snapshot);
    assert_eq!(states[0], ("skipped-target", "none"));
    assert_eq!(states[1], ("resolved", "none"));
    assert_eq!(states[2], ("ambiguous", "none"));
    let small = outcomes(&snapshot, "Small.md");
    assert_eq!(small[0], &target_skipped());
    assert!(matches!(small[1], ReferenceOutcome::Resolved(_)));
    assert!(matches!(small[2], ReferenceOutcome::Ambiguous(c) if c.len() == 2));
}

#[test]
fn a_tie_with_a_skipped_note_is_ambiguous_and_names_no_skipped_note() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    // `Big.md` at the root is skipped. `Notes/Big.md` is indexed. The name `Big` finds the
    // shortest path first, so a tie needs two notes at the same depth.
    limited_vault(
        &root,
        &[
            ("Small.md", "[[Dup]] [[Mid]]\n"),
            ("A/Dup.md", &format!("# Dup\n{}\n", "word ".repeat(60))),
            ("B/Dup.md", "# Dup\n"),
            ("C/Mid.md", &format!("# Mid\n{}\n", "word ".repeat(60))),
            ("D/Mid.md", &format!("# Mid\n{}\n", "word ".repeat(60))),
        ],
    );
    let snapshot = limited_snapshot(&root);
    // Loci gives `ambiguous` for two notes with the same name. A skipped note is one of them.
    assert_eq!(
        link_states(&snapshot),
        [("ambiguous", "none"), ("ambiguous", "none")]
    );
    // One indexed note and one skipped note: the link has no single target to name.
    // Two skipped notes: no target at all.
    assert_eq!(
        outcomes(&snapshot, "Small.md"),
        [&target_skipped(), &target_skipped()]
    );
    // The skipped notes get no stub.
    let paths: Vec<_> = snapshot.documents.iter().map(|d| d.path.as_str()).collect();
    assert!(!paths.contains(&"A/Dup.md"), "{paths:?}");
    assert!(!paths.contains(&"C/Mid.md"), "{paths:?}");
}

#[test]
fn a_dangling_link_stays_missing_beside_a_skipped_note() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    limited_vault(&root, &[("Small.md", "[[Nowhere]] [[Big]]\n")]);
    let snapshot = limited_snapshot(&root);
    assert_eq!(
        link_states(&snapshot),
        [("missing", "none"), ("skipped-target", "none")]
    );
    assert_eq!(
        outcomes(&snapshot, "Small.md"),
        [
            &ReferenceOutcome::Unresolved("no-path".into()),
            &target_skipped()
        ]
    );
}
