//! The scanner of a note body. It is a port of the walk of loci-core (`source/body.py`).
//!
//! It reads wikilinks, embeds, Markdown links and images, inline tags, headings, and block ids.
//! Every span is a half-open range of UTF-8 bytes in the original file.
//! The port keeps the exact rules of the original, including its limits and its edge cases.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::text::{category_in, is_py_space, py_strip, unquote};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Wikilink,
    Embed,
    MarkdownLink,
    MarkdownEmbed,
    FrontmatterLink,
    Tag,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Wikilink => "wikilink",
            Kind::Embed => "embed",
            Kind::MarkdownLink => "markdown_link",
            Kind::MarkdownEmbed => "markdown_embed",
            Kind::FrontmatterLink => "frontmatter_link",
            Kind::Tag => "tag",
        }
    }

    pub fn is_markdown(self) -> bool {
        matches!(self, Kind::MarkdownLink | Kind::MarkdownEmbed)
    }
}

/// A link that the walk found.
#[derive(Clone, Debug)]
pub struct Relation {
    pub kind: Kind,
    /// The target. For a Markdown link it is the path with the percent escapes decoded.
    pub target: String,
    pub label: Option<String>,
    pub heading: Option<String>,
    /// The block id with its caret.
    pub block: Option<String>,
    pub span: (usize, usize),
    pub prop: Option<String>,
    pub embed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagOrigin {
    Body,
    Frontmatter,
}

#[derive(Clone, Debug)]
pub struct TagOccurrence {
    pub tag: String,
    pub span: (usize, usize),
    pub origin: TagOrigin,
    pub prop: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorKind {
    Heading,
    Block,
    /// A footnote definition `[^id]:`.
    Footnote,
    /// An HTML `id` attribute, or the `name` attribute of an `<a>` tag.
    Html,
}

#[derive(Clone, Debug)]
pub struct Anchor {
    pub kind: AnchorKind,
    /// The heading text, the block id without its caret, or the id of a footnote or an element.
    pub text: String,
    pub level: u8,
    pub span: (usize, usize),
}

#[derive(Clone, Copy, Debug)]
pub struct ExtractOptions {
    pub markdown_links: bool,
    pub inline_tags: bool,
}

#[derive(Debug, Default)]
pub struct BodyFacts {
    pub relations: Vec<Relation>,
    pub anchors: Vec<Anchor>,
    pub tags: Vec<TagOccurrence>,
}

const MAX_LINK_TEXT: usize = 4096;
const MAX_LINK_DEST: usize = 4096;
const DEST_BUDGET_PER_BYTE: i64 = 8;
const DEST_BUDGET_FLOOR: i64 = 1 << 16;
const MAX_NEST: u32 = 4;

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Python `bytes.strip()` removes these bytes.
fn is_ascii_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn strip_bytes(raw: &[u8]) -> &[u8] {
    let start = raw
        .iter()
        .position(|&b| !is_ascii_space(b))
        .unwrap_or(raw.len());
    let end = raw
        .iter()
        .rposition(|&b| !is_ascii_space(b))
        .map_or(start, |e| e + 1);
    &raw[start..end]
}

/// Slice with the clamping of Python.
fn slice(raw: &[u8], a: usize, b: usize) -> &[u8] {
    let b = b.min(raw.len());
    let a = a.min(b);
    &raw[a..b]
}

fn find_byte(raw: &[u8], needle: u8, start: usize, end: usize) -> Option<usize> {
    let end = end.min(raw.len());
    if start >= end {
        return None;
    }
    raw[start..end]
        .iter()
        .position(|&b| b == needle)
        .map(|p| p + start)
}

fn find_bytes(raw: &[u8], needle: &[u8], start: usize, end: usize) -> Option<usize> {
    let end = end.min(raw.len());
    if start >= end || end - start < needle.len() {
        return None;
    }
    raw[start..end]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + start)
}

fn rfind_byte_before(raw: &[u8], needle: u8, end: usize) -> Option<usize> {
    raw[..end.min(raw.len())].iter().rposition(|&b| b == needle)
}

/// If `raw[i..i+3]` opens a fenced code block, return the index past its close.
fn skip_fence(raw: &[u8], i: usize) -> usize {
    let head = slice(raw, i, i + 3);
    if head != b"```" && head != b"~~~" {
        return i;
    }
    let line_start = rfind_byte_before(raw, b'\n', i).map_or(0, |p| p + 1);
    if !strip_bytes(&raw[line_start..i]).is_empty() {
        return i;
    }
    let fence = head.to_vec();
    let Some(j) = find_byte(raw, b'\n', i, raw.len()) else {
        return raw.len();
    };
    let mut pos = j + 1;
    loop {
        let nxt = find_byte(raw, b'\n', pos, raw.len());
        let line_end = nxt.unwrap_or(raw.len());
        let line = strip_bytes(slice(raw, pos, line_end));
        if line.starts_with(&fence) {
            return line_end + 1;
        }
        match nxt {
            None => return raw.len(),
            Some(n) => pos = n + 1,
        }
    }
}

/// The next run of exactly `run` backticks in `raw[start..end]`.
fn find_backtick_run(raw: &[u8], start: usize, end: usize, run: usize) -> Option<usize> {
    let end = end.min(raw.len());
    let mut i = start;
    while i < end {
        let j = find_byte(raw, b'`', i, end)?;
        let mut k = j;
        while k < end && raw[k] == b'`' {
            k += 1;
        }
        if k - j == run {
            return Some(j);
        }
        i = k;
    }
    None
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// `(caret index, word start)` when `raw[line_start..line_end]` ends with ` ^word`.
fn trailing_block_id(raw: &[u8], line_start: usize, line_end: usize) -> Option<(usize, usize)> {
    let mut k = line_end;
    while k > line_start && is_word_byte(raw[k - 1]) {
        k -= 1;
    }
    if k == line_end || k < line_start + 2 {
        return None;
    }
    if raw[k - 1] != b'^' || raw[k - 2] != b' ' {
        return None;
    }
    Some((k - 1, k))
}

/// Headings and block ids, line by line. The fences that the walk found are skipped.
fn extract_anchors(raw: &[u8], start: usize, end: usize, fences: &[(usize, usize)]) -> Vec<Anchor> {
    let mut anchors = Vec::new();
    let mut fence_idx = 0;
    let mut pos = start;
    let n = end;
    if pos == 0 && raw.starts_with(b"\xef\xbb\xbf") {
        pos = 3;
    }
    while pos < n {
        while fence_idx < fences.len() && fences[fence_idx].1 <= pos {
            fence_idx += 1;
        }
        if fence_idx < fences.len() && fences[fence_idx].0 <= pos && pos < fences[fence_idx].1 {
            pos = fences[fence_idx].1;
            continue;
        }
        let line_end = find_byte(raw, b'\n', pos, n).unwrap_or(n);
        let content_end = if line_end > pos && raw[line_end - 1] == 0x0d {
            line_end - 1
        } else {
            line_end
        };
        if let Some((caret, word_start)) = trailing_block_id(raw, pos, content_end) {
            anchors.push(Anchor {
                kind: AnchorKind::Block,
                text: lossy(&raw[word_start..content_end]),
                level: 0,
                span: (caret, content_end),
            });
        }
        if raw.get(pos) == Some(&b'#') {
            let mut level = 0u8;
            let mut j = pos;
            while j < n && raw[j] == b'#' && level < 6 {
                level += 1;
                j += 1;
            }
            if level > 0 && j < line_end && matches!(raw[j], b' ' | b'\t') {
                let mut text_start = j;
                while text_start < line_end && matches!(raw[text_start], b' ' | b'\t') {
                    text_start += 1;
                }
                let mut text_end = line_end;
                while text_end > text_start && is_ascii_space(raw[text_end - 1]) {
                    text_end -= 1;
                }
                let text = lossy(&raw[text_start..text_end]);
                if !text.is_empty() {
                    anchors.push(Anchor {
                        kind: AnchorKind::Heading,
                        text,
                        level,
                        span: (pos, line_end),
                    });
                }
            }
        }
        pos = line_end + 1;
    }
    anchors
}

fn in_fence(fences: &[(usize, usize)], at: usize) -> bool {
    // The fences are sorted by start. Find the last one that starts at or before `at`.
    let i = fences.partition_point(|f| f.0 <= at);
    i > 0 && fences[i - 1].0 <= at && at < fences[i - 1].1
}

/// Footnote definition ids and HTML ids of the body, outside fenced code.
/// Inline code is not skipped: an `id="x"` in a code span counts as an id.
fn extract_ids(raw: &[u8], start: usize, end: usize, fences: &[(usize, usize)]) -> Vec<Anchor> {
    static FOOTNOTE: OnceLock<regex::bytes::Regex> = OnceLock::new();
    static TAG: OnceLock<regex::bytes::Regex> = OnceLock::new();
    static ATTR: OnceLock<regex::bytes::Regex> = OnceLock::new();
    let footnote = FOOTNOTE.get_or_init(|| {
        regex::bytes::Regex::new(r"(?m-u)^ {0,3}\[\^([^\]\x20\t\n\r\x0b\x0c]+)\]:").expect("regex")
    });
    let tag = TAG.get_or_init(|| {
        regex::bytes::Regex::new(r"(?-u)<([A-Za-z][A-Za-z0-9]*)([ \t\n\r\x0b\x0c][^<>]{0,2000})>")
            .expect("regex")
    });
    let attr = ATTR.get_or_init(|| {
        regex::bytes::Regex::new(
            r#"(?-u)(?i)(id|name)[ \t\n\r\x0b\x0c]*=[ \t\n\r\x0b\x0c]*(?:"([^"]*)"|'([^']*)'|([^ \t\n\r\x0b\x0c"'=<>`]+))"#,
        )
        .expect("regex")
    });
    let end = end.min(raw.len());
    let start = if start == 0 && raw.starts_with(b"\xef\xbb\xbf") {
        3
    } else {
        start
    };
    let haystack = &raw[..end];
    let mut found = Vec::new();
    let mut pos = start;
    while pos <= end {
        let Some(m) = footnote.captures_at(haystack, pos) else {
            break;
        };
        let whole = m.get(0).expect("match");
        if !in_fence(fences, whole.start()) {
            found.push(Anchor {
                kind: AnchorKind::Footnote,
                text: lossy(&m[1]),
                level: 0,
                span: (whole.start(), whole.end()),
            });
        }
        pos = whole.end().max(whole.start() + 1);
    }
    let mut pos = start;
    while pos <= end {
        let Some(m) = tag.captures_at(haystack, pos) else {
            break;
        };
        let whole = m.get(0).expect("match");
        pos = whole.end().max(whole.start() + 1);
        if in_fence(fences, whole.start()) {
            continue;
        }
        let is_link = m[1].eq_ignore_ascii_case(b"a");
        let attrs = &m[2];
        let mut at = 0;
        while at <= attrs.len() {
            let Some(a) = attr.captures_at(attrs, at) else {
                break;
            };
            let whole_attr = a.get(0).expect("match");
            // Python reads `(?<![\w:.-])`: the byte before the name is no word byte.
            let before = whole_attr.start().checked_sub(1).map(|i| attrs[i]);
            if before.is_some_and(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.' | b'-')
            }) {
                at = whole_attr.start() + 1;
                continue;
            }
            at = whole_attr.end().max(whole_attr.start() + 1);
            if a[1].eq_ignore_ascii_case(b"name") && !is_link {
                continue;
            }
            let value = a
                .get(2)
                .or(a.get(3))
                .or(a.get(4))
                .map_or_else(String::new, |g| lossy(g.as_bytes()));
            let value = py_strip(&value);
            if !value.is_empty() {
                found.push(Anchor {
                    kind: AnchorKind::Html,
                    text: value.to_string(),
                    level: 0,
                    span: (whole.start(), whole.end()),
                });
            }
        }
    }
    found
}

/// The brackets of one range: the index of each `[` mapped to the index of its `]`.
fn pair_brackets(raw: &[u8], start: usize, end: usize) -> HashMap<usize, usize> {
    let end = end.min(raw.len());
    let mut pairs = HashMap::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut skip = usize::MAX;
    let mut j = start;
    while j < end {
        let c = raw[j];
        if !matches!(c, b'[' | b']' | b'\\' | b'\n') {
            j += 1;
            continue;
        }
        if j == skip {
            j += 1;
            continue;
        }
        match c {
            b'\\' => skip = j + 1,
            b'\n' => {
                let mut k = j + 1;
                while k < end && matches!(raw[k], b' ' | b'\t' | b'\r') {
                    k += 1;
                }
                if k >= end || raw[k] == b'\n' {
                    stack.clear();
                }
            }
            b'[' => stack.push(j),
            _ => {
                if let Some(open) = stack.pop() {
                    pairs.insert(open, j);
                }
            }
        }
        j += 1;
    }
    pairs
}

/// The destination and the optional title of an inline link, from just after the `(`.
/// Return the destination and the index past the closing parenthesis.
fn parse_md_dest(raw: &[u8], mut p: usize, n: usize) -> Option<(Vec<u8>, usize)> {
    let limit = n.min(p + MAX_LINK_DEST);
    let at = |i: usize| raw.get(i).copied();
    while p < limit && matches!(raw[p], b' ' | b'\t') {
        p += 1;
    }
    if p >= limit {
        return None;
    }
    let dest;
    if raw[p] == b'<' {
        let mut q = p + 1;
        while q < limit && raw[q] != b'>' {
            if matches!(raw[q], b'\n' | b'<') {
                return None;
            }
            q += if raw[q] == b'\\' { 2 } else { 1 };
        }
        if q >= limit {
            return None;
        }
        dest = slice(raw, p + 1, q).to_vec();
        p = q + 1;
    } else {
        let mut depth = 0i32;
        let mut q = p;
        while q < limit {
            let ch = raw[q];
            if ch == b'\\' {
                q += 2;
                continue;
            }
            if matches!(ch, b' ' | b'\t' | b'\r' | b'\n') {
                break;
            }
            if ch == b'(' {
                depth += 1;
            } else if ch == b')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            q += 1;
        }
        if depth != 0 {
            return None;
        }
        dest = slice(raw, p, q).to_vec();
        p = q;
    }
    while p < limit && matches!(raw[p], b' ' | b'\t' | b'\r' | b'\n') {
        p += 1;
    }
    if p < limit && matches!(raw[p], b'"' | b'\'' | b'(') {
        let close = if raw[p] == b'(' { b')' } else { raw[p] };
        let mut q = p + 1;
        while q < limit && raw[q] != close {
            q += if raw[q] == b'\\' { 2 } else { 1 };
        }
        if q >= limit {
            return None;
        }
        p = q + 1;
        while p < limit && matches!(raw[p], b' ' | b'\t' | b'\r' | b'\n') {
            p += 1;
        }
    }
    if p < limit && at(p) == Some(b')') {
        return Some((dest, p + 1));
    }
    None
}

/// A destination that leaves the vault: a scheme (`https:`, `mailto:`) or a `//` host.
fn is_external(dest: &[u8]) -> bool {
    if dest.starts_with(b"//") {
        return true;
    }
    let Some(first) = dest.first() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    for (i, &b) in dest.iter().enumerate().skip(1) {
        if b == b':' {
            return i > 0;
        }
        if !(b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-')) {
            return false;
        }
    }
    false
}

/// Remove the backslash before an ASCII punctuation mark.
fn unescape_punct(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if next.is_ascii_punctuation() {
                    out.push(next);
                    chars.next();
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// `(path, heading, block)` of a Markdown destination. `None` for an external or empty one.
fn md_target(dest: &[u8]) -> Option<(String, Option<String>, Option<String>)> {
    if dest.is_empty() || is_external(dest) {
        return None;
    }
    let text = unescape_punct(&lossy(dest));
    let (path, frag) = match text.split_once('#') {
        Some((p, f)) => (p.to_string(), f.to_string()),
        None => (text, String::new()),
    };
    let path = py_strip(&unquote(&path)).to_string();
    let frag = py_strip(&unquote(&frag)).to_string();
    if path.is_empty() && frag.is_empty() {
        return None;
    }
    if frag.starts_with('^') {
        return Some((path, None, Some(frag)));
    }
    let heading = if frag.is_empty() { None } else { Some(frag) };
    Some((path, heading, None))
}

/// `(target, label, heading, block)` of the inner text of a wikilink.
fn split_wikilink(text: &str) -> (String, Option<String>, Option<String>, Option<String>) {
    let (mut target_part, label) = match text.split_once('|') {
        Some((t, l)) => (t.to_string(), Some(l.to_string())),
        None => (text.to_string(), None),
    };
    if label.is_some() {
        let trailing = target_part.len() - target_part.trim_end_matches('\\').len();
        if trailing % 2 == 1 {
            target_part.pop();
        }
    }
    let (target, suffix) = match target_part.split_once('#') {
        Some((t, s)) => (t.to_string(), s.to_string()),
        None => (target_part.clone(), String::new()),
    };
    let suffix = py_strip(&suffix).to_string();
    let (mut heading, mut block) = (None, None);
    if !suffix.is_empty() {
        if suffix.starts_with('^') {
            block = Some(suffix);
        } else {
            heading = Some(suffix);
        }
    }
    let label = label.filter(|l| !l.is_empty());
    (py_strip(&target).to_string(), label, heading, block)
}

/// The inline tag that starts at the `#` at `j`: the tag and the index past it.
fn tag_at(raw: &[u8], j: usize, n: usize, body_start: usize) -> Option<(String, usize)> {
    const PRECEDE: &[u8] = b" \t\r\n([{>\"',;";
    if j > body_start && !PRECEDE.contains(&raw[j - 1]) {
        return None;
    }
    let mut e = j + 1;
    while e < n.min(raw.len()) {
        let b = raw[e];
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'/' | b'-') || b >= 0x80 {
            e += 1;
        } else {
            break;
        }
    }
    if e == j + 1 {
        return None;
    }
    let text = lossy(&raw[j + 1..e]);
    let mut kept = String::new();
    for ch in text.chars() {
        if (ch as u32) >= 0x80 && (is_py_space(ch) || category_in(ch, "PZC")) {
            break;
        }
        kept.push(ch);
    }
    let text = kept.trim_end_matches('/').to_string();
    if text.is_empty()
        || text.starts_with('/')
        || text
            .chars()
            .all(|c| c.is_ascii_digit() || c == '/' || c == '-')
    {
        return None;
    }
    let line_start = rfind_byte_before(raw, b'\n', j).map_or(0, |p| p + 1);
    let mut k = line_start;
    while k < j && raw[k] == b'#' {
        k += 1;
    }
    if (1..=6).contains(&(k - line_start)) && k < n && matches!(raw[k], b' ' | b'\t') {
        return None;
    }
    let end = j + 1 + text.len();
    Some((text, end))
}

struct Walk<'a> {
    raw: &'a [u8],
    options: ExtractOptions,
    body_start: usize,
    relations: Vec<Relation>,
    tags: Vec<TagOccurrence>,
    fence_spans: Vec<(usize, usize)>,
    wiki_dead: bool,
    probe: i64,
    link_end: usize,
    top: usize,
    pairs: Option<HashMap<usize, usize>>,
    budget: i64,
}

impl<'a> Walk<'a> {
    fn new(raw: &'a [u8], options: ExtractOptions, body_start: usize) -> Self {
        Self {
            raw,
            options,
            body_start,
            relations: Vec::new(),
            tags: Vec::new(),
            fence_spans: Vec::new(),
            wiki_dead: false,
            probe: -2,
            link_end: 0,
            top: body_start,
            pairs: None,
            budget: DEST_BUDGET_FLOOR,
        }
    }

    fn run(&mut self, mut i: usize, n: usize, depth: u32) {
        let raw = self.raw;
        if depth == 0 {
            self.top = n;
            self.budget = DEST_BUDGET_FLOOR + DEST_BUDGET_PER_BYTE * (n as i64 - i as i64);
        }
        let markdown = self.options.markdown_links;
        let tags = self.options.inline_tags && depth == 0;
        let n = n.min(raw.len());
        while i < n {
            let Some(j) =
                (i..n).find(|&k| matches!(raw[k], b'`' | b'~' | b'[') || (tags && raw[k] == b'#'))
            else {
                return;
            };
            let c = raw[j];
            if c == b'`' || c == b'~' {
                if depth == 0 {
                    let after = skip_fence(raw, j);
                    if after != j {
                        self.fence_spans.push((j, after));
                        i = after;
                        continue;
                    }
                }
                if c == b'~' {
                    i = j + 1;
                    continue;
                }
                let mut run = 1;
                while j + run < n && raw[j + run] == b'`' {
                    run += 1;
                }
                i = match find_backtick_run(raw, j + run, n, run) {
                    Some(close) => close + run,
                    None => j + run,
                };
                continue;
            }
            if c == b'#' {
                match tag_at(raw, j, n, self.body_start) {
                    None => i = j + 1,
                    Some((tag, end)) => {
                        self.tags.push(TagOccurrence {
                            tag,
                            span: (j, end),
                            origin: TagOrigin::Body,
                            prop: None,
                        });
                        i = end;
                    }
                }
                continue;
            }
            // `[`. `![` is an embed or an image when the `!` is part of this walk.
            let bang = j > i && raw[j - 1] == b'!';
            if raw.get(j + 1) == Some(&b'[') && !self.wiki_dead {
                i = self.wikilink(j, n, bang, depth);
                continue;
            }
            if markdown && self.markdown_link(j, n, bang, depth) {
                i = self.link_end;
                continue;
            }
            i = j + 1;
        }
    }

    fn wikilink(&mut self, j: usize, n: usize, bang: bool, depth: u32) -> usize {
        let raw = self.raw;
        let start = if bang { j - 1 } else { j };
        let kind = if bang { Kind::Embed } else { Kind::Wikilink };
        let content_start = j + 2;
        if find_bytes(raw, b"]]", content_start, n).is_none() {
            if depth == 0 {
                self.wiki_dead = true;
            }
            return j + 1;
        }
        let mut k = content_start;
        let mut closed = false;
        while k + 1 < n {
            if &raw[k..k + 2] == b"\\]" {
                k += 2;
                continue;
            }
            if &raw[k..k + 2] == b"]]" {
                closed = true;
                break;
            }
            k += 1;
        }
        if !closed {
            return content_start;
        }
        let inner = &raw[content_start..k];
        let text = replace_bytes(&replace_bytes(inner, b"\\]", b"]"), b"\\[", b"[");
        let text = lossy(&text);
        let (target, label, heading, block) = split_wikilink(&text);
        self.relations.push(Relation {
            kind,
            target,
            label,
            heading,
            block,
            span: (start, k + 2),
            prop: None,
            embed: false,
        });
        k + 2
    }

    fn markdown_link(&mut self, j: usize, n: usize, bang: bool, depth: u32) -> bool {
        let raw = self.raw;
        if self.probe != -1 && self.probe < j as i64 {
            self.probe = find_bytes(raw, b"](", j, raw.len()).map_or(-1, |p| p as i64);
        }
        if self.probe == -1 {
            return false;
        }
        if j > 0 && raw[j - 1] == b'\\' && !bang {
            return false;
        }
        if self.pairs.is_none() {
            self.pairs = Some(pair_brackets(raw, self.body_start, self.top));
        }
        let close = self.pairs.as_ref().and_then(|p| p.get(&j)).copied();
        let Some(close) = close else {
            return false;
        };
        if close >= n || close - j > MAX_LINK_TEXT || raw.get(close + 1) != Some(&b'(') {
            return false;
        }
        if self.budget <= 0 {
            self.probe = -1;
            return false;
        }
        let parsed = parse_md_dest(raw, close + 2, n);
        let used = match &parsed {
            Some((_, end)) => *end,
            None => MAX_LINK_DEST.min(n.saturating_sub(close)),
        };
        self.budget -= used as i64 - close as i64;
        let Some((dest, end)) = parsed else {
            return false;
        };
        if let Some((path, heading, block)) = md_target(&dest) {
            let label = lossy(&raw[j + 1..close]);
            self.relations.push(Relation {
                kind: if bang {
                    Kind::MarkdownEmbed
                } else {
                    Kind::MarkdownLink
                },
                target: path,
                label: if label.is_empty() { None } else { Some(label) },
                heading,
                block,
                span: (if bang { j - 1 } else { j }, end),
                prop: None,
                embed: false,
            });
        }
        if depth < MAX_NEST && close > j + 1 {
            self.run(j + 1, close, depth + 1);
        }
        self.link_end = end;
        true
    }
}

fn replace_bytes(raw: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    out
}

/// Scan the body range for links, tags, headings, and block ids.
pub fn analyze_body(raw: &[u8], body: (usize, usize), options: ExtractOptions) -> BodyFacts {
    let mut walk = Walk::new(raw, options, body.0);
    walk.run(body.0, body.1, 0);
    let mut anchors = extract_anchors(raw, body.0, body.1, &walk.fence_spans);
    anchors.extend(extract_ids(raw, body.0, body.1, &walk.fence_spans));
    BodyFacts {
        relations: walk.relations,
        anchors,
        tags: walk.tags,
    }
}

/// The wikilinks and embeds inside front matter values. `texts` holds `(key, start, end)`.
pub fn scan_property_links(raw: &[u8], texts: &[(String, usize, usize)]) -> Vec<Relation> {
    let options = ExtractOptions {
        markdown_links: false,
        inline_tags: false,
    };
    let mut found: HashMap<(usize, usize), Relation> = HashMap::new();
    for (key, start, end) in texts {
        let mut walk = Walk::new(raw, options, *start);
        walk.run(*start, *end, 0);
        for mut r in walk.relations {
            let span = r.span;
            r.embed = r.kind == Kind::Embed;
            r.kind = Kind::FrontmatterLink;
            r.prop = Some(key.clone());
            found.entry(span).or_insert(r);
        }
    }
    let mut out: Vec<Relation> = found.into_values().collect();
    out.sort_by_key(|r| r.span);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(text: &str) -> BodyFacts {
        analyze_body(
            text.as_bytes(),
            (0, text.len()),
            ExtractOptions {
                markdown_links: true,
                inline_tags: true,
            },
        )
    }

    #[test]
    fn reads_a_wikilink_with_all_parts() {
        let f = scan("see [[Note#Head|label]] now");
        let r = &f.relations[0];
        assert_eq!(r.kind, Kind::Wikilink);
        assert_eq!(r.target, "Note");
        assert_eq!(r.heading.as_deref(), Some("Head"));
        assert_eq!(r.label.as_deref(), Some("label"));
        assert_eq!(r.span, (4, 23));
    }

    #[test]
    fn escaped_pipe_in_a_table() {
        let f = scan("| [[a\\|b]] |");
        let r = &f.relations[0];
        assert_eq!(r.target, "a");
        assert_eq!(r.label.as_deref(), Some("b"));
    }

    #[test]
    fn markdown_link_and_image() {
        let f = scan("[t](<a b.md#H%20x>) ![i](p.png) [e](https://x)");
        assert_eq!(f.relations.len(), 2);
        assert_eq!(f.relations[0].target, "a b.md");
        assert_eq!(f.relations[0].heading.as_deref(), Some("H x"));
        assert_eq!(f.relations[1].kind, Kind::MarkdownEmbed);
    }

    #[test]
    fn code_is_skipped() {
        let f = scan("`[[a]]` and\n```\n[[b]]\n```\n[[c]]");
        assert_eq!(f.relations.len(), 1);
        assert_eq!(f.relations[0].target, "c");
    }

    #[test]
    fn tags_need_a_non_digit_and_not_in_headings() {
        let f = scan("x #2024 #a/b #1-2\n# #head\nurl#frag #ok");
        let tags: Vec<_> = f.tags.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(tags, ["a/b", "ok"]);
    }

    #[test]
    fn ids_of_footnotes_and_html() {
        let f = scan(
            "Text[^a]\n\n[^a]: Note\n<a name=\"nm\" id='i1'>x</a> <div data-id=\"no\" id=x>\n```\n[^b]: no\n```\n",
        );
        let ids: Vec<_> = f
            .anchors
            .iter()
            .filter(|a| matches!(a.kind, AnchorKind::Footnote | AnchorKind::Html))
            .map(|a| a.text.as_str())
            .collect();
        assert_eq!(ids, ["a", "nm", "i1", "x"]);
    }

    #[test]
    fn anchors() {
        let f = scan("# One\ntext ^blk\n## Two ##\n");
        assert_eq!(f.anchors.len(), 3);
        assert_eq!(f.anchors[1].text, "blk");
        assert_eq!(f.anchors[0].span, (0, 5));
    }
}
