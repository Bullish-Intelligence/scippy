//! The facts that the identity store needs: one record for each note, heading, and block id.
//!
//! The records do not change a symbol. The core joins them to the declarations by span.

use std::collections::HashMap;

use scippy_core::{OutlineKind, OutlineRecord};
use sha2::{Digest, Sha256};

use crate::note::Note;
use crate::scan::AnchorKind;

/// The record of the note itself.
pub fn note_record(note: &Note) -> OutlineRecord {
    OutlineRecord {
        kind: OutlineKind::Document,
        external_id: note.loci_id.clone(),
        ..OutlineRecord::default()
    }
}

/// The digest of a section: the text with white space normalized, and its word count.
/// The digest keeps 128 bits of SHA-256, as 32 lowercase hex digits.
fn section_digest(text: &[u8]) -> (String, u32) {
    let text = String::from_utf8_lossy(text);
    let mut hash = Sha256::new();
    let mut words = 0u32;
    for word in text.split_whitespace() {
        if words > 0 {
            hash.update(b" ");
        }
        hash.update(word.as_bytes());
        words = words.saturating_add(1);
    }
    let hex = hash
        .finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect();
    (hex, words)
}

/// The records of the headings and the block ids of a note, by the span of the anchor.
///
/// A heading has its level, its ancestors, its place among equal siblings, and the digest of
/// its section. The section starts after the heading line. It ends at the next heading of the
/// same or a higher level, or at the end of the note.
pub fn anchor_records(note: &Note, text: &str) -> HashMap<(usize, usize), OutlineRecord> {
    let bytes = text.as_bytes();
    let mut out = HashMap::new();
    let headings: Vec<_> = note.headings().collect();
    let mut stack: Vec<(u8, &str)> = Vec::new();
    let mut seen: HashMap<(Vec<String>, &str), u32> = HashMap::new();
    for (i, heading) in headings.iter().enumerate() {
        while stack
            .last()
            .is_some_and(|(level, _)| *level >= heading.level)
        {
            stack.pop();
        }
        let ancestors: Vec<String> = stack.iter().map(|(_, t)| (*t).to_string()).collect();
        stack.push((heading.level, heading.text.as_str()));
        let count = seen
            .entry((ancestors.clone(), heading.text.as_str()))
            .or_insert(0);
        let ordinal = *count;
        *count += 1;
        let start = heading.span.1.min(bytes.len());
        let end = headings[i + 1..]
            .iter()
            .find(|next| next.level <= heading.level)
            .map_or(bytes.len(), |next| next.span.0)
            .max(start);
        let (digest, tokens) = section_digest(&bytes[start..end]);
        out.insert(
            heading.span,
            OutlineRecord {
                kind: OutlineKind::Section,
                level: Some(heading.level),
                ancestors,
                text: Some(heading.text.clone()),
                ordinal: Some(ordinal),
                section_digest: Some(digest),
                section_tokens: Some(tokens),
                ..OutlineRecord::default()
            },
        );
    }
    for anchor in note.anchors.iter().filter(|a| a.kind == AnchorKind::Block) {
        out.insert(
            anchor.span,
            OutlineRecord {
                kind: OutlineKind::Block,
                block_id: Some(anchor.text.clone()),
                ..OutlineRecord::default()
            },
        );
    }
    out
}
