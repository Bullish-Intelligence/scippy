//! `scippy metalayer edges`: single edges, one site each.

use std::fmt::Write;
use std::process::ExitCode;

use clap::Args as ClapArgs;
use scippy_metalayer::{EdgeKind, EdgeQuery, EdgeRecord, SymbolRef, View};
use serde_json::{Value, json};

use super::common::{CliError, Common, Output, execute, list_value};

/// The kinds of an edge, in the order of the contract.
const KINDS: [&str; 6] = [
    "import",
    "inheritance",
    "override",
    "alias",
    "link",
    "embed",
];

/// List single edges, sorted by path, span, kind, source, and target.
#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    common: Common,
    /// Keep this kind: import, inheritance, override, alias, link, or embed. Repeat the option
    /// to keep several kinds.
    #[arg(long, value_name = "K", value_parser = clap::builder::PossibleValuesParser::new(KINDS))]
    kind: Vec<String>,
    /// Keep the edges of this source file.
    #[arg(long, value_name = "PATH")]
    from: Option<String>,
    /// Keep the edges to this target symbol (a SCIP symbol string).
    #[arg(long, value_name = "SYMBOL")]
    to: Option<String>,
    /// The file of a local target. A local symbol needs it.
    #[arg(long, value_name = "PATH", requires = "to")]
    to_file: Option<String>,
    /// Keep the edges with this state: resolved, missing, ambiguous, or none.
    #[arg(long, value_name = "S")]
    state: Option<String>,
}

fn kind_of(text: &str) -> Option<EdgeKind> {
    Some(match text {
        "import" => EdgeKind::Import,
        "inheritance" => EdgeKind::Inheritance,
        "override" => EdgeKind::Override,
        "alias" => EdgeKind::Alias,
        "link" => EdgeKind::Link,
        "embed" => EdgeKind::Embed,
        _ => return None,
    })
}

/// A symbol as `{symbol, file}`. The file is set for a local symbol.
fn symbol(symbol: &Option<SymbolRef>) -> Value {
    symbol
        .as_ref()
        .map_or(Value::Null, |s| json!({"symbol": s.symbol, "file": s.file}))
}

/// The path of the document that defines a symbol, or null.
fn defined_in(view: &View<'_>, symbol: &SymbolRef) -> Result<Value, CliError> {
    Ok(view
        .symbol(symbol)?
        .and_then(|s| s.defined_in)
        .map_or(Value::Null, Value::String))
}

fn item(view: &View<'_>, edge: &EdgeRecord) -> Result<Value, CliError> {
    let site = matches!(edge.kind, EdgeKind::Link | EdgeKind::Embed);
    let target_path = match (&edge.target, site) {
        (Some(target), true) => defined_in(view, target)?,
        _ => Value::Null,
    };
    let anchor = match &edge.anchor {
        None => Value::Null,
        Some(anchor) => {
            let record = view.symbol(anchor)?;
            json!({
                "symbol": anchor.symbol,
                "file": anchor.file,
                "kind": record.as_ref().map(|r| r.kind.clone()),
                "display_name": record.as_ref().map(|r| r.display_name.clone()),
            })
        }
    };
    let mut candidates = Vec::new();
    for candidate in &edge.candidates {
        candidates.push(json!({
            "symbol": candidate.symbol,
            "file": candidate.file,
            "path": defined_in(view, candidate)?,
        }));
    }
    Ok(json!({
        "kind": edge.kind.as_str(),
        "path": edge.path,
        "span": edge.span.map(|(start, end)| [start, end]),
        "source": symbol(&edge.source),
        "target": symbol(&edge.target),
        "target_path": target_path,
        "anchor": anchor,
        "link_kind": edge.link_kind,
        "label": edge.label,
        "state": edge.state,
        "anchor_state": edge.anchor_state,
        "candidates": candidates,
    }))
}

pub fn run(args: Args) -> ExitCode {
    execute("edges", &args.common, |view| {
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
        let edges = view.edges_where(&EdgeQuery {
            kinds,
            path: args.from.clone(),
            target,
            state: args.state.clone(),
        })?;
        let kept = args
            .common
            .limit
            .map_or(edges.len(), |n| n.min(edges.len()));
        let mut items = Vec::with_capacity(edges.len());
        for edge in &edges {
            items.push(item(view, edge)?);
        }
        let value = list_value(items, args.common.limit);
        let mut text = String::new();
        for edge in edges.iter().take(kept) {
            let span = edge
                .span
                .map_or("-".to_owned(), |(start, end)| format!("{start}..{end}"));
            let _ = writeln!(
                text,
                "{}  {span}  {}  {}  {}",
                edge.path,
                edge.kind.as_str(),
                edge.state.as_deref().unwrap_or("-"),
                edge.target
                    .as_ref()
                    .map_or("-", |target| target.symbol.as_str()),
            );
        }
        if kept < edges.len() {
            let _ = writeln!(text, "# truncated: {kept} of {}", edges.len());
        }
        Ok(Output::new(value, text))
    })
}
