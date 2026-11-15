//! `scippy metalayer files`: the input manifest.

use std::fmt::Write;
use std::process::ExitCode;

use clap::Args as ClapArgs;
use scippy_metalayer::FileRecord;
use serde_json::{Value, json};

use super::common::{CliError, Common, Output, execute, list_value};

/// The statuses that a file can have. A filter with another value is a bad argument.
const STATUSES: [&str; 4] = ["indexed", "skipped", "failed", "shadowed"];

/// List the input manifest, sorted by path.
#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    common: Common,
    /// Keep the files with this status: indexed, skipped, failed, or shadowed.
    #[arg(long, value_name = "S")]
    status: Option<String>,
    /// Keep the files of this language.
    #[arg(long, value_name = "L")]
    language: Option<String>,
    /// Keep the files at this path or below this directory.
    #[arg(long, value_name = "DIR")]
    prefix: Option<String>,
}

fn item(file: &FileRecord) -> Value {
    json!({
        "path": file.path,
        "status": file.status.as_str(),
        "reason": file.reason,
        "pack": file.pack,
        "language": file.language,
        "sha256": file.sha256,
        "size": file.size,
        "stale": file.stale,
    })
}

/// Whether `path` is `prefix` or lies below the directory `prefix`.
fn under(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    prefix.is_empty()
        || path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub fn run(args: Args) -> ExitCode {
    execute("files", &args.common, |view| {
        if let Some(status) = &args.status
            && !STATUSES.contains(&status.as_str())
        {
            return Err(CliError::bad_argument(format!(
                "unknown status {status:?}; use one of {}",
                STATUSES.join(", ")
            )));
        }
        let files: Vec<FileRecord> = view
            .files(None)?
            .into_iter()
            .filter(|f| {
                args.status
                    .as_deref()
                    .is_none_or(|s| f.status.as_str() == s)
            })
            .filter(|f| {
                args.language
                    .as_deref()
                    .is_none_or(|l| f.language.as_deref() == Some(l))
            })
            .filter(|f| args.prefix.as_deref().is_none_or(|p| under(&f.path, p)))
            .collect();
        let value = list_value(files.iter().map(item).collect(), args.common.limit);
        let kept = value["items"].as_array().map_or(0, Vec::len);
        let mut text = String::new();
        for file in files.iter().take(kept) {
            write!(
                text,
                "{}  {}  {}  {}",
                file.path,
                file.status.as_str(),
                file.language.as_deref().unwrap_or("-"),
                file.size.map_or("-".to_owned(), |n| n.to_string()),
            )
            .and_then(|()| {
                if file.stale {
                    write!(text, "  stale")?;
                }
                if let Some(reason) = &file.reason {
                    write!(text, "  {reason}")?;
                }
                writeln!(text)
            })
            .map_err(|e| CliError::new("io", e.to_string()))?;
        }
        if kept < files.len() {
            let _ = writeln!(text, "# truncated: {kept} of {}", files.len());
        }
        Ok(Output::new(value, text))
    })
}
