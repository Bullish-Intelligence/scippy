//! `scippy metalayer relations`: the aggregated file-to-symbol relations.

use std::fmt::Write;
use std::process::ExitCode;

use clap::Args as ClapArgs;
use scippy_metalayer::{RelationKind, RelationQuery, RelationRecord, SymbolRef};
use serde_json::{Value, json};

use super::common::{CliError, Common, Output, execute, list_value};

/// The kinds of a relation, in the order of the contract.
const KINDS: [&str; 4] = ["import", "use", "link", "embed"];

/// List the relations from files to symbols, sorted by path, target symbol, and kind.
#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    common: Common,
    /// Keep the relations of this source file.
    #[arg(long, value_name = "PATH")]
    from: Option<String>,
    /// Keep the relations to this target symbol (a SCIP symbol string).
    #[arg(long, value_name = "SYMBOL")]
    to: Option<String>,
    /// The file of a local target. A local symbol needs it.
    #[arg(long, value_name = "PATH", requires = "to")]
    to_file: Option<String>,
    /// Keep the relations to targets with this exact display name.
    #[arg(long, value_name = "NAME")]
    target_name: Option<String>,
    /// Keep this kind: import, use, link, or embed. Repeat the option to keep several kinds.
    #[arg(long, value_name = "KIND", value_parser = clap::builder::PossibleValuesParser::new(KINDS))]
    kind: Vec<String>,
    /// Keep the relations to external targets: no document of the project defines them.
    #[arg(long, conflicts_with = "internal")]
    external: bool,
    /// Keep the relations to internal targets: a document of the project defines them.
    #[arg(long)]
    internal: bool,
}

fn kind_of(text: &str) -> Option<RelationKind> {
    Some(match text {
        "import" => RelationKind::Import,
        "use" => RelationKind::Use,
        "link" => RelationKind::Link,
        "embed" => RelationKind::Embed,
        _ => return None,
    })
}

fn item(relation: &RelationRecord) -> Value {
    let target = &relation.target;
    json!({
        "path": relation.path,
        "target": {
            "symbol": target.symbol.symbol,
            "file": target.symbol.file,
            "kind": target.kind,
            "display_name": target.display_name,
            "external": target.external,
            "defined_in": target.defined_in,
        },
        "kind": relation.kind.as_str(),
        "supports": relation.supports,
        "limited": relation.limited,
    })
}

pub fn run(args: Args) -> ExitCode {
    execute("relations", &args.common, |view| {
        let target = match (&args.to, &args.to_file) {
            (None, _) => None,
            (Some(symbol), Some(file)) => Some(SymbolRef::local(symbol, file)),
            (Some(symbol), None) if symbol.starts_with("local ") => {
                return Err(CliError::bad_argument(
                    "a local symbol needs --to-file, the file that holds it",
                ));
            }
            (Some(symbol), None) => Some(SymbolRef::global(symbol)),
        };
        let kinds = args
            .kind
            .iter()
            .map(|k| kind_of(k))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| CliError::bad_argument("unknown kind"))?;
        let relations = view.relations(RelationQuery {
            path: args.from.clone(),
            target,
            target_name: args.target_name.clone(),
            kinds,
            external: match (args.external, args.internal) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            },
        })?;
        let value = list_value(relations.iter().map(item).collect(), args.common.limit);
        let kept = value["items"].as_array().map_or(0, Vec::len);
        let mut text = String::new();
        for relation in relations.iter().take(kept) {
            let _ = writeln!(
                text,
                "{}  {}  {}  x{}",
                relation.path,
                relation.kind.as_str(),
                relation.target.symbol.symbol,
                relation.supports
            );
        }
        if kept < relations.len() {
            let _ = writeln!(text, "# truncated: {kept} of {}", relations.len());
        }
        Ok(Output::new(value, text))
    })
}
