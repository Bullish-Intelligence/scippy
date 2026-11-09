//! Convert byte spans to SCIP ranges.
//!
//! All positions are 0-based. A range is half-open: `[start, end)`.
//! A column is a count of UTF-8 bytes from the start of the line.
//! Only `\n` ends a line, as in Tree-sitter. A `\r` before it stays in the line.

/// A range with line numbers and UTF-8 byte columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Range {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

/// Line start offsets of one source text.
pub(crate) struct LineIndex<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        Self { text, line_starts }
    }

    /// Convert a byte span to a range.
    ///
    /// The span must be non-empty, inside the text, and start and end on
    /// character boundaries.
    pub fn range(&self, start_byte: u32, end_byte: u32) -> Result<Range, &'static str> {
        let (start, end) = (start_byte as usize, end_byte as usize);
        if start >= end {
            return Err("span is empty or reversed");
        }
        if end > self.text.len() {
            return Err("span ends after the source text");
        }
        if !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
            return Err("span does not start and end on UTF-8 character boundaries");
        }
        let (start_line, start_character) = self.position(start);
        let (end_line, end_character) = self.position(end);
        Ok(Range {
            start_line,
            start_character,
            end_line,
            end_character,
        })
    }

    fn position(&self, byte: usize) -> (u32, u32) {
        let line = self.line_starts.partition_point(|&start| start <= byte) - 1;
        (line as u32, (byte - self.line_starts[line]) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(text: &str, start: u32, end: u32) -> Range {
        LineIndex::new(text).range(start, end).unwrap()
    }

    #[test]
    fn ascii_single_line() {
        let r = range("ab\ncd ef\n", 6, 8);
        assert_eq!(
            (r.start_line, r.start_character, r.end_line, r.end_character),
            (1, 3, 1, 5)
        );
    }

    #[test]
    fn columns_count_utf8_bytes_not_characters() {
        // "é" takes 2 bytes and "日" takes 3 bytes, so `name` starts at byte 8.
        let text = "é = 日 + name\n";
        let start = text.find("name").unwrap() as u32;
        assert_eq!(start, 11);
        let r = range(text, start, start + 4);
        assert_eq!((r.start_character, r.end_character), (11, 15));
    }

    #[test]
    fn non_ascii_line_after_ascii_line() {
        let text = "x = 1\nπ = 2\ny = π\n";
        let start = text.rfind('π').unwrap() as u32;
        let r = range(text, start, start + 'π'.len_utf8() as u32);
        assert_eq!(
            (r.start_line, r.start_character, r.end_character),
            (2, 4, 6)
        );
    }

    #[test]
    fn crlf_keeps_carriage_return_in_the_previous_line() {
        let text = "a = 1\r\nb = 2\r\n";
        let start = text.find('b').unwrap() as u32;
        let r = range(text, start, start + 1);
        assert_eq!((r.start_line, r.start_character), (1, 0));
        let cr = text.find('\r').unwrap() as u32;
        let r = range(text, cr - 1, cr + 1);
        assert_eq!(
            (r.start_line, r.start_character, r.end_character),
            (0, 4, 6)
        );
    }

    #[test]
    fn multi_line_span() {
        let text = "ab\ncd\nef";
        let r = range(text, 1, 7);
        assert_eq!(
            (r.start_line, r.start_character, r.end_line, r.end_character),
            (0, 1, 2, 1)
        );
    }

    #[test]
    fn span_may_end_at_the_end_of_the_text() {
        let r = range("abc", 0, 3);
        assert_eq!((r.end_line, r.end_character), (0, 3));
    }

    #[test]
    fn rejects_bad_spans() {
        let index = LineIndex::new("aé");
        assert!(index.range(1, 1).is_err(), "empty");
        assert!(index.range(2, 1).is_err(), "reversed");
        assert!(index.range(0, 9).is_err(), "past the end");
        assert!(index.range(2, 3).is_err(), "inside the two bytes of é");
        assert!(index.range(0, 2).is_err(), "ends inside é");
        assert!(index.range(0, 3).is_ok());
    }
}
