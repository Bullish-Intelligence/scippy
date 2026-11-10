//! The front matter of a note: the envelope, the YAML facts, and the owned `loci` region.
//!
//! The envelope is a port of `source/envelope.py`. The YAML reading follows PyYAML: the same
//! implicit types for plain scalars, and node spans in bytes of the original file.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use saphyr_parser::{Event, Parser, ScalarStyle};

/// The envelope of a note: where the front matter and the body lie.
#[derive(Clone, Debug)]
pub struct Envelope {
    /// The whole front matter with its delimiters, when the note has one.
    pub frontmatter: Option<(usize, usize)>,
    /// The YAML content between the delimiters.
    pub content: Option<(usize, usize)>,
    pub body: (usize, usize),
}

fn is_close_line(line: &[u8]) -> bool {
    let mut end = line.len();
    while end > 0 && line[end - 1] == b'\r' {
        end -= 1;
    }
    while end > 0 && matches!(line[end - 1], b' ' | b'\t') {
        end -= 1;
    }
    let stripped = &line[..end];
    (stripped == b"---" || stripped == b"...") && !line.starts_with(b"--- ")
}

pub fn scan_envelope(raw: &[u8]) -> Envelope {
    let n = raw.len();
    let start = if raw.starts_with(b"\xef\xbb\xbf") {
        3
    } else {
        0
    };
    let mut opening_end = None;
    if n - start >= 3 && &raw[start..start + 3] == b"---" {
        let rest = &raw[start + 3..];
        if rest.starts_with(b"\r\n") {
            opening_end = Some(start + 5);
        } else if rest.starts_with(b"\n") {
            opening_end = Some(start + 4);
        }
    }
    let Some(open_end) = opening_end else {
        return Envelope {
            frontmatter: None,
            content: None,
            body: (0, n),
        };
    };
    let mut pos = open_end;
    while pos < n {
        let nxt = raw[pos..].iter().position(|&b| b == b'\n').map(|p| p + pos);
        let line_end = nxt.unwrap_or(n);
        if is_close_line(&raw[pos..line_end]) {
            return Envelope {
                frontmatter: Some((start, line_end + 1)),
                content: Some((open_end, pos)),
                body: (line_end + 1, n),
            };
        }
        match nxt {
            None => break,
            Some(p) => pos = p + 1,
        }
    }
    Envelope {
        frontmatter: Some((start, n)),
        content: Some((open_end, n)),
        body: (n, n),
    }
}

/// A YAML node with its span in bytes of the original file.
#[derive(Clone, Debug)]
pub enum Node {
    Scalar {
        value: String,
        style: ScalarStyle,
        /// The explicit tag, as `handle` and `suffix`.
        tag: Option<(String, String)>,
        span: (usize, usize),
    },
    Seq {
        items: Vec<Node>,
        flow: bool,
        span: (usize, usize),
    },
    Map {
        pairs: Vec<(Node, Node)>,
        span: (usize, usize),
    },
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

/// The kind of a scalar after the implicit resolution of PyYAML.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolved {
    Str,
    Null,
    Bool(bool),
    Int,
    Float,
    Other,
}

fn resolve_plain(value: &str) -> Resolved {
    static BOOL: OnceLock<Regex> = OnceLock::new();
    static FLOAT: OnceLock<Regex> = OnceLock::new();
    static INT: OnceLock<Regex> = OnceLock::new();
    static NULL: OnceLock<Regex> = OnceLock::new();
    static TIME: OnceLock<Regex> = OnceLock::new();
    if regex(&NULL, r"^(?:~|null|Null|NULL|)$").is_match(value) {
        return Resolved::Null;
    }
    if regex(
        &BOOL,
        r"^(?:yes|Yes|YES|no|No|NO|true|True|TRUE|false|False|FALSE|on|On|ON|off|Off|OFF)$",
    )
    .is_match(value)
    {
        let truth = matches!(
            value,
            "yes" | "Yes" | "YES" | "true" | "True" | "TRUE" | "on" | "On" | "ON"
        );
        return Resolved::Bool(truth);
    }
    if regex(
        &FLOAT,
        r"^(?:[-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+][0-9]+)?|\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?|[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$",
    )
    .is_match(value)
    {
        return Resolved::Float;
    }
    if regex(
        &INT,
        r"^(?:[-+]?0b[0-1_]+|[-+]?0[0-7_]+|[-+]?(?:0|[1-9][0-9_]*)|[-+]?0x[0-9a-fA-F_]+|[-+]?[1-9][0-9_]*(?::[0-5]?[0-9])+)$",
    )
    .is_match(value)
    {
        return Resolved::Int;
    }
    if regex(
        &TIME,
        r"^(?:[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]|[0-9][0-9][0-9][0-9]-[0-9][0-9]?-[0-9][0-9]?(?:[Tt]|[ \t]+)[0-9][0-9]?:[0-9][0-9]:[0-9][0-9](?:\.[0-9]*)?(?:[ \t]*(?:Z|[-+][0-9][0-9]?(?::[0-9][0-9])?))?)$",
    )
    .is_match(value)
        || value == "<<"
        || value == "="
    {
        return Resolved::Other;
    }
    Resolved::Str
}

impl Node {
    pub fn span(&self) -> (usize, usize) {
        match self {
            Node::Scalar { span, .. } | Node::Seq { span, .. } | Node::Map { span, .. } => *span,
        }
    }

    /// The type of a scalar node. `None` for a collection.
    pub fn resolved(&self) -> Option<Resolved> {
        let Node::Scalar {
            value, style, tag, ..
        } = self
        else {
            return None;
        };
        if let Some((handle, suffix)) = tag {
            let core = handle == "!!" || handle.starts_with("tag:yaml.org,2002:");
            return Some(match (core, suffix.as_str()) {
                (true, "str") => Resolved::Str,
                (true, "null") => Resolved::Null,
                (true, "int") => Resolved::Int,
                (true, "float") => Resolved::Float,
                _ => Resolved::Other,
            });
        }
        Some(match style {
            ScalarStyle::Plain => resolve_plain(value),
            _ => Resolved::Str,
        })
    }

    /// The string value of a scalar of the string type.
    pub fn as_str(&self) -> Option<&str> {
        match (self, self.resolved()) {
            (Node::Scalar { value, .. }, Some(Resolved::Str)) => Some(value),
            _ => None,
        }
    }
}

struct Builder {
    stack: Vec<Frame>,
    root: Option<Node>,
    anchors: HashMap<usize, Node>,
    documents: usize,
}

enum Frame {
    Seq {
        items: Vec<Node>,
        anchor: usize,
        start: usize,
    },
    Map {
        pairs: Vec<(Node, Node)>,
        key: Option<Node>,
        anchor: usize,
        start: usize,
    },
}

impl Builder {
    fn push(&mut self, node: Node, anchor: usize) {
        if anchor != 0 {
            self.anchors.insert(anchor, node.clone());
        }
        match self.stack.last_mut() {
            Some(Frame::Seq { items, .. }) => items.push(node),
            Some(Frame::Map { pairs, key, .. }) => match key.take() {
                Some(k) => pairs.push((k, node)),
                None => *key = Some(node),
            },
            None => self.root = Some(node),
        }
    }
}

/// Parse YAML text into one node. `base` is the byte offset of the text in the file.
/// Return `Err` when the text is not one valid YAML document.
fn parse_yaml(content: &str, base: usize) -> Result<Option<Node>, ()> {
    // The marker of the parser counts characters. Convert to bytes.
    let mut char_to_byte: Vec<usize> = content.char_indices().map(|(b, _)| b).collect();
    char_to_byte.push(content.len());
    let to_byte = |index: usize| base + char_to_byte[index.min(char_to_byte.len() - 1)];
    let mut parser = Parser::new_from_str(content);
    let mut builder = Builder {
        stack: Vec::new(),
        root: None,
        anchors: HashMap::new(),
        documents: 0,
    };
    while let Some(next) = parser.next_event() {
        let (event, span) = next.map_err(|_| ())?;
        let (s, e) = (to_byte(span.start.index()), to_byte(span.end.index()));
        match event {
            Event::DocumentStart(_) => {
                builder.documents += 1;
                if builder.documents > 1 {
                    return Err(());
                }
            }
            Event::Scalar(value, style, anchor, tag) => {
                let tag = tag.map(|t| (t.handle.clone(), t.suffix.clone()));
                builder.push(
                    Node::Scalar {
                        value: value.into_owned(),
                        style,
                        tag,
                        span: (s, e),
                    },
                    anchor,
                );
            }
            Event::Alias(id) => {
                let node = builder.anchors.get(&id).cloned().ok_or(())?;
                builder.push(node, 0);
            }
            Event::SequenceStart(anchor, _) => builder.stack.push(Frame::Seq {
                items: Vec::new(),
                anchor,
                start: s,
            }),
            Event::SequenceEnd => {
                let Some(Frame::Seq {
                    items,
                    anchor,
                    start,
                }) = builder.stack.pop()
                else {
                    return Err(());
                };
                let flow = start
                    .checked_sub(base)
                    .and_then(|o| content.as_bytes().get(o))
                    == Some(&b'[');
                builder.push(
                    Node::Seq {
                        items,
                        flow,
                        span: (start, e),
                    },
                    anchor,
                );
            }
            Event::MappingStart(anchor, _) => builder.stack.push(Frame::Map {
                pairs: Vec::new(),
                key: None,
                anchor,
                start: s,
            }),
            Event::MappingEnd => {
                let Some(Frame::Map {
                    pairs,
                    anchor,
                    start,
                    ..
                }) = builder.stack.pop()
                else {
                    return Err(());
                };
                builder.push(
                    Node::Map {
                        pairs,
                        span: (start, e),
                    },
                    anchor,
                );
            }
            _ => {}
        }
    }
    Ok(builder.root)
}

/// The facts of the front matter that the resolver needs.
#[derive(Clone, Debug, Default)]
pub struct FrontFacts {
    /// `(key, start, end)` of each value node that can hold a wikilink.
    pub link_texts: Vec<(String, usize, usize)>,
    /// `(tag, start, end)` of each item of the `tags` property.
    pub tag_items: Vec<(String, usize, usize)>,
    /// `(alias, start, end)` of each alias, in order.
    pub aliases: Vec<(String, usize, usize)>,
    /// The owned `loci` region is invalid or duplicate: the note is degraded.
    pub degraded: bool,
    /// A `clip_url_properties` property is an http or https address: the note is a captured web page.
    pub has_url: bool,
}

const MAX_NODE_DEPTH: usize = 64;

fn collect_link_texts(node: &Node, key: &str, out: &mut Vec<(String, usize, usize)>, depth: usize) {
    if depth > MAX_NODE_DEPTH {
        return;
    }
    match node {
        Node::Scalar { value, span, .. } => {
            if node.resolved() == Some(Resolved::Str) && value.contains("[[") {
                out.push((key.to_string(), span.0, span.1));
            }
        }
        Node::Seq { items, flow, span } => {
            if *flow && matches!(items.first(), Some(Node::Seq { .. })) {
                out.push((key.to_string(), span.0, span.1));
                return;
            }
            for child in items {
                collect_link_texts(child, key, out, depth + 1);
            }
        }
        Node::Map { pairs, .. } => {
            for (_, child) in pairs {
                collect_link_texts(child, key, out, depth + 1);
            }
        }
    }
}

fn collect_tag_items(node: &Node) -> Vec<(String, usize, usize)> {
    let item = |n: &Node| n.as_str().map(|v| (v.to_string(), n.span().0, n.span().1));
    match node {
        Node::Scalar { .. } => item(node).into_iter().collect(),
        Node::Seq { items, .. } => items
            .iter()
            .map(item)
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default(),
        Node::Map { .. } => Vec::new(),
    }
}

/// The aliases of one `aliases` value: a string, a list of strings, or an empty value.
/// `None` when the shape is not supported.
fn alias_items(node: &Node) -> Option<Vec<(String, usize, usize)>> {
    match node {
        Node::Scalar { .. } => match node.resolved() {
            Some(Resolved::Str) => Some(collect_tag_items(node)),
            Some(Resolved::Null) => Some(Vec::new()),
            _ => None,
        },
        Node::Seq { items, .. } => items
            .iter()
            .map(|n| n.as_str().map(|v| (v.to_string(), n.span().0, n.span().1)))
            .collect(),
        Node::Map { .. } => None,
    }
}

fn is_uuid(text: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    regex(&RE, r"^[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}$").is_match(text)
}

/// The numeric value of a scalar, when it has the type int, float, or bool.
fn number(node: &Node) -> Option<f64> {
    let Node::Scalar { value, .. } = node else {
        return None;
    };
    match node.resolved()? {
        Resolved::Bool(b) => Some(if b { 1.0 } else { 0.0 }),
        Resolved::Int => {
            let clean: String = value.replace('_', "");
            let (sign, digits) = match clean.strip_prefix('-') {
                Some(rest) => (-1.0, rest.to_string()),
                None => (1.0, clean.trim_start_matches('+').to_string()),
            };
            let n = if let Some(h) = digits.strip_prefix("0x") {
                i64::from_str_radix(h, 16).ok()?
            } else if let Some(b) = digits.strip_prefix("0b") {
                i64::from_str_radix(b, 2).ok()?
            } else if digits.len() > 1 && digits.starts_with('0') {
                i64::from_str_radix(&digits[1..], 8).ok()?
            } else {
                digits.parse::<i64>().ok()?
            };
            Some(sign * n as f64)
        }
        Resolved::Float => value.replace('_', "").parse::<f64>().ok(),
        _ => None,
    }
}

/// Say whether the owned `loci` region is valid, as `validate_loci` decides it.
fn loci_valid(node: &Node) -> bool {
    let Node::Map { pairs, .. } = node else {
        return false;
    };
    let mut schema = None;
    let mut id = None;
    let mut projects = None;
    let mut unknown = false;
    for (key, value) in pairs {
        match key.as_str() {
            Some("schema") => schema = Some(value),
            Some("id") => id = Some(value),
            Some("projects") => projects = Some(value),
            _ => unknown = true,
        }
    }
    if unknown {
        return false;
    }
    if schema.and_then(number) != Some(1.0) {
        return false;
    }
    if !id.and_then(|n| n.as_str()).is_some_and(is_uuid) {
        return false;
    }
    match projects {
        None => true,
        Some(node) => match node {
            Node::Scalar { .. } => node.resolved() == Some(Resolved::Null),
            Node::Seq { items, .. } => items.iter().all(|i| i.as_str().is_some_and(is_uuid)),
            Node::Map { .. } => false,
        },
    }
}

const MAX_URL: usize = 2048;

/// An http or https URL with a host, without white space or control characters.
fn safe_url(value: &str) -> bool {
    if value.is_empty()
        || value.chars().count() > MAX_URL
        || value
            .chars()
            .any(|c| crate::text::is_py_space(c) || (c as u32) < 32)
    {
        return false;
    }
    let Some((scheme, rest)) = value.split_once(':') else {
        return false;
    };
    let scheme_ok = scheme
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok || !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return false;
    }
    let Some(rest) = rest.strip_prefix("//") else {
        return false;
    };
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = netloc.rsplit('@').next().unwrap_or("");
    let host = if let Some(inner) = hostport.strip_prefix('[') {
        inner.split(']').next().unwrap_or("")
    } else {
        hostport.split(':').next().unwrap_or("")
    };
    !host.is_empty()
}

/// True when a line of the window is `loci:` and white space only.
fn has_loci_line(window: &[u8]) -> bool {
    window.split(|&b| b == b'\n').any(|line| {
        line.starts_with(b"loci:")
            && line[5..]
                .iter()
                .all(|&b| matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c))
    })
}

/// Read the front matter of a note.
pub fn analyze_front(raw: &[u8], envelope: &Envelope, clip_url: &[String]) -> FrontFacts {
    let mut facts = FrontFacts::default();
    let (Some(content_span), Some(whole)) = (envelope.content, envelope.frontmatter) else {
        return facts;
    };
    let content_bytes = &raw[content_span.0..content_span.1];
    let Ok(content) = std::str::from_utf8(content_bytes) else {
        // loci-core reads no YAML fact from content that is not UTF-8.
        return facts;
    };
    let window_has_loci = || has_loci_line(&raw[whole.0..whole.1]);
    if content_bytes
        .iter()
        .all(|&b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
    {
        return facts;
    }
    let root = match parse_yaml(content, content_span.0) {
        Ok(Some(root)) => root,
        Ok(None) => return facts,
        Err(()) => {
            facts.degraded = window_has_loci();
            return facts;
        }
    };
    let Node::Map { pairs, .. } = &root else {
        facts.degraded = window_has_loci();
        return facts;
    };
    let has_links = content.contains("[[");
    let mut loci_nodes = 0;
    let mut loci_ok = true;
    for (key_node, value) in pairs {
        let Node::Scalar { value: key, .. } = key_node else {
            continue;
        };
        if has_links {
            collect_link_texts(value, key, &mut facts.link_texts, 0);
        }
        if key == "tags" {
            facts.tag_items.extend(collect_tag_items(value));
        }
        if key == "aliases" {
            if let Some(items) = alias_items(value) {
                facts.aliases.extend(items);
            }
        }
        if clip_url.iter().any(|k| k == key) && value.as_str().is_some_and(safe_url) {
            facts.has_url = true;
        }
        if key == "loci" {
            loci_nodes += 1;
            loci_ok = loci_valid(value);
        }
    }
    if loci_nodes > 1 || (loci_nodes == 1 && !loci_ok) {
        facts.degraded = true;
    }
    facts
}
