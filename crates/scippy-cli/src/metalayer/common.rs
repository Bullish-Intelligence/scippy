//! What every `metalayer` command shares: the common options, the envelope, and the error
//! rules. A command module calls [`execute`] with a closure that reads one [`View`].

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use scippy_core::ScippyError;
use scippy_metalayer::{
    FixedIds, IDENTITY_VERSION, IdSource, Identity, IdentityError, IdentityOptions, IdentityProbe,
    Metalayer, Probe, SCHEMA_VERSION, SystemIds, View, default_identity_path,
};
use serde_json::{Value, json};

/// The version of the JSON contract. A breaking change bumps it.
pub const FORMAT_VERSION: u64 = 1;

/// The options of every command. Flatten them into the arguments of a command.
#[derive(Args, Clone, Debug)]
pub struct Common {
    /// The metalayer database.
    #[arg(long, value_name = "PATH")]
    pub db: PathBuf,
    /// Print one JSON envelope to the standard output stream.
    #[arg(long)]
    pub json: bool,
    /// Keep the first N items of a list. The value reports `total` and `truncated`.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
}

/// The options of the commands that read or write the identity store (`entity`, `annotate`).
/// They need no facts store: `--db` only names the place of the default identity path.
#[derive(Args, Clone, Debug)]
pub struct IdentityArgs {
    /// The metalayer database. The identity store is `<db dir>/<db stem>.identity.db`.
    #[arg(long, value_name = "PATH", required_unless_present = "identity")]
    pub db: Option<PathBuf>,
    /// The identity store. It overrides the default path.
    #[arg(long, value_name = "PATH")]
    pub identity: Option<PathBuf>,
    /// Print one JSON envelope to the standard output stream.
    #[arg(long)]
    pub json: bool,
}

impl IdentityArgs {
    /// The path of the identity store: `--identity`, else the default next to `--db`.
    pub fn store_path(&self) -> PathBuf {
        match (&self.identity, &self.db) {
            (Some(path), _) => path.clone(),
            (None, Some(db)) => default_identity_path(db),
            (None, None) => PathBuf::from("metalayer.identity.db"),
        }
    }
}

/// How a command uses the identity store.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Open the store read-only. A store with an older version is refused.
    Read,
    /// Open the store for writing. An older version migrates forward.
    Write,
}

/// The result of one command, before the envelope wraps it.
pub struct Output {
    /// The `value` of the envelope.
    pub value: Value,
    /// The text of the human mode. It ends with a new line, or it is empty.
    pub human: String,
    /// The exit code. Only `check` sets a code other than 0 on success.
    pub exit: u8,
}

impl Output {
    pub fn new(value: Value, human: String) -> Self {
        Self {
            value,
            human,
            exit: 0,
        }
    }
}

/// A failure. The kind is a stable kebab-case string of the contract.
#[derive(Debug)]
pub struct CliError {
    pub kind: &'static str,
    pub message: String,
    pub exit: u8,
}

impl CliError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            exit: 1,
        }
    }

    pub fn bad_argument(message: impl Into<String>) -> Self {
        Self::new("bad-argument", message)
    }

    /// For a command that names a record that the database lacks.
    #[allow(dead_code)]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not-found", message)
    }
}

impl From<IdentityError> for CliError {
    fn from(error: IdentityError) -> Self {
        Self::new(error.kind(), error.to_string())
    }
}

impl From<ScippyError> for CliError {
    fn from(error: ScippyError) -> Self {
        let kind = match error {
            ScippyError::Io { .. } => "io",
            _ => "storage",
        };
        Self::new(kind, error.to_string())
    }
}

/// The `value` of a list: the kept items, the count before the limit, and whether it cut items.
pub fn list_value(mut items: Vec<Value>, limit: Option<usize>) -> Value {
    let total = items.len();
    if let Some(limit) = limit {
        items.truncate(limit);
    }
    json!({"items": items, "total": total, "truncated": items.len() < total})
}

/// Open the database, read one revision, and print the result of `run`.
///
/// The human mode prints `Output::human`, and an error as `scippy: {error}` on the standard
/// error stream. The `--json` mode prints one envelope to the standard output stream, also
/// for an error. The exit code is 0, 1 (error), 4 (no revision), or `Output::exit`.
pub fn execute(
    command: &'static str,
    common: &Common,
    run: impl FnOnce(&View<'_>) -> Result<Output, CliError>,
) -> ExitCode {
    match read(command, common, run) {
        Ok((envelope, output)) => {
            if common.json {
                println!("{envelope}");
            } else {
                print!("{}", output.human);
            }
            ExitCode::from(output.exit)
        }
        Err(error) => {
            if common.json {
                println!("{}", error_envelope(command, &error));
            } else {
                eprintln!("scippy: {}", error.message);
            }
            ExitCode::from(error.exit)
        }
    }
}

fn read(
    command: &'static str,
    common: &Common,
    run: impl FnOnce(&View<'_>) -> Result<Output, CliError>,
) -> Result<(Value, Output), CliError> {
    let path = &common.db;
    match Metalayer::probe(path)? {
        Probe::Missing => {
            return Err(CliError::new(
                "database-missing",
                format!(
                    "no database at {}; run `scippy index --metalayer` first",
                    path.display()
                ),
            ));
        }
        Probe::Version(version) if version == SCHEMA_VERSION => {}
        Probe::Version(version) => {
            return Err(CliError::new(
                "schema-mismatch",
                format!(
                    "the metalayer database has format version {version}, but this build reads version {SCHEMA_VERSION}; run `scippy index` to rebuild it"
                ),
            ));
        }
        _ => {
            return Err(CliError::new(
                "schema-mismatch",
                format!("{} is not a Scippy metalayer database", path.display()),
            ));
        }
    }
    let mut layer = Metalayer::open_read_only(path)?;
    let Some(view) = layer.view()? else {
        return Err(CliError {
            kind: "no-revision",
            message: "the database holds no revision; run `scippy index --metalayer` first".into(),
            exit: 4,
        });
    };
    let output = run(&view)?;
    let revision = view.revision();
    let root_set = view.root_set()?;
    let mut envelope = envelope_head(command, true);
    envelope.extend(
        json!({
            "revision": {
                "id": revision.id,
                "input_digest": revision.input_digest,
                "created_at": revision.created_at,
                "tool_version": revision.tool_version,
                "scip_sha256": revision.scip_sha256,
                "scip_size": revision.scip_size,
            },
            "root_set": {
                "key": root_set.key,
                "project_root": root_set.project_root,
                "shards": root_set.shards,
            },
            "input_complete": revision.input_complete,
            "analysis_exact": revision.analysis_exact,
            "position_encoding": revision.position_encoding,
            "schema_version": view.schema_version()?,
            "value": output.value,
        })
        .as_object()
        .cloned()
        .unwrap_or_default(),
    );
    Ok((Value::Object(envelope), output))
}

/// The fields that both envelopes hold.
fn envelope_head(command: &str, ok: bool) -> serde_json::Map<String, Value> {
    json!({
        "format_version": FORMAT_VERSION,
        "tool": {"name": "scippy", "version": env!("CARGO_PKG_VERSION")},
        "command": command,
        "ok": ok,
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
}

fn error_envelope(command: &str, error: &CliError) -> Value {
    let mut envelope = envelope_head(command, false);
    envelope.insert(
        "error".into(),
        json!({"kind": error.kind, "message": error.message}),
    );
    Value::Object(envelope)
}

/// The environment variable that selects fixed ids and a fixed clock. It is for tests only.
///
/// The value is `TIME` or `TIME,START`: `TIME` is the clock (`2026-01-01T00:00:00Z`), and
/// `START` is the first id number (default 1). Ids are `00000000-0000-7000-8000-<START in hex>`
/// and count up. Without the variable, the commands use UUIDv7 ids and the system clock.
pub const FIXED_IDS_VAR: &str = "SCIPPY_TEST_FIXED_IDS";

fn id_source() -> Box<dyn IdSource> {
    let Some(value) = std::env::var_os(FIXED_IDS_VAR) else {
        return Box::new(SystemIds);
    };
    let value = value.to_string_lossy();
    let (time, start) = match value.split_once(',') {
        Some((time, start)) => (time, start.trim().parse().unwrap_or(1)),
        None => (value.as_ref(), 1),
    };
    Box::new(FixedIds::starting_at(start, time))
}

/// Open the identity store and print the result of `run`.
///
/// The envelope has the `identity` block (`path`, `version`, `bound_input_digest`) and no
/// revision fields, because the command never reads the facts store. An error envelope is the
/// same as for the facts commands. The exit code is 0, or 1 for an error.
pub fn execute_identity(
    command: &'static str,
    args: &IdentityArgs,
    access: Access,
    run: impl FnOnce(&mut Identity, &dyn IdSource) -> Result<Output, CliError>,
) -> ExitCode {
    match read_identity(command, args, access, run) {
        Ok((envelope, output)) => {
            if args.json {
                println!("{envelope}");
            } else {
                print!("{}", output.human);
            }
            ExitCode::from(output.exit)
        }
        Err(error) => {
            if args.json {
                println!("{}", error_envelope(command, &error));
            } else {
                eprintln!("scippy: {}", error.message);
            }
            ExitCode::from(error.exit)
        }
    }
}

fn read_identity(
    command: &'static str,
    args: &IdentityArgs,
    access: Access,
    run: impl FnOnce(&mut Identity, &dyn IdSource) -> Result<Output, CliError>,
) -> Result<(Value, Output), CliError> {
    let path = args.store_path();
    match Identity::probe(&path)? {
        IdentityProbe::Missing => {
            return Err(CliError::new(
                "no-identity-store",
                format!(
                    "no identity store at {}; a publish creates it",
                    path.display()
                ),
            ));
        }
        IdentityProbe::Version(version) if version == IDENTITY_VERSION => {}
        IdentityProbe::Version(version)
            if version < IDENTITY_VERSION && access == Access::Write => {}
        IdentityProbe::Version(version) => {
            let advice = if version > IDENTITY_VERSION {
                "use a newer scippy"
            } else {
                "run a command that writes the store to migrate it"
            };
            return Err(CliError::new(
                "identity-schema-mismatch",
                format!(
                    "the identity store has format version {version}, but this build reads version {IDENTITY_VERSION}; {advice}"
                ),
            ));
        }
        _ => {
            return Err(CliError::new(
                "identity-schema-mismatch",
                format!("{} is not a Scippy identity store", path.display()),
            ));
        }
    }
    let mut store = match access {
        Access::Read => Identity::open_read_only(&path)?,
        Access::Write => Identity::open(&path, &IdentityOptions::default())?,
    };
    let ids = id_source();
    let output = run(&mut store, ids.as_ref())?;
    let mut envelope = envelope_head(command, true);
    envelope.extend(
        json!({
            "identity": {
                "path": path.to_string_lossy(),
                "version": store.version(),
                "bound_input_digest": store.bound_input_digest()?,
            },
            "value": output.value,
        })
        .as_object()
        .cloned()
        .unwrap_or_default(),
    );
    Ok((Value::Object(envelope), output))
}
