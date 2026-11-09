//! The facts of one note: front matter, body, headings, and tags.

use crate::config::Settings;
use crate::front::{analyze_front, scan_envelope};
use crate::scan::{
    Anchor, AnchorKind, ExtractOptions, Relation, TagOccurrence, TagOrigin, analyze_body,
    scan_property_links,
};

/// Everything that the resolver and the index need to know about one note.
#[derive(Clone, Debug, Default)]
pub struct Note {
    /// The note has an invalid owned region. loci-core keeps no fact of such a note.
    pub degraded: bool,
    /// The links in source order: front matter links first, then the body.
    pub relations: Vec<Relation>,
    pub anchors: Vec<Anchor>,
    /// The tags: the items of `tags` first, then the inline tags.
    pub tags: Vec<TagOccurrence>,
    /// The aliases with their spans.
    pub aliases: Vec<(String, usize, usize)>,
    /// The note is a captured web page: it has a `url` property.
    pub page: bool,
    pub len: usize,
}

pub fn analyze_note(raw: &[u8], settings: &Settings) -> Note {
    let envelope = scan_envelope(raw);
    let front = analyze_front(raw, &envelope);
    let options = ExtractOptions {
        markdown_links: settings.markdown_links,
        inline_tags: settings.inline_tags,
    };
    let body = analyze_body(raw, envelope.body, options);
    let property_links = if settings.frontmatter_links {
        scan_property_links(raw, &front.link_texts)
    } else {
        Vec::new()
    };
    let mut note = Note {
        degraded: front.degraded,
        len: raw.len(),
        ..Note::default()
    };
    if note.degraded {
        return note;
    }
    note.relations = property_links;
    note.relations.extend(body.relations);
    note.anchors = body.anchors;
    note.tags = front
        .tag_items
        .into_iter()
        .map(|(tag, start, end)| TagOccurrence {
            tag,
            span: (start, end),
            origin: TagOrigin::Frontmatter,
            prop: Some("tags".to_string()),
        })
        .collect();
    note.tags.extend(body.tags);
    note.aliases = front.aliases;
    note.page = front.has_url;
    note
}

impl Note {
    pub fn headings(&self) -> impl Iterator<Item = &Anchor> {
        self.anchors
            .iter()
            .filter(|a| a.kind == AnchorKind::Heading)
    }
}
