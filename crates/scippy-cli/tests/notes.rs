//! A persistent worker refuses bad messages without losing its committed generation.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn protocol_is_persistent_and_refusals_are_atomic() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_scippy"))
        .arg("notes")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut send = |value: Value| -> Value {
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let profile = json!({"format_version":1,"extractor_version":1,"pack_digests":{
        "markdown":format!("sha256:{}","a".repeat(64)),
        "markdown_inline":format!("sha256:{}","b".repeat(64)),
        "yaml":format!("sha256:{}","c".repeat(64))
    }});
    assert_eq!(
        send(json!({"format_version":1,"command":"ping"}))["value"]["generation"],
        0
    );
    let reset = json!({"format_version":1,"id":"reset","command":"reset","base_generation":0,
        "profile":profile,"sources":[],"attachments":[]});
    let response = send(reset.clone());
    assert_eq!(response["id"], "reset");
    assert_eq!(response["value"]["generation"], 1);
    assert_eq!(send(reset)["ok"], false);
    assert_eq!(
        send(json!({"format_version":99,"command":"snapshot"}))["ok"],
        false
    );
    let snapshot = send(json!({"format_version":1,"command":"snapshot"}));
    assert_eq!(snapshot["value"]["generation"], 1);
    assert_eq!(snapshot["value"]["changed"], json!({}));
    drop(send);
    drop(input);
    assert!(child.wait().unwrap().success());
}
