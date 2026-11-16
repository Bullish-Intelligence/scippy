//! `scippy metalayer entity`: one entity with its locator history, outcome, and annotations.
//!
//! The JSON helpers of this file also serve `annotate`.

use std::fmt::Write;
use std::process::ExitCode;

use clap::Args as ClapArgs;
use scippy_metalayer::{Annotation, BindOutcome, Entity, EntityDetail, Locator, OutcomeRow, Tier};
use serde_json::{Value, json};

use super::common::{Access, CliError, IdentityArgs, Output, execute_identity};

/// Print one entity: its id, locator history, outcome, vetoes, and annotations.
#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    ids: IdentityArgs,
    /// An entity id, a symbol, `path::Outer.Inner`, `note.md`, `note.md#H1/H2`, or `note.md^id`.
    locator: String,
}

/// A locator as JSON. `seq` is 0 for the current locator.
pub fn locator_json(locator: &Locator) -> Value {
    let mut value = locator.to_json();
    if let Some(map) = value.as_object_mut() {
        map.insert("seq".into(), json!(locator.seq));
    }
    value
}

/// An annotation as JSON. The snapshot is parsed JSON, or the stored text if it does not parse.
pub fn annotation_json(annotation: &Annotation) -> Value {
    json!({
        "id": annotation.id,
        "entity_id": annotation.entity_id,
        "created_at": annotation.created_at,
        "author": annotation.author,
        "tag": annotation.tag,
        "body": annotation.body,
        "snapshot": serde_json::from_str::<Value>(&annotation.snapshot)
            .unwrap_or_else(|_| Value::String(annotation.snapshot.clone())),
    })
}

/// The state and the tier of an entity.
fn state_of(outcome: Option<&OutcomeRow>) -> (&'static str, Option<&'static str>) {
    match outcome {
        Some(row) => (row.outcome.as_str(), row.tier.map(Tier::as_str)),
        None => (BindOutcome::Matched.as_str(), None),
    }
}

/// An entity in a short form: id, family, kind, state, tier, and current locator.
pub fn summary_json(
    entity: &Entity,
    locator: Option<&Locator>,
    state: BindOutcome,
    tier: Option<Tier>,
) -> Value {
    json!({
        "id": entity.id,
        "family": entity.family.as_str(),
        "kind": entity.kind.as_str(),
        "state": state.as_str(),
        "tier": tier.map(Tier::as_str),
        "locator": locator.map(locator_json),
    })
}

/// The summary of an entity detail.
pub fn detail_summary(detail: &EntityDetail) -> Value {
    summary_json(
        &detail.entity,
        detail.locator(),
        detail.state(),
        detail.outcome.as_ref().and_then(|o| o.tier),
    )
}

/// The locator text of an entity, or `-` when it has no locator.
pub fn locator_text(entity: &Entity, locator: Option<&Locator>) -> String {
    locator.map_or_else(|| "-".to_owned(), |l| l.display(entity.kind))
}

/// The first line of a text. A longer text gets `...`.
pub fn first_line(text: &str) -> String {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    if lines.next().is_some() {
        format!("{first} ...")
    } else {
        first.to_owned()
    }
}

fn detail_json(detail: &EntityDetail) -> Value {
    let (state, tier) = state_of(detail.outcome.as_ref());
    let entity = &detail.entity;
    json!({
        "id": entity.id,
        "family": entity.family.as_str(),
        "kind": entity.kind.as_str(),
        "parent_id": entity.parent_id,
        "created_at": entity.created_at,
        "outcome": {
            "state": state,
            "tier": tier,
            "candidates": detail.outcome.as_ref().map_or(&[][..], |o| &o.candidates[..]),
            "since_digest": detail.outcome.as_ref().map(|o| &o.since_digest),
        },
        "locator": detail.locator().map(locator_json),
        "locators": detail.locators.iter().map(locator_json).collect::<Vec<_>>(),
        "vetoes": detail.vetoes,
        "annotations": detail.annotations.iter().map(annotation_json).collect::<Vec<_>>(),
    })
}

fn detail_text(detail: &EntityDetail) -> Result<String, std::fmt::Error> {
    let entity = &detail.entity;
    let (state, tier) = state_of(detail.outcome.as_ref());
    let mut text = String::new();
    writeln!(text, "id  {}", entity.id)?;
    writeln!(text, "family  {}", entity.family.as_str())?;
    writeln!(text, "kind  {}", entity.kind.as_str())?;
    writeln!(text, "locator  {}", locator_text(entity, detail.locator()))?;
    writeln!(text, "state  {state}")?;
    if let Some(tier) = tier {
        writeln!(text, "tier  {tier}")?;
    }
    if let Some(row) = &detail.outcome {
        if !row.candidates.is_empty() {
            writeln!(text, "candidates  {}", row.candidates.join(" "))?;
        }
        writeln!(text, "since  {}", row.since_digest)?;
    }
    if !detail.vetoes.is_empty() {
        writeln!(text, "vetoes  {}", detail.vetoes.join(" "))?;
    }
    for locator in &detail.locators {
        writeln!(
            text,
            "locator[{}]  {}  {}",
            locator.seq,
            locator.display(entity.kind),
            locator.key
        )?;
    }
    for annotation in &detail.annotations {
        writeln!(
            text,
            "annotation  {}  {}  {}  {}  {}",
            annotation.id,
            annotation.created_at,
            annotation.tag.as_deref().unwrap_or("-"),
            annotation.author.as_deref().unwrap_or("-"),
            first_line(&annotation.body)
        )?;
    }
    Ok(text)
}

pub fn run(args: Args) -> ExitCode {
    execute_identity("entity", &args.ids, Access::Read, |store, _| {
        let detail = store.detail(&args.locator)?;
        let human = detail_text(&detail).map_err(|e| CliError::new("io", e.to_string()))?;
        Ok(Output::new(detail_json(&detail), human))
    })
}
