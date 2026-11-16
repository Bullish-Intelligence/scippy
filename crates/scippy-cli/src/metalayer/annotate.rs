//! `scippy metalayer annotate`: the user annotations on entities, and the review of moved,
//! ambiguous, and orphaned entities. The group is the first writer of the metalayer. It reads
//! and writes the identity store only, so it needs no facts store.

use std::fmt::Write;
use std::process::ExitCode;

use clap::{Args as ClapArgs, Subcommand};
use scippy_metalayer::{AcceptAction, AnnotationFilter, BindOutcome, IdSource, Identity, Tier};
use serde_json::json;

use super::common::{Access, CliError, IdentityArgs, Output, execute_identity, list_value};
use super::entity::{annotation_json, detail_summary, first_line, locator_text, summary_json};

/// Add, list, remove, and review annotations.
#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    command: Sub,
}

#[derive(Subcommand, Debug)]
enum Sub {
    Add(AddArgs),
    List(ListArgs),
    Rm(RmArgs),
    Status(StatusArgs),
    Accept(AcceptArgs),
    Reject(RejectArgs),
}

/// Add an annotation to an entity.
#[derive(ClapArgs, Debug)]
struct AddArgs {
    #[command(flatten)]
    ids: IdentityArgs,
    /// An entity id, a symbol, `path::Outer.Inner`, `note.md`, `note.md#H1/H2`, or `note.md^id`.
    locator: String,
    /// The text of the annotation.
    #[arg(long, value_name = "TEXT")]
    body: String,
    /// A free label.
    #[arg(long, value_name = "T")]
    tag: Option<String>,
    /// The name of the author.
    #[arg(long, value_name = "A")]
    author: Option<String>,
}

/// List annotations, sorted by path, name, creation time, and id.
#[derive(ClapArgs, Debug)]
struct ListArgs {
    #[command(flatten)]
    ids: IdentityArgs,
    /// Keep the annotations of one entity. Any locator form works.
    #[arg(long, value_name = "ID")]
    entity: Option<String>,
    /// Keep the annotations whose entity has this path.
    #[arg(long, value_name = "P")]
    path: Option<String>,
    /// Keep the annotations whose entity has this state: matched, moved, ambiguous, or orphaned.
    #[arg(long, value_name = "S")]
    status: Option<String>,
}

/// Remove one annotation.
#[derive(ClapArgs, Debug)]
struct RmArgs {
    #[command(flatten)]
    ids: IdentityArgs,
    /// The id of the annotation.
    annotation_id: String,
}

/// List the entities that need review: moved, ambiguous, and orphaned.
#[derive(ClapArgs, Debug)]
struct StatusArgs {
    #[command(flatten)]
    ids: IdentityArgs,
    /// Keep the entities that have an annotation.
    #[arg(long)]
    annotated: bool,
}

/// Accept a moved entity, bind an ambiguous entity to a candidate, or accept all moved entities
/// of one tier.
#[derive(ClapArgs, Debug)]
struct AcceptArgs {
    #[command(flatten)]
    ids: IdentityArgs,
    /// An entity id or another locator.
    #[arg(required_unless_present = "all")]
    entity: Option<String>,
    /// For an ambiguous entity: the key of the candidate to bind.
    #[arg(long, value_name = "KEY", conflicts_with = "all")]
    candidate: Option<String>,
    /// Accept every moved entity of the tier in `--tier`.
    #[arg(long, requires = "tier", conflicts_with = "entity")]
    all: bool,
    /// The tier for `--all`: T1h, T2f, T2, T3, T4, T5, M2, M3, M4, M5, or Param.
    #[arg(long, value_name = "T", requires = "all")]
    tier: Option<String>,
}

/// Reject a moved or ambiguous entity: add a veto, and make the entity orphaned.
#[derive(ClapArgs, Debug)]
struct RejectArgs {
    #[command(flatten)]
    ids: IdentityArgs,
    /// An entity id or another locator.
    entity: String,
}

pub fn run(args: Args) -> ExitCode {
    match args.command {
        Sub::Add(a) => execute_identity("annotate add", &a.ids, Access::Write, |s, ids| {
            add(s, ids, &a)
        }),
        Sub::List(a) => execute_identity("annotate list", &a.ids, Access::Read, |s, _| list(s, &a)),
        Sub::Rm(a) => execute_identity("annotate rm", &a.ids, Access::Write, |s, _| rm(s, &a)),
        Sub::Status(a) => execute_identity("annotate status", &a.ids, Access::Read, |s, _| {
            status(s, &a)
        }),
        Sub::Accept(a) => execute_identity("annotate accept", &a.ids, Access::Write, |s, _| {
            accept(s, &a)
        }),
        Sub::Reject(a) => execute_identity("annotate reject", &a.ids, Access::Write, |s, _| {
            reject(s, &a)
        }),
    }
}

fn io(error: std::fmt::Error) -> CliError {
    CliError::new("io", error.to_string())
}

fn add(store: &mut Identity, ids: &dyn IdSource, args: &AddArgs) -> Result<Output, CliError> {
    let annotation = store.add(
        ids,
        &args.locator,
        &args.body,
        args.tag.as_deref(),
        args.author.as_deref(),
    )?;
    let detail = store.detail(&annotation.entity_id)?;
    let human = format!(
        "added  {}  {}\n",
        annotation.id,
        locator_text(&detail.entity, detail.locator())
    );
    Ok(Output::new(
        json!({"annotation": annotation_json(&annotation), "entity": detail_summary(&detail)}),
        human,
    ))
}

fn list(store: &mut Identity, args: &ListArgs) -> Result<Output, CliError> {
    let mut filter = AnnotationFilter::default();
    filter.entity.clone_from(&args.entity);
    filter.path.clone_from(&args.path);
    if let Some(status) = &args.status {
        filter.status = Some(BindOutcome::parse(status).ok_or_else(|| {
            CliError::bad_argument(format!(
                "unknown status {status:?}; use one of matched, moved, ambiguous, orphaned"
            ))
        })?);
    }
    let entries = store.list(&filter)?;
    let mut text = String::new();
    let mut items = Vec::new();
    for entry in &entries {
        writeln!(
            text,
            "{}  {}  {}  {}  {}  {}",
            entry.annotation.id,
            entry.annotation.created_at,
            entry.state.as_str(),
            locator_text(&entry.entity, entry.locator.as_ref()),
            entry.annotation.tag.as_deref().unwrap_or("-"),
            first_line(&entry.annotation.body)
        )
        .map_err(io)?;
        items.push(json!({
            "annotation": annotation_json(&entry.annotation),
            "entity": summary_json(&entry.entity, entry.locator.as_ref(), entry.state, entry.tier),
        }));
    }
    Ok(Output::new(list_value(items, None), text))
}

fn rm(store: &mut Identity, args: &RmArgs) -> Result<Output, CliError> {
    let annotation = store.remove(&args.annotation_id)?;
    Ok(Output::new(
        json!({"removed": annotation_json(&annotation)}),
        format!("removed  {}\n", annotation.id),
    ))
}

fn status(store: &mut Identity, args: &StatusArgs) -> Result<Output, CliError> {
    let mut text = String::new();
    let mut items = Vec::new();
    for entry in store.status()? {
        if args.annotated && entry.annotations == 0 {
            continue;
        }
        writeln!(
            text,
            "{}  {}  {}  {}  annotations={}",
            entry.outcome.outcome.as_str(),
            entry.outcome.tier.map_or("-", Tier::as_str),
            entry.entity.id,
            locator_text(&entry.entity, entry.locator.as_ref()),
            entry.annotations
        )
        .map_err(io)?;
        items.push(json!({
            "entity": summary_json(
                &entry.entity,
                entry.locator.as_ref(),
                entry.outcome.outcome,
                entry.outcome.tier,
            ),
            "candidates": entry.outcome.candidates,
            "since_digest": entry.outcome.since_digest,
            "annotations": entry.annotations,
        }));
    }
    Ok(Output::new(list_value(items, None), text))
}

/// Read a tier name. The match ignores case.
fn parse_tier(text: &str) -> Result<Tier, CliError> {
    const TIERS: [Tier; 12] = [
        Tier::T1,
        Tier::T1H,
        Tier::T2F,
        Tier::T2,
        Tier::T3,
        Tier::T4,
        Tier::T5,
        Tier::M2,
        Tier::M3,
        Tier::M4,
        Tier::M5,
        Tier::Param,
    ];
    TIERS
        .into_iter()
        .find(|t| t.as_str().eq_ignore_ascii_case(text))
        .ok_or_else(|| {
            CliError::bad_argument(format!(
                "unknown tier {text:?}; use one of {}",
                TIERS.map(Tier::as_str).join(", ")
            ))
        })
}

fn accept(store: &mut Identity, args: &AcceptArgs) -> Result<Output, CliError> {
    if args.all {
        let tier = parse_tier(args.tier.as_deref().unwrap_or_default())?;
        let accepted = store.accept_all(tier)?;
        let mut human = String::new();
        for id in &accepted {
            writeln!(human, "accepted  {id}").map_err(io)?;
        }
        return Ok(Output::new(
            json!({"tier": tier.as_str(), "accepted": accepted, "total": accepted.len()}),
            human,
        ));
    }
    let locator = args.entity.as_deref().unwrap_or_default();
    let result = store.accept(locator, args.candidate.as_deref())?;
    let detail = store.detail(&result.entity_id)?;
    let (action, candidate, removed) = match &result.action {
        AcceptAction::Cleared => ("cleared", None, None),
        AcceptAction::Bound {
            candidate,
            removed_entity,
        } => (
            "bound",
            Some(candidate.as_str()),
            Some(removed_entity.as_str()),
        ),
        _ => ("unknown", None, None),
    };
    let human = format!(
        "{action}  {}  {}\n",
        result.entity_id,
        locator_text(&detail.entity, detail.locator())
    );
    Ok(Output::new(
        json!({
            "entity": detail_summary(&detail),
            "action": action,
            "candidate": candidate,
            "removed_entity": removed,
        }),
        human,
    ))
}

fn reject(store: &mut Identity, args: &RejectArgs) -> Result<Output, CliError> {
    let entity = store.resolve(&args.entity)?;
    let vetoed = store.reject(&entity.id)?;
    let detail = store.detail(&entity.id)?;
    let human = format!(
        "orphaned  {}  {}\n",
        entity.id,
        locator_text(&detail.entity, detail.locator())
    );
    Ok(Output::new(
        json!({"entity": detail_summary(&detail), "vetoed": vetoed}),
        human,
    ))
}
