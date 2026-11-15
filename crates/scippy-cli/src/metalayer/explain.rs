//! `scippy metalayer explain`: why does file F depend on symbol S.
//!
//! The value holds the file, the target, the matching relations, and the supports. Each support
//! has its source line and column, and the reference at the same span. The command does not use
//! `--limit`: the value is one record, not a list.

use std::fmt::Write;
use std::process::ExitCode;

use clap::Args as ClapArgs;
use scippy_metalayer::{
    FileRecord, Outcome, ReferenceRecord, RelationQuery, Source, SymbolRecord, SymbolRef, View,
};
use serde_json::{Value, json};

use super::common::{CliError, Common, Output, execute};

/// Explain why a file depends on a symbol.
#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    common: Common,
    /// The file that depends on the symbol.
    #[arg(long, value_name = "PATH")]
    from: String,
    /// The symbol string of the target.
    #[arg(long, value_name = "SYMBOL")]
    to: String,
    /// The file of the target, for a local symbol (`local N`).
    #[arg(long, value_name = "PATH")]
    to_file: Option<String>,
}

/// A symbol as the contract names it: the symbol string, and the file for a local symbol.
fn symbol_value(symbol: &SymbolRef) -> Value {
    json!({"symbol": symbol.symbol, "file": symbol.file})
}

/// The text of a symbol for the human mode. A local symbol shows its file.
fn symbol_text(symbol: &SymbolRef) -> String {
    match &symbol.file {
        Some(file) => format!("{} ({file})", symbol.symbol),
        None => symbol.symbol.clone(),
    }
}

/// The 1-based line and column of a byte offset, or `None` when the offset is not a position
/// in the text. The column counts characters, not bytes.
fn position(text: &str, offset: u32) -> Option<(usize, usize)> {
    let before = text.get(..usize::try_from(offset).ok()?)?;
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    Some((line, before[start..].chars().count() + 1))
}

/// The state of the source file against the stored hash, and its text when it matches.
struct Text {
    state: &'static str,
    text: Option<String>,
}

impl Text {
    fn read(view: &View<'_>, file: &FileRecord, needed: bool) -> Result<Self, CliError> {
        // A file without a stored hash has no relation and no reference, so nothing needs it.
        if !needed || file.sha256.is_none() {
            return Ok(Self {
                state: "ok",
                text: None,
            });
        }
        let root = view.root_set()?.project_root;
        Ok(
            match view.source(std::path::Path::new(&root), &file.path)? {
                Source::Verified(text) => Self {
                    state: "ok",
                    text: Some(text),
                },
                Source::Changed => Self {
                    state: "changed",
                    text: None,
                },
                Source::Missing => Self {
                    state: "missing",
                    text: None,
                },
                _ => Self {
                    state: "changed",
                    text: None,
                },
            },
        )
    }

    /// The line and column of a span start, or two nulls.
    fn at(&self, span: (u32, u32)) -> (Value, Value) {
        match self.text.as_deref().and_then(|t| position(t, span.0)) {
            Some((line, column)) => (json!(line), json!(column)),
            None => (Value::Null, Value::Null),
        }
    }
}

/// The reference record of the contract. The candidates are sorted by symbol string.
fn reference_value(reference: &ReferenceRecord) -> Value {
    let (outcome, reason, candidates) = match &reference.outcome {
        Outcome::Resolved(_) => ("resolved", None, Vec::new()),
        Outcome::Ambiguous(candidates) => {
            let mut sorted = candidates.clone();
            sorted.sort();
            sorted.dedup();
            ("ambiguous", None, sorted)
        }
        Outcome::Unresolved(reason) => ("unresolved", Some(reason.clone()), Vec::new()),
        Outcome::Bounded(bound) => ("bounded", Some(bound.clone()), Vec::new()),
        _ => ("unknown", None, Vec::new()),
    };
    json!({
        "kind": reference.kind.map(|k| k.code()),
        "outcome": outcome,
        "reason": reason,
        "limited": reference.limited.map(|b| b.code()),
        "candidates": candidates.iter().map(symbol_value).collect::<Vec<_>>(),
    })
}

fn target_value(target: &SymbolRecord) -> Value {
    json!({
        "symbol": target.symbol.symbol,
        "file": target.symbol.file,
        "kind": target.kind,
        "display_name": target.display_name,
        "external": target.external,
        "defined_in": target.defined_in,
    })
}

fn file_value(file: &FileRecord) -> Value {
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

pub fn run(args: Args) -> ExitCode {
    execute("explain", &args.common, |view| {
        let file = view.file(&args.from)?.ok_or_else(|| {
            CliError::not_found(format!("no file {:?} in the manifest", args.from))
        })?;
        let wanted = SymbolRef {
            symbol: args.to.clone(),
            file: args.to_file.clone(),
        };
        let target = view.symbol(&wanted)?.ok_or_else(|| {
            let hint = if args.to.starts_with("local ") && args.to_file.is_none() {
                "; a local symbol needs --to-file"
            } else {
                ""
            };
            CliError::not_found(format!(
                "no symbol {:?} in this revision{hint}",
                symbol_text(&wanted)
            ))
        })?;

        let relations = view.file_references(RelationQuery {
            path: Some(file.path.clone()),
            target: Some(wanted.clone()),
            kind: None,
        })?;
        let supports = view.supports(&file.path, &wanted)?;
        let ambiguous = view.ambiguous_with(&file.path, &wanted)?;
        let text = Text::read(view, &file, !supports.is_empty() || !ambiguous.is_empty())?;

        let support_items = supports
            .iter()
            .map(|s| {
                let (line, column) = text.at(s.span);
                let reference = view.references_at(&file.path, s.span)?;
                Ok(json!({
                    "kind": s.kind.as_str(),
                    "span": [s.span.0, s.span.1],
                    "limited": s.limited.map(|b| b.code()),
                    "line": line,
                    "column": column,
                    "reference": reference.as_ref().map(reference_value),
                }))
            })
            .collect::<Result<Vec<_>, CliError>>()?;
        let ambiguous_items: Vec<Value> = ambiguous
            .iter()
            .map(|r| {
                let (line, column) = text.at(r.span);
                json!({
                    "span": [r.span.0, r.span.1],
                    "line": line,
                    "column": column,
                    "reference": reference_value(r),
                })
            })
            .collect();
        let unresolved_in_file = if relations.is_empty() {
            let count = view
                .references(scippy_metalayer::ReferenceQuery {
                    path: Some(file.path.clone()),
                    outcome: None,
                })?
                .iter()
                .filter(|r| matches!(r.outcome, Outcome::Unresolved(_) | Outcome::Ambiguous(_)))
                .count();
            json!(count)
        } else {
            Value::Null
        };

        let mut human = String::new();
        let human_error = |e: std::fmt::Error| CliError::new("io", e.to_string());
        write_human(
            &mut human,
            &file,
            &target,
            &relations,
            &support_items,
            &ambiguous_items,
            &unresolved_in_file,
            text.state,
        )
        .map_err(human_error)?;

        let value = json!({
            "file": file_value(&file),
            "target": target_value(&target),
            "source": text.state,
            "relations": relations
                .iter()
                .map(|r| json!({"kind": r.kind.as_str(), "supports": r.supports, "limited": r.limited}))
                .collect::<Vec<_>>(),
            "supports": support_items,
            "ambiguous": ambiguous_items,
            "unresolved_in_file": unresolved_in_file,
        });
        Ok(Output::new(value, human))
    })
}

/// The place of an item for the human mode: `path:line:column`, or `path@start-end` when the
/// source does not match the stored hash.
fn place(path: &str, item: &Value) -> String {
    match (item["line"].as_u64(), item["column"].as_u64()) {
        (Some(line), Some(column)) => format!("{path}:{line}:{column}"),
        _ => format!("{path}@{}-{}", item["span"][0], item["span"][1]),
    }
}

fn reference_text(reference: &Value) -> String {
    let mut text = format!(
        "{} {}",
        reference["kind"].as_str().unwrap_or("-"),
        reference["outcome"].as_str().unwrap_or("-")
    );
    if let Some(reason) = reference["reason"].as_str() {
        let _ = write!(text, " {reason}");
    }
    if let Some(limited) = reference["limited"].as_str() {
        let _ = write!(text, " limited={limited}");
    }
    text
}

#[allow(clippy::too_many_arguments)]
fn write_human(
    out: &mut String,
    file: &FileRecord,
    target: &SymbolRecord,
    relations: &[scippy_metalayer::FileReferenceRecord],
    supports: &[Value],
    ambiguous: &[Value],
    unresolved_in_file: &Value,
    source: &str,
) -> std::fmt::Result {
    writeln!(
        out,
        "{}  depends on  {}  {} {}{}",
        file.path,
        symbol_text(&target.symbol),
        target.kind,
        if target.display_name.is_empty() {
            "-"
        } else {
            &target.display_name
        },
        match &target.defined_in {
            Some(path) => format!("  defined in {path}"),
            None => "  external".to_owned(),
        },
    )?;
    if source != "ok" {
        writeln!(out, "source  {source}; the lines and columns are unknown")?;
    }
    if relations.is_empty() {
        writeln!(out, "no relation")?;
    }
    for relation in relations {
        writeln!(
            out,
            "relation  {}  x{}  limited {}",
            relation.kind.as_str(),
            relation.supports,
            relation.limited
        )?;
    }
    for support in supports {
        write!(
            out,
            "  {}  {}",
            place(&file.path, support),
            support["kind"].as_str().unwrap_or("-")
        )?;
        if let Some(limited) = support["limited"].as_str() {
            write!(out, "  limited={limited}")?;
        }
        if support["reference"].is_object() {
            write!(out, "  [{}]", reference_text(&support["reference"]))?;
        }
        writeln!(out)?;
    }
    for item in ambiguous {
        write!(
            out,
            "  maybe  {}  [{}]  candidates",
            place(&file.path, item),
            reference_text(&item["reference"]),
        )?;
        for candidate in item["reference"]["candidates"]
            .as_array()
            .into_iter()
            .flatten()
        {
            write!(out, " {}", candidate["symbol"].as_str().unwrap_or("-"))?;
        }
        writeln!(out)?;
    }
    if let Some(count) = unresolved_in_file.as_u64() {
        writeln!(
            out,
            "unresolved or ambiguous references in the file  {count}"
        )?;
    }
    Ok(())
}
