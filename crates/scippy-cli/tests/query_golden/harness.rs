//! The helpers of the golden tests.
//!
//! One call checks one case: `golden("case", &["command", "--option"])` runs
//! `scippy metalayer command --option --db <fixture db> --json`, normalises the envelope,
//! validates it against the schema, and compares it with
//! `tests/golden/query/<command>-<case>.json`. The fixture is `tests/fixtures/query/`. The
//! harness indexes a copy of it once for all test processes.

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

/// The directory of the shared fixture. The name is a key of the inputs: the `scippy` binary,
/// this test binary, and the fixture files. Another input set gives another directory.
fn shared_dir(tmp: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    fn stamp(path: &Path, hasher: &mut impl Hasher) {
        let meta = fs::metadata(path).unwrap();
        path.hash(hasher);
        meta.len().hash(hasher);
        meta.modified().unwrap().hash(hasher);
    }
    fn walk(dir: &Path, hasher: &mut impl Hasher) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, hasher);
            } else {
                stamp(&path, hasher);
            }
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    stamp(Path::new(env!("CARGO_BIN_EXE_scippy")), &mut hasher);
    stamp(&std::env::current_exe().unwrap(), &mut hasher);
    walk(&manifest("tests/fixtures/query"), &mut hasher);
    tmp.join(format!("shared-{:016x}", hasher.finish()))
}

/// A lock between test processes: the directory `lock.d` exists while its owner holds it.
/// `File::lock` needs Rust 1.89, and the minimum supported version is 1.85.
struct DirLock(PathBuf);

impl DirLock {
    fn acquire(tmp: &Path) -> DirLock {
        let path = tmp.join("lock.d");
        let stale = std::time::Duration::from_secs(1800);
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return DirLock(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // An owner that died leaves the directory. Remove it after 30 minutes.
                    let age = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok());
                    if age.is_some_and(|age| age > stale) {
                        let _ = fs::remove_dir(&path);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => panic!("cannot take the fixture lock: {e}"),
            }
        }
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

/// Index the fixture once for all test processes. Nextest runs each test in its own process,
/// so the first process builds the copy under a lock and the others reuse it.
fn shared_fixture(tmp: &Path) -> PathBuf {
    let _lock = DirLock::acquire(tmp);
    let shared = shared_dir(tmp);
    if !shared.join("done").exists() {
        // Remove the output of an earlier input set and the scratch of earlier runs.
        for entry in fs::read_dir(tmp).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().is_some_and(|n| n == "lock.d") {
                continue;
            }
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
        fs::create_dir_all(&shared).unwrap();
        let shared = shared.canonicalize().unwrap();
        let root = shared.join("project");
        copy_tree(&manifest("tests/fixtures/query"), &root);
        let indexed = Command::new(env!("CARGO_BIN_EXE_scippy"))
            .args(["index", root.to_str().unwrap(), "--output"])
            .arg(shared.join("index.scip"))
            .arg("--metalayer")
            .arg(shared.join("layer.sqlite"))
            .output()
            .unwrap();
        assert!(
            indexed.status.success(),
            "{}",
            String::from_utf8_lossy(&indexed.stderr)
        );
        fs::write(shared.join("done"), "").unwrap();
    }
    shared.canonicalize().unwrap()
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("query-golden");
        fs::create_dir_all(&tmp).unwrap();
        let shared = shared_fixture(&tmp);
        // The scratch of this process. A test writes only here.
        let base = tmp.join(format!("run-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let root = shared.join("project");
        let db = shared.join("layer.sqlite");
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

// ---- The identity store ----
//
// `entity` and `annotate` need no facts store. Their cases use a seeded identity store in a
// scratch file. `identity_store(name)` makes a fresh one for each case, because the writers
// change it. The commands run with `SCIPPY_TEST_FIXED_IDS`, so ids and times do not vary.

/// The clock and the first id number of the commands. The ids of the seed differ from these.
const FIXED_IDS: &str = "2026-01-02T00:00:00Z,1";

/// The id of a seeded entity or annotation: `00000000-0000-7000-8000-<n in hex>`.
pub fn seed_id(n: u64) -> String {
    format!("00000000-0000-7000-8000-{n:012x}")
}

/// The path of the identity store of one case. The file does not exist yet.
pub fn identity_path(name: &str) -> PathBuf {
    let dir = fixture().base.join("scratch");
    fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.identity.db"))
}

/// Create a store with this content. The seed goes through SQL, because the records of the
/// crate are `#[non_exhaustive]`. The store holds:
///
/// - `0xe1` module, `0xe2` class `Base` (annotation `0xa1`), `0xe3` method `Base.make`
///   (annotation `0xa2`): matched.
/// - `0xe4` function `helper`: moved by T3 from `pkg/base.py` to `pkg/util.py` (annotation
///   `0xa3`). `0xe9` function `loader`: moved by T2, no annotation.
/// - `0xe5` function `pick`: ambiguous between `0xe10` and `0xe11` (annotation `0xa4`). `0xe10`
///   has no annotation. `0xe11` has one (`0xa6`).
/// - `0xe6` function `gone`: orphaned, one veto (annotation `0xa5`).
/// - `0xe7` function `dup` and `0xe8` variable `dup`: matched, same path and name.
/// - `0xf1` note `notes/index.md`, `0xf2` heading `Intro/Setup` (annotation `0xa7`), `0xf3`
///   block `^b1`.
pub fn identity_store(name: &str) -> PathBuf {
    let path = identity_path(name);
    let _ = fs::remove_file(&path);
    drop(
        scippy_metalayer::Identity::open(&path, &scippy_metalayer::IdentityOptions::default())
            .unwrap(),
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    let t = "2026-01-01T00:00:00Z";
    let id = seed_id;
    let entity = |n: u64, family: &str, kind: &str| {
        db.execute(
            "INSERT INTO entity (id, family, kind, parent_id, created_at) VALUES (?1, ?2, ?3, NULL, ?4)",
            rusqlite::params![id(n), family, kind, t],
        )
        .unwrap();
    };
    // (entity, seq, key, symbol, path, kind, name, container)
    let locator = |n: u64,
                   seq: i64,
                   key: &str,
                   path: &str,
                   kind: &str,
                   name: &str,
                   container: &str,
                   digest: &str| {
        let symbol = format!("scippy python demo 0.1.0 {key}");
        db.execute(
            "INSERT INTO locator (entity_id, seq, key, symbol, path, kind, name, container,
               sig_digest, body_digest, body_tokens, input_digest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11)",
            rusqlite::params![
                id(n),
                seq,
                key,
                symbol,
                path,
                kind,
                name,
                container,
                format!("sig{n:x}"),
                12,
                digest
            ],
        )
        .unwrap();
    };
    let outcome = |n: u64, state: &str, tier: Option<&str>, candidates: Option<&str>| {
        db.execute(
            "INSERT INTO outcome (entity_id, outcome, tier, candidates, since_digest)
             VALUES (?1, ?2, ?3, ?4, 'digest-2')",
            rusqlite::params![id(n), state, tier, candidates],
        )
        .unwrap();
    };
    let annotation = |n: u64,
                      entity: u64,
                      at: &str,
                      author: Option<&str>,
                      tag: Option<&str>,
                      body: &str,
                      snapshot: &str| {
        db.execute(
            "INSERT INTO annotation (id, entity_id, created_at, author, tag, body, snapshot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id(n), id(entity), at, author, tag, body, snapshot],
        )
        .unwrap();
    };

    db.execute_batch(
        "INSERT INTO meta (key, value) VALUES ('root_key', 'root-1'), ('bound_input_digest', 'digest-2');",
    )
    .unwrap();
    entity(0xe1, "code", "module");
    locator(
        0xe1,
        0,
        "pkg/base/__init__:",
        "pkg/base.py",
        "Module",
        "base",
        "",
        "digest-1",
    );
    entity(0xe2, "code", "class");
    locator(
        0xe2,
        0,
        "pkg/base/Base#",
        "pkg/base.py",
        "Class",
        "Base",
        "",
        "digest-1",
    );
    entity(0xe3, "code", "method");
    locator(
        0xe3,
        0,
        "pkg/base/Base#make().",
        "pkg/base.py",
        "Method",
        "make",
        "Base",
        "digest-1",
    );
    entity(0xe4, "code", "function");
    locator(
        0xe4,
        0,
        "pkg/util/helper().",
        "pkg/util.py",
        "Function",
        "helper",
        "",
        "digest-2",
    );
    locator(
        0xe4,
        1,
        "pkg/base/helper().",
        "pkg/base.py",
        "Function",
        "helper",
        "",
        "digest-1",
    );
    outcome(0xe4, "moved", Some("T3"), None);
    entity(0xe9, "code", "function");
    locator(
        0xe9,
        0,
        "pkg/new/loader().",
        "pkg/new.py",
        "Function",
        "loader",
        "",
        "digest-2",
    );
    locator(
        0xe9,
        1,
        "pkg/old/loader().",
        "pkg/new.py",
        "Function",
        "loader",
        "",
        "digest-1",
    );
    outcome(0xe9, "moved", Some("T2"), None);
    entity(0xe5, "code", "function");
    locator(
        0xe5,
        0,
        "pkg/old/pick().",
        "pkg/old.py",
        "Function",
        "pick",
        "",
        "digest-1",
    );
    entity(0xe10, "code", "function");
    locator(
        0xe10,
        0,
        "pkg/a/pick().",
        "pkg/a.py",
        "Function",
        "pick",
        "",
        "digest-2",
    );
    entity(0xe11, "code", "function");
    locator(
        0xe11,
        0,
        "pkg/b/pick().",
        "pkg/b.py",
        "Function",
        "pick",
        "",
        "digest-2",
    );
    outcome(
        0xe5,
        "ambiguous",
        None,
        Some("[\"pkg/a/pick().\",\"pkg/b/pick().\"]"),
    );
    entity(0xe6, "code", "function");
    locator(
        0xe6,
        0,
        "pkg/gone/gone().",
        "pkg/gone.py",
        "Function",
        "gone",
        "",
        "digest-1",
    );
    outcome(0xe6, "orphaned", None, None);
    db.execute(
        "INSERT INTO veto (entity_id, key) VALUES (?1, 'pkg/other/gone().')",
        [id(0xe6)],
    )
    .unwrap();
    entity(0xe7, "code", "function");
    locator(
        0xe7,
        0,
        "pkg/dup/dup().",
        "pkg/dup.py",
        "Function",
        "dup",
        "",
        "digest-1",
    );
    entity(0xe8, "code", "variable");
    locator(
        0xe8,
        0,
        "pkg/dup/dup.",
        "pkg/dup.py",
        "Variable",
        "dup",
        "",
        "digest-1",
    );
    entity(0xf1, "note", "note");
    locator(
        0xf1,
        0,
        "id:0000-note-index",
        "notes/index.md",
        "Note",
        "index.md",
        "",
        "digest-1",
    );
    entity(0xf2, "note", "heading");
    locator(
        0xf2,
        0,
        "0xf1/Intro/Setup#0",
        "notes/index.md",
        "Heading",
        "Setup",
        "Intro",
        "digest-1",
    );
    entity(0xf3, "note", "block");
    locator(
        0xf3,
        0,
        "0xf1/^b1",
        "notes/index.md",
        "Block",
        "^b1",
        "",
        "digest-1",
    );

    let snap = |path: &str, name: &str| format!("{{\"path\":\"{path}\",\"name\":\"{name}\"}}");
    annotation(
        0xa1,
        0xe2,
        "2026-01-01T09:00:00Z",
        Some("ann"),
        Some("design"),
        "Root class of the package.",
        &snap("pkg/base.py", "Base"),
    );
    annotation(
        0xa2,
        0xe3,
        "2026-01-01T10:00:00Z",
        None,
        None,
        "Check the cache first.\nThen build.",
        &snap("pkg/base.py", "make"),
    );
    annotation(
        0xa3,
        0xe4,
        "2026-01-01T11:00:00Z",
        Some("ann"),
        Some("todo"),
        "Move this to the core.",
        &snap("pkg/base.py", "helper"),
    );
    annotation(
        0xa4,
        0xe5,
        "2026-01-01T12:00:00Z",
        None,
        None,
        "Which pick is it?",
        &snap("pkg/old.py", "pick"),
    );
    annotation(
        0xa5,
        0xe6,
        "2026-01-01T13:00:00Z",
        Some("bob"),
        None,
        "Deleted upstream.",
        &snap("pkg/gone.py", "gone"),
    );
    annotation(
        0xa6,
        0xe11,
        "2026-01-01T14:00:00Z",
        None,
        Some("keep"),
        "Mine, not the old one.",
        &snap("pkg/b.py", "pick"),
    );
    annotation(
        0xa7,
        0xf2,
        "2026-01-01T15:00:00Z",
        Some("ann"),
        None,
        "Rewrite the setup steps.",
        &snap("notes/index.md", "Setup"),
    );
    drop(db);
    path
}

/// The text of the key of a seeded candidate, for `--candidate`.
pub const CANDIDATE_A: &str = "pkg/a/pick().";
pub const CANDIDATE_B: &str = "pkg/b/pick().";

/// Run `scippy metalayer <args>` with fixed ids. The caller passes `--identity` or `--db`.
pub fn run_fixed(args: &[&str]) -> Run {
    fixture();
    let out = Command::new(env!("CARGO_BIN_EXE_scippy"))
        .arg("metalayer")
        .args(args)
        .env("SCIPPY_TEST_FIXED_IDS", FIXED_IDS)
        .output()
        .unwrap();
    Run {
        status: out.status.code().unwrap(),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

/// Run a command of the identity store on `store`, with `--json`. Check the exit code, the
/// envelope, the schema, and the golden file `tests/golden/query/<command>-<case>.json`.
pub fn golden_identity(case: &str, store: &Path, args: &[&str], exit: i32) {
    let mut all: Vec<&str> = args.to_vec();
    all.extend(["--identity", store.to_str().unwrap()]);
    golden_identity_raw(case, &all, exit);
}

/// Like [`golden_identity`], but the caller passes `--identity` or `--db` itself.
pub fn golden_identity_raw(case: &str, args: &[&str], exit: i32) {
    let fixture = fixture();
    let mut all: Vec<&str> = args.to_vec();
    all.push("--json");
    let out = run_fixed(&all);
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

/// Check the human output of a command of the identity store against
/// `tests/golden/query/<command>-<case>.txt`.
pub fn golden_identity_text(case: &str, store: &Path, args: &[&str]) {
    let fixture = fixture();
    let mut all: Vec<&str> = args.to_vec();
    all.extend(["--identity", store.to_str().unwrap()]);
    let out = run_fixed(&all);
    assert_eq!(out.status, 0, "stderr: {}", out.stderr);
    let path = manifest(&format!(
        "tests/golden/query/{}-{case}.txt",
        command_of(args)
    ));
    check_file(&path, &normalise_text(fixture, &out.stdout));
}
