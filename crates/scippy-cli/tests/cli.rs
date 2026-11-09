//! Run the `scippy` binary end to end.
//!
//! At Gate 1 the binary writes documents without occurrences.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use protobuf::Message;
use scip::types::Index;

fn scippy(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scippy"))
        .args(args)
        .output()
        .unwrap()
}

fn write_tree(root: &Path, files: &[&str]) {
    for file in files {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x = 1\n").unwrap();
    }
}

fn parse(path: &Path) -> Index {
    Index::parse_from_bytes(&fs::read(path).unwrap()).unwrap()
}

/// Drop the location-sensitive root URI.
fn normalised_bytes(path: &Path) -> Vec<u8> {
    let mut index = parse(path);
    index.metadata.as_mut().unwrap().project_root.clear();
    index.write_to_bytes().unwrap()
}

#[test]
fn lists_the_built_in_languages() {
    let output = scippy(&["languages"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "python\ntypescript\n"
    );
}

#[test]
fn indexes_a_tree_into_sorted_documents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    write_tree(
        &root,
        &[
            "b.py",
            "pkg/z.ts",
            "pkg/a.tsx",
            "a.py",
            "notes.txt",
            "node_modules/skip.py",
            ".git/skip.py",
        ],
    );
    let out = dir.path().join("index.scip");

    let output = scippy(&[
        "index",
        root.to_str().unwrap(),
        "--output",
        out.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("files discovered: 5\nfiles indexed: 4\n"),
        "{stdout}"
    );

    let index = parse(&out);
    let documents: Vec<_> = index
        .documents
        .iter()
        .map(|d| (d.relative_path.as_str(), d.language.as_str()))
        .collect();
    assert_eq!(
        documents,
        [
            ("a.py", "Python"),
            ("b.py", "Python"),
            ("pkg/a.tsx", "TypeScript"),
            ("pkg/z.ts", "TypeScript")
        ]
    );
    assert!(index.documents.iter().all(|d| d.occurrences.is_empty()));
    let root_uri = &index.metadata.as_ref().unwrap().project_root;
    assert!(
        root_uri.starts_with("file:///") && root_uri.ends_with("/project"),
        "{root_uri}"
    );
}

#[test]
fn file_creation_order_and_root_location_do_not_change_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let files = ["a.py", "src/m.py", "src/n.ts", "src/deep/o.py", "z.ts"];
    let mut reversed = files;
    reversed.reverse();
    let (one, two) = (dir.path().join("one"), dir.path().join("two"));
    write_tree(&one, &files);
    write_tree(&two, &reversed);
    let (out_one, out_two) = (dir.path().join("one.scip"), dir.path().join("two.scip"));
    for (root, out) in [(&one, &out_one), (&two, &out_two)] {
        assert!(
            scippy(&["index", root.to_str().unwrap(), "-o", out.to_str().unwrap()])
                .status
                .success()
        );
    }
    assert_ne!(
        fs::read(&out_one).unwrap(),
        fs::read(&out_two).unwrap(),
        "the root URIs differ"
    );
    assert_eq!(normalised_bytes(&out_one), normalised_bytes(&out_two));
}

#[test]
fn a_second_run_with_the_output_inside_the_root_gives_the_same_bytes() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(dir.path(), &["a.py"]);
    let out = dir.path().join("index.scip");
    let args = [
        "index",
        dir.path().to_str().unwrap(),
        "--output",
        out.to_str().unwrap(),
    ];
    assert!(scippy(&args).status.success());
    let first = fs::read(&out).unwrap();
    assert!(scippy(&args).status.success());
    assert_eq!(first, fs::read(&out).unwrap());
    assert_eq!(parse(&out).documents.len(), 1);
}

#[test]
fn a_missing_root_fails_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("index.scip");
    let output = scippy(&[
        "index",
        dir.path().join("missing").to_str().unwrap(),
        "--output",
        out.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid project root"));
    assert!(!out.exists());
}

#[test]
fn an_output_in_a_missing_directory_fails() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(dir.path(), &["a.py"]);
    let out: PathBuf = dir.path().join("none").join("index.scip");
    let output = scippy(&[
        "index",
        dir.path().to_str().unwrap(),
        "--output",
        out.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
}

/// The index must pass `scip lint`. The step skips when the CLI is absent.
#[test]
fn scip_lint_accepts_the_cli_output() {
    let found = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join("scip"))
        .find(|candidate| candidate.is_file());
    let Some(scip) = found else {
        assert!(
            std::env::var_os("SCIPPY_REQUIRE_SCIP").is_none(),
            "scip is required but not on PATH"
        );
        eprintln!("SKIPPED: the scip CLI is not on PATH");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write_tree(&dir.path().join("p"), &["a.py", "b/c.ts"]);
    let out = dir.path().join("index.scip");
    assert!(
        scippy(&[
            "index",
            dir.path().join("p").to_str().unwrap(),
            "-o",
            out.to_str().unwrap()
        ])
        .status
        .success()
    );
    let lint = Command::new(scip)
        .args(["lint", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        lint.status.success(),
        "{}",
        String::from_utf8_lossy(&lint.stderr)
    );
}
