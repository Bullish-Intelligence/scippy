//! The helpers of the golden tests.
//!
//! One call checks one case: `golden("case", &["command", "--option"])` runs
//! `scippy metalayer command --option --db <fixture db> --json`, normalises the envelope,
//! validates it against the schema, and compares it with
//! `tests/golden/query/<command>-<case>.json`. The fixture is `tests/fixtures/query/`. The
//! harness indexes a copy of it once for the whole test binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use jsonschema::{Retrieve, Uri, Validator};
use serde_json::{Value, json};

const SCHEMA_ID: &str = "https://scippy.invalid/schema/";
const HINT: &str = "run `UPDATE_GOLDEN=1 cargo test -p scippy-cli --test query_golden` to update; a change is a contract change";

/// The indexed copy of the fixture.
struct Fixture {
    /// The directory that holds the copy, the database, and the scratch files.
    base: PathBuf,
    root: PathBuf,
    db: PathBuf,
    /// Volatile text of the human output, with the placeholder of each.
    volatile: Vec<(String, &'static str)>,
}

pub struct Run {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

fn manifest(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("query-golden");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let root = base.join("project");
        copy_tree(&manifest("tests/fixtures/query"), &root);
        let db = base.join("layer.sqlite");
        let indexed = Command::new(env!("CARGO_BIN_EXE_scippy"))
            .args(["index", root.to_str().unwrap(), "--output"])
            .arg(base.join("index.scip"))
            .arg("--metalayer")
            .arg(&db)
            .output()
            .unwrap();
        assert!(
            indexed.status.success(),
            "{}",
            String::from_utf8_lossy(&indexed.stderr)
        );
        let mut fixture = Fixture {
            base,
            root,
            db,
            volatile: Vec::new(),
        };
        let shown = run_in(&fixture, &["show"], true);
        let envelope: Value = serde_json::from_str(&shown.stdout).unwrap();
        let text = |pointer: &str| {
            envelope
                .pointer(pointer)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        };
        fixture.volatile = vec![
            (
                format!("tool_version  {}", text("/revision/tool_version")),
                "tool_version  <tool_version>",
            ),
            (text("/revision/created_at"), "<created_at>"),
            (text("/revision/input_digest"), "<input_digest>"),
            (text("/revision/scip_sha256"), "<scip_sha256>"),
            (text("/root_set/key"), "<root_key>"),
            (
                format!(
                    "scip_size  {}",
                    envelope.pointer("/revision/scip_size").unwrap()
                ),
                "scip_size  <scip_size>",
            ),
        ];
        fixture
    })
}

/// Run `scippy metalayer <args> --db <fixture db>`, with `--json` when `json` is true.
fn run_in(fixture: &Fixture, args: &[&str], json: bool) -> Run {
    run_with_db(&fixture.db, args, json)
}

fn run_with_db(db: &Path, args: &[&str], json: bool) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scippy"));
    command.arg("metalayer").args(args).arg("--db").arg(db);
    if json {
        command.arg("--json");
    }
    let out = command.output().unwrap();
    Run {
        status: out.status.code().unwrap(),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

/// Run `scippy metalayer <args>` as given. The caller passes `--db` itself.
pub fn run(args: &[&str]) -> Run {
    fixture();
    let out = Command::new(env!("CARGO_BIN_EXE_scippy"))
        .arg("metalayer")
        .args(args)
        .output()
        .unwrap();
    Run {
        status: out.status.code().unwrap(),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

/// A path for a scratch database of one test. The directory is in the fixture base.
pub fn scratch_db(name: &str) -> PathBuf {
    let dir = fixture().base.join("scratch");
    fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.sqlite"))
}

/// The path of the fixture project, for a test that edits or reads its files.
#[allow(dead_code)]
pub fn project_root() -> &'static Path {
    &fixture().root
}

/// The path of the fixture database, for a test that runs commands itself.
pub fn fixture_db() -> &'static Path {
    &fixture().db
}

fn update() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| !v.is_empty())
}

pub fn golden_dir() -> impl Iterator<Item = PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(manifest("tests/golden/query"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    paths.into_iter()
}

pub fn read_golden(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// Replace the values that change from run to run, and the temporary paths.
fn normalise(fixture: &Fixture, value: &mut Value) {
    let root = fixture.root.to_str().unwrap().to_owned();
    let base = fixture.base.to_str().unwrap().to_owned();
    let replace_paths = |text: &str| text.replace(&root, "<root>").replace(&base, "<tmp>");
    match value {
        Value::String(text) => *text = replace_paths(text),
        Value::Array(items) => items.iter_mut().for_each(|v| normalise(fixture, v)),
        Value::Object(map) => map.values_mut().for_each(|v| normalise(fixture, v)),
        _ => {}
    }
    let set = |value: &mut Value, pointer: &str, to: Value| {
        if let Some(slot) = value.pointer_mut(pointer) {
            *slot = to;
        }
    };
    set(value, "/tool/version", json!("<tool_version>"));
    set(value, "/revision/tool_version", json!("<tool_version>"));
    set(value, "/revision/created_at", json!("<created_at>"));
    set(value, "/revision/input_digest", json!("<input_digest>"));
    set(value, "/revision/scip_sha256", json!("<scip_sha256>"));
    // The SCIP file holds the project root, so its size depends on the path.
    set(value, "/revision/scip_size", json!(0));
    set(value, "/root_set/key", json!("<root_key>"));
}

fn normalise_text(fixture: &Fixture, text: &str) -> String {
    let mut text = text
        .replace(fixture.root.to_str().unwrap(), "<root>")
        .replace(fixture.base.to_str().unwrap(), "<tmp>");
    for (value, placeholder) in &fixture.volatile {
        text = text.replace(value, placeholder);
    }
    text
}

/// Reads the schema files from `crates/scippy-cli/schema`. The `$id` of the root maps to that
/// directory.
struct SchemaFiles;

impl Retrieve for SchemaFiles {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let relative = uri
            .as_str()
            .strip_prefix(SCHEMA_ID)
            .ok_or_else(|| format!("unknown schema {uri}"))?;
        let text = fs::read_to_string(manifest("schema").join(relative))?;
        Ok(serde_json::from_str(&text)?)
    }
}

fn validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let root: Value = serde_json::from_str(
            &fs::read_to_string(manifest("schema/metalayer-v1.schema.json")).unwrap(),
        )
        .unwrap();
        jsonschema::options()
            .with_retriever(SchemaFiles)
            .build(&root)
            .unwrap()
    })
}

/// Validate an envelope against the schema. The error is the list of messages.
pub fn validate(envelope: &Value) -> Result<(), String> {
    let errors: Vec<String> = validator()
        .iter_errors(envelope)
        .map(|e| format!("{e} (at {})", e.instance_path()))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn check_file(path: &Path, actual: &str) {
    if update() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}; {HINT}", path.display()));
    assert_eq!(expected, actual, "{} differs; {HINT}", path.display());
}

fn command_of<'a>(args: &[&'a str]) -> &'a str {
    args.first().expect("the args name a command")
}

/// Run a command on a database, and check the exit code, the envelope, and the golden file.
pub fn golden_db(case: &str, db: &Path, args: &[&str], exit: i32) {
    let fixture = fixture();
    let out = run_with_db(db, args, true);
    assert_eq!(
        out.status, exit,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    let mut envelope: Value = serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", out.stdout));
    normalise(fixture, &mut envelope);
    if let Err(errors) = validate(&envelope) {
        panic!("the envelope breaks the schema: {errors}\n{envelope:#}");
    }
    let path = manifest(&format!(
        "tests/golden/query/{}-{case}.json",
        command_of(args)
    ));
    let mut text = serde_json::to_string_pretty(&envelope).unwrap();
    text.push('\n');
    check_file(&path, &text);
}

/// Check a case on the fixture database. The exit code is 0.
pub fn golden(case: &str, args: &[&str]) {
    golden_exit(case, args, 0);
}

/// Check a case on the fixture database, with an exit code that is not 0.
pub fn golden_exit(case: &str, args: &[&str], exit: i32) {
    golden_db(case, &fixture().db, args, exit);
}

/// Check the human output of a case against `tests/golden/query/<command>-<case>.txt`.
pub fn golden_text(case: &str, args: &[&str]) {
    let fixture = fixture();
    let out = run_in(fixture, args, false);
    assert_eq!(out.status, 0, "stderr: {}", out.stderr);
    let path = manifest(&format!(
        "tests/golden/query/{}-{case}.txt",
        command_of(args)
    ));
    check_file(&path, &normalise_text(fixture, &out.stdout));
}
