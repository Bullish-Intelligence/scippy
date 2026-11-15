//! `scippy metalayer show`: the revision, the flags, and the counts.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::process::ExitCode;

use clap::Args as ClapArgs;
use serde_json::json;

use super::common::{CliError, Common, Output, execute};

/// Print the revision, the root set, the flags, and the counts.
#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    common: Common,
}

pub fn run(args: Args) -> ExitCode {
    execute("show", &args.common, |view| {
        let revision = view.revision();
        let root_set = view.root_set()?;
        let counts = view.counts()?;
        let value = json!({
            "parent": revision.parent,
            "strict": revision.strict,
            "counts": {
                "files": counts.files,
                "symbols": counts.symbols,
                "references": counts.references,
                "edges": counts.edges,
                "diagnostics": counts.diagnostics,
            },
        });
        let group = |map: &BTreeMap<String, u64>| {
            map.iter()
                .map(|(key, n)| format!("{key}={n}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut text = String::new();
        let mut line = |name: &str, value: &dyn std::fmt::Display| {
            writeln!(text, "{name}  {value}").map_err(|e| CliError::new("io", e.to_string()))
        };
        line("revision", &revision.id)?;
        line(
            "parent",
            &revision.parent.map_or("-".to_owned(), |p| p.to_string()),
        )?;
        line("created_at", &revision.created_at)?;
        line("tool_version", &revision.tool_version)?;
        line("input_digest", &revision.input_digest)?;
        line("scip_sha256", &revision.scip_sha256)?;
        line("scip_size", &revision.scip_size)?;
        line("project_root", &root_set.project_root)?;
        line("root_key", &root_set.key)?;
        let shards = if root_set.shards.is_empty() {
            "-".to_owned()
        } else {
            root_set.shards.join(" ")
        };
        line("shards", &shards)?;
        line("input_complete", &revision.input_complete)?;
        line("analysis_exact", &revision.analysis_exact)?;
        line("strict", &revision.strict)?;
        line("position_encoding", &revision.position_encoding)?;
        line("schema_version", &view.schema_version()?)?;
        line("files", &group(&counts.files))?;
        line("symbols", &counts.symbols)?;
        line("references", &group(&counts.references))?;
        line("edges", &group(&counts.edges))?;
        line("diagnostics", &group(&counts.diagnostics))?;
        Ok(Output::new(value, text))
    })
}
