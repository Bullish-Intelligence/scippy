//! Repair of Python 3.14 syntax that `tree-sitter-python` 0.23.5 rejects.
//!
//! The core calls [`repair`] for a file that has a syntax error (`Analysis::repair_syntax`).
//! The core parses the result. The result keeps the length in bytes and every line break, so each
//! position stays valid. Each repair changes ASCII bytes to ASCII bytes only.
//!
//! The scanner follows the lexical rules of Python: comments, line continuations, brackets, and
//! all string forms, including the nested fields of f-strings and t-strings (PEP 701).
//! It runs on the original text, so it never reads text inside a string as code.
//!
//! | Rule | Construct | Repair |
//! |---|---|---|
//! | R-1 | PEP 758: `except A, B, C:` and `except* A, B:` | Each comma of the type list that is not in a bracket becomes `\|`. |
//! | R-2 | PEP 750: `t"..{x}.."` | The prefix letter `t` becomes `f`. The fields keep their references. |
//! | R-3 | PEP 696: `class C[T = int]`, `def f[T = int]`, `type X[T = int] = ...` | The default, from `=` to the next comma or `]`, becomes spaces. |
//!
//! R-1 skips a clause that has `as` or `lambda` outside a bracket. Python 3.14 rejects `as` there.
//! A comma list with `|` between the names gives the same references as the list in brackets.
//! R-3 drops the references inside the default. The type parameter keeps its declaration.
//! A repair that leaves an error gives a `syntax-error`, as before.

/// The repaired text, or `None` when no rule applies.
pub fn repair(source: &str) -> Option<String> {
    let mut scanner = Scanner::new(source.as_bytes());
    scanner.run();
    if scanner.edits.is_empty() {
        return None;
    }
    let mut bytes = source.as_bytes().to_vec();
    for &(at, byte) in &scanner.edits {
        bytes[at] = byte;
    }
    // Each edit changes one ASCII byte of the text into another, or a byte of a multi-byte
    // character into a space. The second case happens only inside a blanked default, where
    // every byte of the character is blanked, so the text stays valid UTF-8.
    String::from_utf8(bytes).ok()
}

/// The `except` clause that the scanner reads now.
struct Clause {
    /// The positions of the commas that no bracket holds.
    commas: Vec<usize>,
    /// The clause has `as` or `lambda` outside a bracket. R-1 leaves it as it is.
    unsafe_: bool,
}

struct Scanner<'a> {
    src: &'a [u8],
    pos: usize,
    edits: Vec<(usize, u8)>,
    /// The nesting depth of `(`, `[`, and `{`.
    depth: usize,
    /// The next token starts a logical line.
    line_start: bool,
    clause: Option<Clause>,
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

fn is_quote(b: u8) -> bool {
    b == b'"' || b == b'\''
}

/// The string prefixes of Python 3.14.
fn is_prefix(word: &[u8]) -> bool {
    if word.len() > 2 {
        return false;
    }
    let lower: Vec<u8> = word.iter().map(u8::to_ascii_lowercase).collect();
    matches!(
        lower.as_slice(),
        b"r" | b"u" | b"b" | b"f" | b"t" | b"br" | b"rb" | b"fr" | b"rf" | b"tr" | b"rt"
    )
}

impl<'a> Scanner<'a> {
    fn new(src: &'a [u8]) -> Self {
        Scanner {
            src,
            pos: 0,
            edits: Vec::new(),
            depth: 0,
            line_start: true,
            clause: None,
        }
    }

    fn at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn skip_comment(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
            self.pos += 1;
        }
    }

    /// Read a name or a string at the position. A string is consumed whole and gives `None`.
    /// A name gives its range.
    fn word_or_string(&mut self) -> Option<(usize, usize)> {
        let start = self.pos;
        if is_quote(self.src[start]) {
            self.string(start, start);
            return None;
        }
        let mut end = start;
        while end < self.src.len() && is_word_byte(self.src[end]) {
            end += 1;
        }
        if end < self.src.len() && is_quote(self.src[end]) && is_prefix(&self.src[start..end]) {
            self.pos = end;
            self.string(start, end);
            return None;
        }
        self.pos = end;
        Some((start, end))
    }

    /// Skip one string. The prefix is `src[prefix_start..quote_at]`. The position is at the
    /// opening quote. The end position is after the closing quote.
    fn string(&mut self, prefix_start: usize, quote_at: usize) {
        let prefix = &self.src[prefix_start..quote_at];
        let raw = prefix.iter().any(|b| matches!(b, b'r' | b'R'));
        let template = prefix.iter().position(|b| matches!(b, b't' | b'T'));
        if let Some(i) = template {
            // R-2: the grammar knows the f-string, and the syntax is the same.
            let letter = if prefix[i] == b't' { b'f' } else { b'F' };
            self.edits.push((prefix_start + i, letter));
        }
        let interpolated = template.is_some() || prefix.iter().any(|b| matches!(b, b'f' | b'F'));
        let quote = self.src[quote_at];
        let triple = self.src.get(quote_at + 1) == Some(&quote)
            && self.src.get(quote_at + 2) == Some(&quote);
        self.pos = quote_at + if triple { 3 } else { 1 };
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if b == b'\\' {
                match self.at(1) {
                    // A field follows the backslash. The backslash is text.
                    Some(b'{' | b'}') if interpolated => self.pos += 1,
                    // `\N{name}` is an escape, not a field.
                    Some(b'N') if interpolated && !raw && self.at(2) == Some(b'{') => {
                        while self.pos < self.src.len() && self.src[self.pos] != b'}' {
                            self.pos += 1;
                        }
                        self.pos += 1;
                    }
                    _ => self.pos += 2,
                }
            } else if b == quote {
                if !triple {
                    self.pos += 1;
                    return;
                }
                if self.at(1) == Some(quote) && self.at(2) == Some(quote) {
                    self.pos += 3;
                    return;
                }
                self.pos += 1;
            } else if b == b'\n' && !triple {
                // An unterminated string. The grammar reports it.
                return;
            } else if b == b'{' && interpolated {
                if self.at(1) == Some(b'{') {
                    self.pos += 2;
                } else {
                    self.pos += 1;
                    self.field();
                }
            } else if b == b'}' && interpolated && self.at(1) == Some(b'}') {
                self.pos += 2;
            } else {
                self.pos += 1;
            }
        }
    }

    /// Skip the replacement field of an f-string or a t-string, after its `{`.
    /// The end position is after the matching `}`.
    fn field(&mut self) {
        let mut depth = 0usize;
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            match b {
                b'#' => self.skip_comment(),
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    self.pos += 1;
                }
                b')' | b']' => {
                    depth = depth.saturating_sub(1);
                    self.pos += 1;
                }
                b'}' => {
                    self.pos += 1;
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                b':' if depth == 0 => {
                    self.pos += 1;
                    self.format_spec();
                    return;
                }
                _ if is_quote(b) || is_word_byte(b) => {
                    self.word_or_string();
                }
                _ => self.pos += 1,
            }
        }
    }

    /// Skip the format specification of a field, up to and with its `}`.
    /// The text of a specification is literal. A `{` in it opens a nested field.
    fn format_spec(&mut self) {
        while self.pos < self.src.len() {
            match self.src[self.pos] {
                b'}' => {
                    self.pos += 1;
                    return;
                }
                b'{' => {
                    self.pos += 1;
                    self.field();
                }
                // The closing quote of the string, or the end of a one-line string. The string
                // reader handles both.
                b'\n' | b'"' | b'\'' => return,
                _ => self.pos += 1,
            }
        }
    }

    fn run(&mut self) {
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            match b {
                b'#' => self.skip_comment(),
                b'\n' => {
                    self.pos += 1;
                    if self.depth == 0 {
                        self.clause = None;
                        self.line_start = true;
                    }
                }
                b'\\' if matches!(self.at(1), Some(b'\n')) => self.pos += 2,
                b'\\' if self.at(1) == Some(b'\r') && self.at(2) == Some(b'\n') => self.pos += 3,
                b' ' | b'\t' | b'\x0c' | b'\r' => self.pos += 1,
                b'(' | b'[' | b'{' => {
                    self.depth += 1;
                    self.pos += 1;
                    self.line_start = false;
                }
                b')' | b']' | b'}' => {
                    self.depth = self.depth.saturating_sub(1);
                    self.pos += 1;
                    self.line_start = false;
                }
                b',' => {
                    if self.depth == 0 {
                        if let Some(clause) = &mut self.clause {
                            clause.commas.push(self.pos);
                        }
                    }
                    self.pos += 1;
                    self.line_start = false;
                }
                b':' => {
                    if self.depth == 0 && self.at(1) != Some(b'=') {
                        self.end_clause();
                    }
                    self.pos += 1;
                    self.line_start = false;
                }
                _ if is_quote(b) || is_word_byte(b) => {
                    let at_start = self.line_start;
                    self.line_start = false;
                    if let Some((start, end)) = self.word_or_string() {
                        let src = self.src;
                        self.word(&src[start..end], at_start);
                    }
                }
                _ => {
                    self.pos += 1;
                    self.line_start = false;
                }
            }
        }
    }

    /// A name that the scanner read. `at_start` tells that it starts a logical line.
    fn word(&mut self, word: &[u8], at_start: bool) {
        match word {
            b"except" if at_start => {
                self.clause = Some(Clause {
                    commas: Vec::new(),
                    unsafe_: false,
                });
            }
            b"as" | b"lambda" if self.depth == 0 => {
                if let Some(clause) = &mut self.clause {
                    clause.unsafe_ = true;
                }
            }
            // `async def` starts a logical line with `async`.
            b"async" if at_start => self.line_start = true,
            b"def" | b"class" | b"type" if at_start => self.type_parameters(),
            _ => {}
        }
    }

    /// The `:` that ends the header of an `except` clause. R-1.
    fn end_clause(&mut self) {
        if let Some(clause) = self.clause.take() {
            if !clause.unsafe_ {
                self.edits
                    .extend(clause.commas.iter().map(|&at| (at, b'|')));
            }
        }
    }

    /// Skip blanks and line continuations. A line break ends the logical line, so it stays.
    fn skip_space(&mut self) {
        while let Some(b) = self.at(0) {
            match b {
                b' ' | b'\t' | b'\x0c' => self.pos += 1,
                b'\\' if self.at(1) == Some(b'\n') => self.pos += 2,
                _ => return,
            }
        }
    }

    /// After `def`, `class`, or `type`: read the name. When a `[` follows it, read the list of
    /// type parameters. R-3.
    fn type_parameters(&mut self) {
        self.skip_space();
        let start = self.pos;
        while self.at(0).is_some_and(is_word_byte) {
            self.pos += 1;
        }
        if self.pos == start {
            return;
        }
        self.skip_space();
        if self.at(0) != Some(b'[') {
            return;
        }
        self.pos += 1;
        let mut depth = 1usize;
        let mut default: Option<usize> = None;
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            match b {
                b'#' => self.skip_comment(),
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    self.pos += 1;
                }
                b')' | b'}' => {
                    depth = depth.saturating_sub(1);
                    self.pos += 1;
                }
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        self.blank(default.take(), self.pos);
                        self.pos += 1;
                        return;
                    }
                    self.pos += 1;
                }
                b',' if depth == 1 => {
                    self.blank(default.take(), self.pos);
                    self.pos += 1;
                }
                b'=' if depth == 1
                    && default.is_none()
                    && self.at(1) != Some(b'=')
                    && !matches!(self.src[self.pos - 1], b'=' | b'!' | b'<' | b'>' | b':') =>
                {
                    default = Some(self.pos);
                    self.pos += 1;
                }
                _ if is_quote(b) || is_word_byte(b) => {
                    self.word_or_string();
                }
                _ => self.pos += 1,
            }
        }
    }

    /// Blank the bytes from `from` to `to`. Line breaks stay.
    fn blank(&mut self, from: Option<usize>, to: usize) {
        let Some(from) = from else { return };
        for at in from..to {
            if !matches!(self.src[at], b'\n' | b'\r') {
                self.edits.push((at, b' '));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::repair;

    fn fixed(source: &str) -> String {
        let out = repair(source).expect("a repair");
        assert_eq!(out.len(), source.len(), "the length stays");
        assert_eq!(
            out.matches('\n').count(),
            source.matches('\n').count(),
            "the line breaks stay"
        );
        out
    }

    #[test]
    fn r1_a_comma_list_becomes_a_union() {
        assert_eq!(
            fixed("try:\n    pass\nexcept A, B, C:\n    pass\n"),
            "try:\n    pass\nexcept A| B| C:\n    pass\n"
        );
        assert_eq!(
            fixed("try: x\nexcept* A, B: y\n"),
            "try: x\nexcept* A| B: y\n"
        );
        assert_eq!(
            fixed("try: x\nexcept a.b, c.d: y\n"),
            "try: x\nexcept a.b| c.d: y\n"
        );
    }

    #[test]
    fn r1_a_comma_in_a_bracket_stays() {
        assert!(repair("try: x\nexcept (A, B): y\n").is_none());
        assert!(repair("try: x\nexcept f(A, B): y\n").is_none());
        assert_eq!(
            fixed("try: x\nexcept (A, B), C: y\n"),
            "try: x\nexcept (A, B)| C: y\n"
        );
    }

    #[test]
    fn r1_a_clause_with_as_or_lambda_stays() {
        assert!(repair("try: x\nexcept A, B, C as e: y\n").is_none());
        assert!(repair("try: x\nexcept lambda: A, B: y\n").is_none());
    }

    #[test]
    fn r1_text_in_a_string_or_a_comment_is_not_a_clause() {
        let text =
            "x = \"\"\"\nexcept A, B, C:\n\"\"\"\n# except A, B, C:\ny = 'except A, B, C:'\n";
        assert!(repair(text).is_none());
        assert!(repair("def f():\n    return 1  # t\"x\"\n").is_none());
    }

    #[test]
    fn r1_a_clause_inside_a_block_and_after_a_string_is_found() {
        let text =
            "s = f\"{'}'}\"\nif x:\n    try:\n        pass\n    except A, B, C:\n        pass\n";
        assert_eq!(
            fixed(text),
            "s = f\"{'}'}\"\nif x:\n    try:\n        pass\n    except A| B| C:\n        pass\n"
        );
    }

    #[test]
    fn r1_a_multi_line_clause_keeps_its_line_breaks() {
        assert_eq!(
            fixed("try: x\nexcept (A,\n  B), C, \\\n  D:\n    y\n"),
            "try: x\nexcept (A,\n  B)| C| \\\n  D:\n    y\n"
        );
    }

    #[test]
    fn r2_the_t_prefix_becomes_f() {
        assert_eq!(fixed("a = t\"x {y}\"\n"), "a = f\"x {y}\"\n");
        assert_eq!(fixed("a = T'x'\n"), "a = F'x'\n");
        assert_eq!(fixed("a = rt\"\\d{y}\"\n"), "a = rf\"\\d{y}\"\n");
        assert_eq!(
            fixed("a = Tr\"\"\"x\n{y}\"\"\"\n"),
            "a = Fr\"\"\"x\n{y}\"\"\"\n"
        );
    }

    #[test]
    fn r2_a_nested_template_is_found() {
        assert_eq!(
            fixed("a = t\"o {f\"i {x}\"} {t\"d {y!r:>{w}}\"}\"\n"),
            "a = f\"o {f\"i {x}\"} {f\"d {y!r:>{w}}\"}\"\n"
        );
    }

    #[test]
    fn r2_a_name_called_t_and_other_prefixes_stay() {
        assert!(repair("t = 1\nprint(t)\nu = b\"t\" + rb\"x\" + f\"{t}\"\n").is_none());
        assert!(repair("a = \"t\\\"x\"\nb = 'it\\'s'\n").is_none());
    }

    #[test]
    fn r2_a_template_after_an_escape_and_a_literal_brace_is_found() {
        assert_eq!(
            fixed("a = f\"{{ t\\\" }}\" + t\"{{x}}\"\n"),
            "a = f\"{{ t\\\" }}\" + f\"{{x}}\"\n"
        );
    }

    #[test]
    fn r3_a_default_is_blanked() {
        assert_eq!(
            fixed("class C[T = int]:\n    pass\n"),
            "class C[T      ]:\n    pass\n"
        );
        assert_eq!(
            fixed("def f[T: A = B, *Ts = *tuple[int], **P = [int, str]](x): pass\n"),
            "def f[T: A    , *Ts              , **P             ](x): pass\n"
        );
        assert_eq!(
            fixed("type X[T = Y] = list[T]\n"),
            "type X[T    ] = list[T]\n"
        );
        assert_eq!(
            fixed("async def f[T = int](): pass\n"),
            "async def f[T      ](): pass\n"
        );
    }

    #[test]
    fn r3_a_default_with_a_string_and_a_line_break_is_blanked() {
        assert_eq!(
            fixed("class C[\n    T = \"a,]\",\n    U = (1,\n         2),\n]:\n    pass\n"),
            "class C[\n    T        ,\n    U      \n           ,\n]:\n    pass\n"
        );
    }

    #[test]
    fn r3_a_list_without_a_default_stays() {
        assert!(repair("class C[T: int, *Ts, **P]:\n    pass\n").is_none());
        assert!(repair("def f[T: (a == b)](x): pass\n").is_none());
        assert!(repair("class C(Base[T], x=1):\n    pass\n").is_none());
        assert!(repair("def f(x=1): pass\n").is_none());
        // `type` as a name, and a subscript with `=`.
        assert!(repair("type = 5\ntype[x] = 1\nclass_ = d[k = 1]\n").is_none());
    }

    #[test]
    fn a_multibyte_character_in_a_blanked_default_stays_valid_text() {
        let out = fixed("class C[T = \"é\"]:\n    pass\n");
        assert_eq!(out, "class C[T       ]:\n    pass\n");
    }
}
