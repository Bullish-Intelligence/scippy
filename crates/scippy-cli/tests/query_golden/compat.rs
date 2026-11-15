//! The compatibility policy: a reader ignores unknown fields and unknown enum values.

use serde_json::{Value, json};

use crate::harness::{golden_dir, read_golden, validate};

/// Add an unknown field to every object, and an unknown value to every known enum field.
fn extend(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if matches!(key.as_str(), "status" | "kind" | "state" | "outcome")
                    && child.is_string()
                {
                    *child = json!("a-future-value");
                }
                extend(child);
            }
            map.insert("a_future_field".into(), json!({"nested": [1, 2, 3]}));
        }
        Value::Array(items) => items.iter_mut().for_each(extend),
        _ => {}
    }
}

#[test]
fn unknown_fields_and_values_still_validate() {
    let mut seen = 0;
    for path in golden_dir() {
        let mut envelope = read_golden(&path);
        extend(&mut envelope);
        // A new command is valid too.
        envelope["command"] = json!("a-future-command");
        if let Err(errors) = validate(&envelope) {
            panic!("{}: {errors}", path.display());
        }
        seen += 1;
    }
    assert!(seen > 0, "no golden files");
}

#[test]
fn the_schema_rejects_a_broken_envelope() {
    let mut envelope = read_golden(&golden_dir().next().unwrap());
    envelope.as_object_mut().unwrap().remove("ok");
    assert!(validate(&envelope).is_err());
    let mut envelope = read_golden(&golden_dir().next().unwrap());
    envelope["format_version"] = json!(2);
    assert!(validate(&envelope).is_err());
}

#[test]
fn the_value_schema_rejects_a_retyped_field() {
    let mut files = read_golden(
        &golden_dir()
            .find(|p| p.file_name().unwrap() == "files-default.json")
            .unwrap(),
    );
    files["value"]["items"][0]["size"] = json!("big");
    assert!(validate(&files).is_err());
}
