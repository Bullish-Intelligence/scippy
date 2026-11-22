//! Versioned JSON messages for a stateful source-fact resolver.

use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::process::ExitCode;

use scippy_loci::Record;
use scippy_loci::facts::{Change, NotesIndex, Policy, Profile, SourceFacts};
use serde_json::{Value, json};

const LIMIT: u64 = 256 * 1024 * 1024;

fn records(rows: &BTreeMap<String, Vec<Record>>) -> Value {
    json!(
        rows.iter()
            .map(|(path, rows)| (
                path,
                rows.iter()
                    .map(|r| {
                        let mut value = r.to_json();
                        value["state"] = json!(r.state.name());
                        value["target_path"] = json!(r.target_path);
                        value["anchor_state"] = json!(r.anchor_state);
                        value["anchor_kind"] = json!(r.anchor_kind);
                        value["anchor_span"] = json!(r.anchor_span);
                        value["parts"] = json!(r.parts);
                        value
                    })
                    .collect::<Vec<_>>()
            ))
            .collect::<BTreeMap<_, _>>()
    )
}

fn field<T: serde::de::DeserializeOwned>(request: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(
        request
            .get(name)
            .cloned()
            .ok_or_else(|| format!("Missing field: {name}."))?,
    )
    .map_err(|e| e.to_string())
}

fn request(index: &mut Option<NotesIndex>, value: &Value) -> Result<Value, String> {
    if value["format_version"] != 1 {
        return Err("Unsupported message format version.".into());
    }
    match value["command"].as_str() {
        Some("reset") => {
            let base: u64 = field(value, "base_generation")?;
            if base != index.as_ref().map_or(0, NotesIndex::generation) {
                return Err("The index generation changed.".into());
            }
            let profile: Profile = field(value, "profile")?;
            let policy: Policy = value
                .get("policy")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let sources: Vec<SourceFacts> = field(value, "sources")?;
            let attachments: Vec<String> = field(value, "attachments")?;
            let mut next = NotesIndex::new(profile, policy, sources, attachments)?;
            next.set_generation(base + 1);
            let result = json!({"generation":next.generation(),"changed":records(next.records()),"removed":[]});
            *index = Some(next);
            Ok(result)
        }
        Some("update") => {
            let current = index.as_mut().ok_or("Reset the index before an update.")?;
            let base: u64 = field(value, "base_generation")?;
            let profile: Profile = field(value, "profile")?;
            let changes: Vec<Change> = field(value, "changes")?;
            let attachments = value
                .get("attachments")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|e| e.to_string())?;
            let update = current.update(base, &profile, changes, attachments)?;
            Ok(
                json!({"generation":update.generation,"changed":records(&update.changed),"removed":update.removed}),
            )
        }
        Some("snapshot") => {
            let current = index.as_ref().ok_or("Reset the index before a query.")?;
            Ok(
                json!({"generation":current.generation(),"changed":records(current.records()),"removed":[]}),
            )
        }
        Some("ping") => Ok(json!({"generation":index.as_ref().map_or(0, NotesIndex::generation)})),
        _ => Err("Unknown notes command.".into()),
    }
}

pub fn run() -> ExitCode {
    let mut index = None;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    loop {
        let mut line = Vec::new();
        let read = match input.by_ref().take(LIMIT + 1).read_until(b'\n', &mut line) {
            Ok(0) => return ExitCode::SUCCESS,
            Ok(size) => size,
            Err(_) => return ExitCode::FAILURE,
        };
        let mut id = Value::Null;
        let result = if read as u64 > LIMIT {
            if line.last() != Some(&b'\n') && input.skip_until(b'\n').is_err() {
                return ExitCode::FAILURE;
            }
            Err("The message exceeds 256 mebibytes.".into())
        } else {
            serde_json::from_slice::<Value>(&line)
                .map_err(|e| e.to_string())
                .and_then(|value| {
                    id = value.get("id").cloned().unwrap_or_default();
                    request(&mut index, &value)
                })
        };
        let response = match result {
            Ok(value) => json!({"format_version":1,"id":id,"ok":true,"value":value}),
            Err(message) => {
                json!({"format_version":1,"id":id,"ok":false,"generation":index.as_ref().map_or(0,NotesIndex::generation),"error":{"kind":"refused","message":message}})
            }
        };
        if serde_json::to_writer(&mut output, &response).is_err()
            || output.write_all(b"\n").is_err()
            || output.flush().is_err()
        {
            return ExitCode::FAILURE;
        }
    }
}
