//! What every `metalayer` command shares: the common options, the envelope, and the error
//! rules. A command module calls [`execute`] with a closure that reads one [`View`].

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use scippy_core::ScippyError;
use scippy_metalayer::{Metalayer, Probe, SCHEMA_VERSION, View};
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
