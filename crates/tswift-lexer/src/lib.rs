//! A pure-Rust Swift lexer.
//!
//! [`tokenize`] turns UTF-8 Swift source into a flat, zero-copy [`Token`]
//! stream: each token borrows its lexeme directly from the source and carries
//! its 1-based line/column. This is the first stage of the tswift frontend
//! pipeline (lexer → ast → parser → sema), the pure-Rust replacement for the
//! vendored C `msf` frontend.
//!
//! Coverage today spans **Tier 0** lexical structure: integer literals in every
//! radix (with `_` separators), floating-point literals (decimal and hex), the
//! full operator set, string literals (single-line, multiline `"""`, and raw
//! `#"…"#`, with escapes), regex literals (`/…/` and extended `#/…/#`, with
//! `/`-vs-division disambiguation), line/block/nested comments, and unicode
//! identifiers. Keywords are lexed as [`TokenKind::Keyword`].

#![forbid(unsafe_code)]

/// The lexical category of a [`Token`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// An identifier, e.g. `print`, `café`, `_x1`.
    Identifier,
    /// A reserved word, e.g. `let`, `var`, `if`, `true`, `nil`.
    Keyword,
    /// An integer literal in any radix, e.g. `42`, `0xFF`, `0b1010`, `1_000`.
    IntLiteral,
    /// A floating-point literal, e.g. `3.14`, `1.5e3`, `0x1.8p1`.
    FloatLiteral,
    /// A double-quoted string literal *including* its quotes, e.g. `"hi"`.
    StringLiteral,
    /// A regex literal *including* its delimiters, e.g. `/\d+/` or `#/a\/b/#`.
    RegexLiteral,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `,`
    Comma,
    /// `:`
    Colon,
    /// `;`
    Semicolon,
    /// `.` (member access / tuple index; range operators lex as [`TokenKind::Oper`]).
    Dot,
    /// `?` (ternary / optional; `??` lexes as [`TokenKind::Oper`]).
    Question,
    /// Any operator lexeme (`+`, `==`, `&&`, `..<`, `->`, `&+`, …), text in [`Token::text`].
    Oper,
    /// An attribute, e.g. `@main`, `@discardableResult` (text includes the `@`).
    Attribute,
    /// A compiler directive, e.g. `#if`, `#file`, `#warning` (text includes the `#`).
    Directive,
    /// End of input. Always the final token of a [`tokenize`] result.
    Eof,
}

/// One lexical token: its kind, the exact source slice it spans, and the
/// 1-based line/column of its first character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokenKind,
    /// The verbatim source text of the token (zero-copy). Empty for [`TokenKind::Eof`].
    pub text: &'a str,
    /// 1-based source line of the token's first character.
    pub line: u32,
    /// 1-based source column of the token's first character.
    pub col: u32,
    /// Whether a newline was consumed between the previous token and this one.
    /// Lets the parser tell `break outer` (label) from `break` then a new line.
    pub leading_newline: bool,
}

/// An error produced while lexing, with the location it was detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

/// The Swift reserved words this lexer classifies as [`TokenKind::Keyword`].
const KEYWORDS: &[&str] = &[
    "let",
    "var",
    "func",
    "return",
    "if",
    "else",
    "guard",
    "while",
    "repeat",
    "for",
    "in",
    "switch",
    "case",
    "default",
    "break",
    "continue",
    "fallthrough",
    "where",
    "do",
    "catch",
    "throw",
    "throws",
    "rethrows",
    "try",
    "defer",
    "struct",
    "class",
    "enum",
    "protocol",
    "extension",
    "init",
    "deinit",
    "subscript",
    "self",
    "super",
    "nil",
    "true",
    "false",
    "is",
    "as",
    "inout",
    "import",
    "typealias",
    "static",
    "mutating",
    "some",
    "any",
    "associatedtype",
    "indirect",
    "lazy",
    "weak",
    "unowned",
    "open",
    "public",
    "internal",
    "fileprivate",
    "private",
    "package",
];

/// Operator lexemes, **ordered longest-first** for maximal munch.
const OPERATORS: &[&str] = &[
    "&<<", "&>>", "===", "!==", "..<", "...", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "??",
    "+=", "-=", "*=", "/=", "%=", "&+", "&-", "&*", "&=", "|=", "^=", "->", "<", ">", "+", "-",
    "*", "/", "%", "&", "|", "^", "~", "!", "=",
];

/// A "pure" operator byte: one that may form a (possibly custom) operator and
/// has no special standalone role (unlike `.` ranges or `?` ternary/optional).
fn is_pure_operator_byte(b: u8) -> bool {
    matches!(
        b,
        b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'=' | b'!' | b'&' | b'|' | b'^' | b'~'
    )
}

/// Lex `source` into a token stream terminated by a single [`TokenKind::Eof`].
///
/// Whitespace and comments are consumed and used only to advance the line/column
/// counters; they produce no tokens. Returns the first [`LexError`] encountered
/// (e.g. an unterminated string or block comment).
pub fn tokenize(source: &str) -> Result<Vec<Token<'_>>, LexError> {
    Lexer::new(source).run()
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    /// Byte offset of the next unconsumed character.
    pos: usize,
    line: u32,
    col: u32,
    /// Set when trivia before the next token contained a newline.
    pending_newline: bool,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
            pending_newline: false,
        }
    }

    fn run(mut self) -> Result<Vec<Token<'a>>, LexError> {
        let mut out = Vec::new();
        loop {
            self.pending_newline = self.skip_trivia()?;
            if self.pos >= self.bytes.len() {
                out.push(self.make(TokenKind::Eof, self.pos, self.line, self.col));
                return Ok(out);
            }
            let prev = out.last().copied();
            out.push(self.next_token(prev)?);
        }
    }

    /// Skip whitespace, line comments (`//`), and nested block comments (`/* */`).
    /// Returns whether any newline was consumed.
    fn skip_trivia(&mut self) -> Result<bool, LexError> {
        let mut saw_newline = false;
        loop {
            match self.peek() {
                Some(b'\n') => {
                    saw_newline = true;
                    self.pos += 1;
                    self.line += 1;
                    self.col = 1;
                }
                Some(b' ') | Some(b'\t') | Some(b'\r') => self.advance_byte(),
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.advance_byte();
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    saw_newline |= self.block_comment()?;
                }
                _ => return Ok(saw_newline),
            }
        }
    }

    /// Consume a `/* ... */` comment, honouring nesting. Returns whether it
    /// spanned a newline.
    fn block_comment(&mut self) -> Result<bool, LexError> {
        let (line, col) = (self.line, self.col);
        let mut saw_newline = false;
        self.advance_byte(); // '/'
        self.advance_byte(); // '*'
        let mut depth = 1;
        while depth > 0 {
            match self.peek() {
                None => {
                    return Err(LexError {
                        message: "unterminated block comment".to_string(),
                        line,
                        col,
                    })
                }
                Some(b'\n') => {
                    saw_newline = true;
                    self.pos += 1;
                    self.line += 1;
                    self.col = 1;
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    self.advance_byte();
                    self.advance_byte();
                    depth += 1;
                }
                Some(b'*') if self.peek_at(1) == Some(b'/') => {
                    self.advance_byte();
                    self.advance_byte();
                    depth -= 1;
                }
                Some(_) => self.advance_byte(),
            }
        }
        Ok(saw_newline)
    }

    fn next_token(&mut self, prev: Option<Token<'a>>) -> Result<Token<'a>, LexError> {
        let start = self.pos;
        let line = self.line;
        let col = self.col;
        let c = self.bytes[start];

        let kind = match c {
            b'(' => self.single(TokenKind::LParen),
            b')' => self.single(TokenKind::RParen),
            b'{' => self.single(TokenKind::LBrace),
            b'}' => self.single(TokenKind::RBrace),
            b'[' => self.single(TokenKind::LBracket),
            b']' => self.single(TokenKind::RBracket),
            b',' => self.single(TokenKind::Comma),
            b':' => self.single(TokenKind::Colon),
            b';' => self.single(TokenKind::Semicolon),
            b'"' => return self.string(start, line, col),
            // `#/…/#` extended regex literals are unambiguous regardless of
            // expression position (they cannot be a comment or division).
            b'#' if self.pound_regex_len().is_some() => {
                return self.consume_regex(self.pound_regex_len().unwrap(), start, line, col)
            }
            // `#"…"#` / `#"""…"""#` raw string literals (any number of hashes).
            b'#' if self.is_raw_string_start() => return self.string(start, line, col),
            // A `/` begins a regex literal only where an expression is expected
            // (otherwise it is division). `//` and `/*` are already consumed as
            // comments by `skip_trivia`, so a `/` reaching here is real syntax.
            b'/' if regex_allowed(prev) => match self.bare_regex_len() {
                Some(len) => return self.consume_regex(len, start, line, col),
                None => match self.match_operator() {
                    Some(len) => {
                        self.take(len);
                        TokenKind::Oper
                    }
                    None => unreachable!("'/' always matches an operator"),
                },
            },
            // `@name` attribute and `#name` compiler directive lex as one token.
            b'@' | b'#' => {
                self.advance_byte();
                while self.peek().is_some_and(is_ident_continue) {
                    self.advance_byte();
                }
                if c == b'@' {
                    TokenKind::Attribute
                } else {
                    TokenKind::Directive
                }
            }
            b'.' => {
                // `...`/`..<` are operators; a lone `.` is member/tuple access.
                if self.starts_with("...") || self.starts_with("..<") {
                    self.take(3);
                    TokenKind::Oper
                } else {
                    self.single(TokenKind::Dot)
                }
            }
            b'?' => {
                if self.starts_with("??") {
                    self.take(2);
                    TokenKind::Oper
                } else {
                    self.single(TokenKind::Question)
                }
            }
            // A bare backslash begins a key-path expression (`\Root.path`,
            // `\.path`). In-string interpolation `\(…)` is handled by the string
            // lexer, so a `\` reaching here is always a key-path sigil.
            b'\\' => self.single(TokenKind::Oper),
            // A backtick-escaped identifier — `` `default` `` — lets a reserved
            // word be used as a plain name. It lexes as a single
            // [`TokenKind::Identifier`] whose text is the *inner* slice (the
            // backticks are stripped), so `` `default` `` and a hypothetical
            // non-keyword `default` are indistinguishable to the parser.
            b'`' => {
                self.advance_byte(); // opening backtick
                let inner_start = self.pos;
                while let Some(b) = self.peek() {
                    if b < 0x80 {
                        if !is_ident_continue(b) {
                            break;
                        }
                        self.advance_byte();
                    } else {
                        let (ch, len) = self.scalar_at(self.pos);
                        if is_unicode_operator_scalar(ch) {
                            break;
                        }
                        self.take(len);
                    }
                }
                let inner_end = self.pos;
                if inner_end > inner_start && self.peek() == Some(b'`') {
                    self.advance_byte(); // closing backtick
                    return Ok(Token {
                        kind: TokenKind::Identifier,
                        text: &self.src[inner_start..inner_end],
                        line,
                        col,
                        leading_newline: self.pending_newline,
                    });
                }
                return Err(LexError {
                    message: "unterminated backtick-escaped identifier".to_string(),
                    line,
                    col,
                });
            }
            b'0'..=b'9' => self.number(),
            // A non-ASCII scalar is either a unicode *operator* character
            // (TSPL operator-head: `√`, `°`, `±`, arrows, math symbols, …) or
            // an identifier character — decode the scalar to decide.
            _ if c >= 0x80 && is_unicode_operator_scalar(self.scalar_at(start).0) => {
                self.take(self.scalar_at(start).1); // keeps `col` in sync
                loop {
                    match self.peek() {
                        Some(b) if b < 0x80 && is_pure_operator_byte(b) => self.advance_byte(),
                        Some(b) if b >= 0x80 => {
                            let (ch, len) = self.scalar_at(self.pos);
                            if is_unicode_operator_scalar(ch) {
                                self.take(len);
                            } else {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                TokenKind::Oper
            }
            _ if is_ident_start(c) => {
                while let Some(b) = self.peek() {
                    if b < 0x80 {
                        if !is_ident_continue(b) {
                            break;
                        }
                        self.advance_byte();
                    } else {
                        // A new non-ASCII scalar: an operator character ends
                        // the identifier (`degrees°`); anything else continues it.
                        let (ch, len) = self.scalar_at(self.pos);
                        if is_unicode_operator_scalar(ch) {
                            break;
                        }
                        self.take(len);
                    }
                }
                let text = &self.src[start..self.pos];
                if KEYWORDS.contains(&text) {
                    TokenKind::Keyword
                } else {
                    TokenKind::Identifier
                }
            }
            _ => match self.match_operator() {
                Some(len) => {
                    self.take(len);
                    TokenKind::Oper
                }
                None => {
                    return Err(LexError {
                        message: format!("unexpected character {:?}", c as char),
                        line,
                        col,
                    })
                }
            },
        };

        Ok(self.make(kind, start, line, col))
    }

    /// Scan a numeric literal beginning at the current digit. Returns whether it
    /// is integer or float; handles `0x`/`0o`/`0b` radices, `_` separators, a
    /// fractional part, and decimal/binary (`e`/`p`) exponents.
    fn number(&mut self) -> TokenKind {
        let mut is_float = false;
        if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x') | Some(b'X')) {
            self.take(2);
            self.take_while(is_hex_or_us);
            // hex fractional + binary exponent → float
            if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|c| c.is_ascii_hexdigit()) {
                is_float = true;
                self.advance_byte();
                self.take_while(is_hex_or_us);
            }
            if matches!(self.peek(), Some(b'p') | Some(b'P')) {
                is_float = true;
                self.advance_byte();
                self.take_sign_and_digits();
            }
        } else if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'o') | Some(b'O')) {
            self.take(2);
            self.take_while(|c| matches!(c, b'0'..=b'7' | b'_'));
        } else if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'b') | Some(b'B')) {
            self.take(2);
            self.take_while(|c| matches!(c, b'0' | b'1' | b'_'));
        } else {
            self.take_while(is_dec_or_us);
            // fractional part only when a digit follows the dot (else `.` is member access)
            if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
                is_float = true;
                self.advance_byte();
                self.take_while(is_dec_or_us);
            }
            if matches!(self.peek(), Some(b'e') | Some(b'E')) {
                is_float = true;
                self.advance_byte();
                self.take_sign_and_digits();
            }
        }
        if is_float {
            TokenKind::FloatLiteral
        } else {
            TokenKind::IntLiteral
        }
    }

    fn take_sign_and_digits(&mut self) {
        if matches!(self.peek(), Some(b'+') | Some(b'-')) {
            self.advance_byte();
        }
        self.take_while(is_dec_or_us);
    }

    /// Byte length of a bare `/…/` regex literal at the cursor (delimiters
    /// included), or `None` if no valid literal is present so `/` should lex as
    /// an operator.
    ///
    /// Mirrors Swift's bare-regex constraints: the body may not begin with a
    /// space/tab (which would be ambiguous with division), may not be empty,
    /// may not span a newline, and its closing `/` may not be preceded by a
    /// space. `\` escapes the next byte and `/` inside a `[…]` class is literal.
    fn bare_regex_len(&self) -> Option<usize> {
        debug_assert_eq!(self.bytes.get(self.pos), Some(&b'/'));
        let mut i = self.pos + 1;
        match self.bytes.get(i).copied() {
            None | Some(b' ') | Some(b'\t') | Some(b'/') | Some(b'\n') | Some(b'\r') => {
                return None
            }
            _ => {}
        }
        let mut in_class = false;
        while let Some(&b) = self.bytes.get(i) {
            match b {
                b'\\' => {
                    i += 1;
                    if i >= self.bytes.len() || self.bytes[i] == b'\n' {
                        return None;
                    }
                    i += 1;
                }
                b'\n' => return None,
                b'[' => {
                    in_class = true;
                    i += 1;
                }
                b']' if in_class => {
                    in_class = false;
                    i += 1;
                }
                b'/' if !in_class => {
                    if matches!(self.bytes[i - 1], b' ' | b'\t') {
                        return None;
                    }
                    return Some(i + 1 - self.pos);
                }
                _ => i += 1,
            }
        }
        None
    }

    /// Byte length of an extended `#/…/#` regex literal at the cursor
    /// (delimiters included), or `None` if the cursor is not at one. Any number
    /// of leading `#` is allowed; the body extends to a matching `/#*` and may
    /// contain newlines and unescaped whitespace.
    fn pound_regex_len(&self) -> Option<usize> {
        let hashes = self.leading_hashes();
        if hashes == 0 || self.peek_at(hashes) != Some(b'/') {
            return None;
        }
        let mut i = self.pos + hashes + 1;
        while let Some(&b) = self.bytes.get(i) {
            match b {
                b'\\' => i += 2,
                b'/' if (0..hashes).all(|h| self.bytes.get(i + 1 + h) == Some(&b'#')) => {
                    return Some(i + 1 + hashes - self.pos);
                }
                _ => i += 1,
            }
        }
        None
    }

    /// Consume `len` bytes of a regex literal beginning at `start`, tracking
    /// line/column across any embedded newlines, and emit the token.
    fn consume_regex(
        &mut self,
        len: usize,
        start: usize,
        line: u32,
        col: u32,
    ) -> Result<Token<'a>, LexError> {
        let end = start + len;
        while self.pos < end {
            self.advance_char();
        }
        Ok(self.make(TokenKind::RegexLiteral, start, line, col))
    }

    /// Whether the cursor begins a raw string literal: one or more `#` followed
    /// by a `"`, e.g. `#"…"#` or `##"…"##`.
    fn is_raw_string_start(&self) -> bool {
        let hashes = self.leading_hashes();
        hashes > 0 && self.peek_at(hashes) == Some(b'"')
    }

    /// Number of consecutive `#` at the cursor.
    fn leading_hashes(&self) -> usize {
        let mut n = 0;
        while self.peek_at(n) == Some(b'#') {
            n += 1;
        }
        n
    }

    /// Lex a string literal of any flavour — single-line, multiline (`"""`), and
    /// raw (`#"…"#`) — possibly combined. `start`/`line`/`col` mark the opening
    /// delimiter. The token text spans the full literal including its delimiters;
    /// escape processing and `\(…)` interpolation are decoded downstream.
    fn string(&mut self, start: usize, line: u32, col: u32) -> Result<Token<'a>, LexError> {
        let hashes = self.leading_hashes();
        self.take(hashes); // `#`* raw-string delimiter
        let multiline = self.starts_with("\"\"\"");
        let quotes = if multiline { 3 } else { 1 };
        self.take(quotes); // opening quote(s)
        loop {
            match self.peek() {
                None => break,
                Some(b'"') if self.at_closing_delimiter(quotes, hashes) => {
                    self.take(quotes + hashes);
                    return Ok(self.make(TokenKind::StringLiteral, start, line, col));
                }
                // `\(…)` interpolation: skip the whole balanced group so inner
                // quotes and `)` do not prematurely terminate the literal.
                Some(b'\\') if self.at_interpolation(hashes) => {
                    self.consume_interpolation(hashes, line, col)?;
                }
                // In a raw string `\` is literal; elsewhere it escapes the next
                // character (so an escaped quote does not terminate the literal).
                Some(b'\\') if hashes == 0 => {
                    self.advance_char();
                    if self.peek().is_some() {
                        self.advance_char();
                    }
                }
                Some(b'\n') if !multiline => break,
                _ => self.advance_char(),
            }
        }
        Err(LexError {
            message: "unterminated string literal".to_string(),
            line,
            col,
        })
    }

    /// Whether the cursor begins a `\(…)` interpolation — a `\`, the literal's
    /// `#` delimiter count, then `(`. Raw strings interpolate via `\#(…)`.
    fn at_interpolation(&self, hashes: usize) -> bool {
        self.peek() == Some(b'\\')
            && (0..hashes).all(|i| self.peek_at(1 + i) == Some(b'#'))
            && self.peek_at(1 + hashes) == Some(b'(')
    }

    /// Consume a `\(…)` interpolation, skipping the balanced parenthesised group
    /// and any nested string literals inside it.
    fn consume_interpolation(
        &mut self,
        hashes: usize,
        line: u32,
        col: u32,
    ) -> Result<(), LexError> {
        self.take(1 + hashes + 1); // `\` + `#`* + `(`
        let mut depth = 1usize;
        while depth > 0 {
            match self.peek() {
                None => {
                    return Err(LexError {
                        message: "unterminated string interpolation".to_string(),
                        line,
                        col,
                    })
                }
                Some(b'(') => {
                    depth += 1;
                    self.advance_char();
                }
                Some(b')') => {
                    depth -= 1;
                    self.advance_char();
                }
                Some(b'"') => self.skip_nested_string(),
                _ => self.advance_char(),
            }
        }
        Ok(())
    }

    /// Skip a simple nested string literal appearing inside an interpolation, so
    /// its contents are not mistaken for the enclosing literal's delimiters.
    fn skip_nested_string(&mut self) {
        self.advance_char(); // opening quote
        while let Some(c) = self.peek() {
            match c {
                b'"' => {
                    self.advance_char();
                    return;
                }
                b'\\' => {
                    self.advance_char();
                    if self.peek().is_some() {
                        self.advance_char();
                    }
                }
                b'\n' => return,
                _ => self.advance_char(),
            }
        }
    }

    /// Whether the cursor is at the literal's closing delimiter: `quotes` quote
    /// characters followed by exactly `hashes` `#`.
    fn at_closing_delimiter(&self, quotes: usize, hashes: usize) -> bool {
        (0..quotes).all(|i| self.peek_at(i) == Some(b'"'))
            && (0..hashes).all(|i| self.peek_at(quotes + i) == Some(b'#'))
            && self.peek_at(quotes + hashes) != Some(b'#')
    }

    /// Advance one byte, tracking line breaks so multiline strings keep accurate
    /// positions (unlike [`Lexer::advance_byte`], which only tracks columns).
    fn advance_char(&mut self) {
        if self.peek() == Some(b'\n') {
            self.pos += 1;
            self.line += 1;
            self.col = 1;
        } else {
            self.advance_byte();
        }
    }

    /// Longest operator lexeme matching at the cursor, if any (its byte length).
    fn match_operator(&self) -> Option<usize> {
        let base = OPERATORS
            .iter()
            .find(|op| self.starts_with(op))
            .map(|op| op.len())?;
        // Custom operators are maximal runs of operator characters (`^^`, `<>`,
        // `.+.`). When the matched table operator is built only from "pure"
        // operator characters (no `.`/`?`, which have special meaning), extend
        // it to consume the rest of a contiguous operator-character run.
        if self.bytes[self.pos..self.pos + base]
            .iter()
            .all(|b| is_pure_operator_byte(*b))
        {
            let mut len = base;
            loop {
                match self.peek_at(len) {
                    Some(b) if b < 0x80 && is_pure_operator_byte(b) => len += 1,
                    // A unicode operator scalar continues the run (`+°`).
                    Some(b) if b >= 0x80 => {
                        let (ch, scalar_len) = self.scalar_at(self.pos + len);
                        if is_unicode_operator_scalar(ch) {
                            len += scalar_len;
                        } else {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            return Some(len);
        }
        Some(base)
    }

    // --- cursor helpers ---

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        self.bytes[self.pos..].starts_with(s.as_bytes())
    }

    fn single(&mut self, kind: TokenKind) -> TokenKind {
        self.advance_byte();
        kind
    }

    /// Decode the UTF-8 scalar starting at byte offset `i` and its byte length.
    fn scalar_at(&self, i: usize) -> (char, usize) {
        let ch = self.src[i..].chars().next().unwrap_or('\u{FFFD}');
        (ch, ch.len_utf8())
    }

    fn take(&mut self, n: usize) {
        for _ in 0..n {
            self.advance_byte();
        }
    }

    fn take_while(&mut self, pred: fn(u8) -> bool) {
        while self.peek().is_some_and(pred) {
            self.advance_byte();
        }
    }

    /// Advance past one byte, keeping the column counter in sync. Columns count
    /// UTF-8 leading bytes (not continuation bytes), so multi-byte characters
    /// advance the column by one.
    fn advance_byte(&mut self) {
        let is_continuation = self.bytes[self.pos] & 0xC0 == 0x80;
        self.pos += 1;
        if !is_continuation {
            self.col += 1;
        }
    }

    fn make(&self, kind: TokenKind, start: usize, line: u32, col: u32) -> Token<'a> {
        Token {
            kind,
            text: &self.src[start..self.pos],
            line,
            col,
            leading_newline: self.pending_newline,
        }
    }
}

/// Whether a `/` following `prev` should be lexed as the start of a regex
/// literal (an expression is expected) rather than the division operator.
///
/// Division follows anything that *ends* an expression — an identifier, a
/// literal, a closing `)`/`]`, or a value keyword (`self`, `true`, …). A regex
/// may begin anywhere else: at file start, or after an operator, `(`, `,`, `:`,
/// `return`, etc.
fn regex_allowed(prev: Option<Token<'_>>) -> bool {
    let Some(t) = prev else { return true };
    match t.kind {
        TokenKind::Identifier
        | TokenKind::IntLiteral
        | TokenKind::FloatLiteral
        | TokenKind::StringLiteral
        | TokenKind::RegexLiteral
        | TokenKind::RParen
        | TokenKind::RBracket
        | TokenKind::RBrace
        | TokenKind::Dot => false,
        TokenKind::Keyword => !matches!(t.text, "true" | "false" | "nil" | "self" | "super"),
        _ => true,
    }
}

/// Whether a non-ASCII scalar is a Swift operator character (TSPL
/// "operator-head" unicode ranges — math symbols, arrows, dingbats, …).
fn is_unicode_operator_scalar(c: char) -> bool {
    matches!(u32::from(c),
        0x00A1..=0x00A7
        | 0x00A9 | 0x00AB | 0x00AC | 0x00AE
        | 0x00B0..=0x00B1 | 0x00B6 | 0x00BB | 0x00BF | 0x00D7 | 0x00F7
        | 0x2016..=0x2017 | 0x2020..=0x2027
        | 0x2030..=0x203E | 0x2041..=0x2053 | 0x2055..=0x205E
        | 0x2190..=0x23FF | 0x2500..=0x2775 | 0x2794..=0x2BFF
        | 0x2E00..=0x2E7F | 0x3001..=0x3003 | 0x3008..=0x3020 | 0x3030)
}

fn is_ident_start(c: u8) -> bool {
    // `$` begins closure shorthand args (`$0`) and projected wrapper values (`$x`).
    c == b'_' || c == b'$' || c.is_ascii_alphabetic() || c >= 0x80
}

fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c == b'$' || c.is_ascii_alphanumeric() || c >= 0x80
}

fn is_dec_or_us(c: u8) -> bool {
    c.is_ascii_digit() || c == b'_'
}

fn is_hex_or_us(c: u8) -> bool {
    c.is_ascii_hexdigit() || c == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect (kind, text) pairs, dropping the trailing Eof, for terse asserts.
    fn lex(src: &str) -> Vec<(TokenKind, &str)> {
        let toks = tokenize(src).expect("lex ok");
        assert_eq!(toks.last().unwrap().kind, TokenKind::Eof);
        toks[..toks.len() - 1]
            .iter()
            .map(|t| (t.kind, t.text))
            .collect()
    }

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src).into_iter().map(|(k, _)| k).collect()
    }

    #[test]
    fn lexes_print_string_call() {
        use TokenKind::*;
        assert_eq!(
            lex(r#"print("hi")"#),
            vec![
                (Identifier, "print"),
                (LParen, "("),
                (StringLiteral, r#""hi""#),
                (RParen, ")"),
            ]
        );
    }

    #[test]
    fn lexes_comma_separated_args() {
        use TokenKind::*;
        assert_eq!(
            lex("f(a, b)"),
            vec![
                (Identifier, "f"),
                (LParen, "("),
                (Identifier, "a"),
                (Comma, ","),
                (Identifier, "b"),
                (RParen, ")"),
            ]
        );
    }

    #[test]
    fn lexes_integer_arithmetic() {
        use TokenKind::*;
        assert_eq!(
            lex("1 + 2"),
            vec![(IntLiteral, "1"), (Oper, "+"), (IntLiteral, "2")]
        );
    }

    #[test]
    fn integer_radices_and_separators() {
        use TokenKind::*;
        assert_eq!(
            lex("1_000 0xFF 0o755 0b1010_1010"),
            vec![
                (IntLiteral, "1_000"),
                (IntLiteral, "0xFF"),
                (IntLiteral, "0o755"),
                (IntLiteral, "0b1010_1010"),
            ]
        );
    }

    #[test]
    fn float_forms() {
        use TokenKind::*;
        assert_eq!(
            lex("3.14 1.5e3 0x1.8p1 1_000.5"),
            vec![
                (FloatLiteral, "3.14"),
                (FloatLiteral, "1.5e3"),
                (FloatLiteral, "0x1.8p1"),
                (FloatLiteral, "1_000.5"),
            ]
        );
    }

    #[test]
    fn integer_dot_member_is_not_a_float() {
        use TokenKind::*;
        // `1.description` and `pair.0` keep the dot as member/index access.
        assert_eq!(kinds("pair.0"), vec![Identifier, Dot, IntLiteral]);
    }

    #[test]
    fn keywords_are_classified() {
        use TokenKind::*;
        assert_eq!(
            lex("let x = nil"),
            vec![
                (Keyword, "let"),
                (Identifier, "x"),
                (Oper, "="),
                (Keyword, "nil"),
            ]
        );
    }

    #[test]
    fn multi_char_operators_munch_longest() {
        use TokenKind::*;
        assert_eq!(
            kinds("a == b && c <= d"),
            vec![Identifier, Oper, Identifier, Oper, Identifier, Oper, Identifier]
        );
        assert_eq!(lex("a..<b")[1], (Oper, "..<"));
        assert_eq!(lex("a...b")[1], (Oper, "..."));
        assert_eq!(lex("x ?? y")[1], (Oper, "??"));
        assert_eq!(lex("a === b")[1], (Oper, "==="));
        assert_eq!(lex("m &+ n")[1], (Oper, "&+"));
    }

    #[test]
    fn ternary_question_and_colon_are_structural() {
        use TokenKind::*;
        assert_eq!(
            kinds("a ? b : c"),
            vec![Identifier, Question, Identifier, Colon, Identifier]
        );
    }

    #[test]
    fn line_and_block_comments_are_skipped() {
        use TokenKind::*;
        let src = "let a = 1 // trailing\n/* block /* nested */ */ let b = 2";
        assert_eq!(
            lex(src),
            vec![
                (Keyword, "let"),
                (Identifier, "a"),
                (Oper, "="),
                (IntLiteral, "1"),
                (Keyword, "let"),
                (Identifier, "b"),
                (Oper, "="),
                (IntLiteral, "2"),
            ]
        );
    }

    #[test]
    fn unterminated_block_comment_errors() {
        let err = tokenize("/* never closed").unwrap_err();
        assert!(err.message.contains("block comment"), "{}", err.message);
    }

    #[test]
    fn unicode_identifiers() {
        use TokenKind::*;
        assert_eq!(
            lex("café + 数値"),
            vec![(Identifier, "café"), (Oper, "+"), (Identifier, "数値")]
        );
    }

    #[test]
    fn leading_newline_is_recorded() {
        let toks = tokenize("break\nfoo break outer").unwrap();
        assert!(!toks[0].leading_newline); // break
        assert!(toks[1].leading_newline); // foo, after newline
        assert!(!toks[2].leading_newline); // break, same line as foo
        assert!(!toks[3].leading_newline); // outer, same line
    }

    #[test]
    fn empty_input_is_just_eof() {
        let toks = tokenize("").unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Eof);
    }

    #[test]
    fn tracks_line_and_column_across_multibyte() {
        // `café` is 5 bytes but 4 columns; the next token starts at column 6.
        let toks = tokenize("café x").unwrap();
        assert_eq!((toks[0].line, toks[0].col), (1, 1));
        assert_eq!((toks[1].line, toks[1].col), (1, 6));
    }

    #[test]
    fn string_keeps_escaped_quote() {
        assert_eq!(
            lex(r#""a\"b""#),
            vec![(TokenKind::StringLiteral, r#""a\"b""#)]
        );
    }

    #[test]
    fn multiline_string_spans_newlines() {
        let src = "\"\"\"\n  a\n  b\n  \"\"\"";
        assert_eq!(lex(src), vec![(TokenKind::StringLiteral, src)]);
        // The token after a multiline literal keeps an accurate line number.
        let toks = tokenize("let s = \"\"\"\nx\n\"\"\"\nlet y = 1").unwrap();
        let y = toks.iter().find(|t| t.text == "y").unwrap();
        assert_eq!(y.line, 4);
    }

    #[test]
    fn raw_string_ignores_escapes_and_inner_quotes() {
        assert_eq!(
            lex(r##"#"a "b" \n c"#"##),
            vec![(TokenKind::StringLiteral, r##"#"a "b" \n c"#"##)]
        );
        // A lone `"#` that is not the delimiter does not terminate a `##"…"##`.
        assert_eq!(
            lex(r###"##"x"# y"##"###),
            vec![(TokenKind::StringLiteral, r###"##"x"# y"##"###)]
        );
    }

    #[test]
    fn raw_multiline_string() {
        let src = "#\"\"\"\n line \\n stays raw\n \"\"\"#";
        assert_eq!(lex(src), vec![(TokenKind::StringLiteral, src)]);
    }

    #[test]
    fn interpolation_with_inner_quotes_stays_one_token() {
        let src = r#""value = \("inner")""#;
        assert_eq!(lex(src), vec![(TokenKind::StringLiteral, src)]);
    }

    #[test]
    fn unterminated_string_is_an_error() {
        let err = tokenize("\"oops").unwrap_err();
        assert!(err.message.contains("unterminated"), "{}", err.message);
        assert_eq!((err.line, err.col), (1, 1));
    }

    #[test]
    fn unexpected_character_is_an_error() {
        // A control character is neither an operator, identifier, nor structural
        // token, so it is rejected.
        let err = tokenize("\u{7}").unwrap_err();
        assert!(err.message.contains("unexpected"), "{}", err.message);
        assert_eq!(err.line, 1);
    }

    #[test]
    fn bare_backslash_is_a_key_path_sigil() {
        // A backslash outside a string begins a key-path expression; it lexes as
        // a single operator token rather than an error.
        let toks = tokenize("\\Root.name").unwrap();
        assert_eq!(toks[0].kind, TokenKind::Oper);
        assert_eq!(toks[0].text, "\\");
    }

    #[test]
    fn bare_regex_literal_in_expression_position() {
        use TokenKind::*;
        assert_eq!(
            lex("let r = /\\d+/"),
            vec![
                (Keyword, "let"),
                (Identifier, "r"),
                (Oper, "="),
                (RegexLiteral, "/\\d+/"),
            ]
        );
        // After `(` and after `:` an expression is expected.
        assert_eq!(lex("f(/a/)")[2], (RegexLiteral, "/a/"));
        assert_eq!(lex("g(of: /a-z/)")[4], (RegexLiteral, "/a-z/"));
        // A `/` inside a `[…]` class does not close the literal.
        assert_eq!(lex("let r = /[/a]/")[3], (RegexLiteral, "/[/a]/"));
        // An escaped `/` does not close the literal.
        assert_eq!(lex("let r = /a\\/b/")[3], (RegexLiteral, "/a\\/b/"));
    }

    #[test]
    fn slash_is_division_after_an_expression() {
        use TokenKind::*;
        assert_eq!(kinds("10 / 3"), vec![IntLiteral, Oper, IntLiteral]);
        assert_eq!(kinds("a / b"), vec![Identifier, Oper, Identifier]);
        assert_eq!(kinds("x /= 2"), vec![Identifier, Oper, IntLiteral]);
        // `count) / 2` — a `/` after `)` is division, not a regex.
        assert_eq!(lex("f() / 2")[3], (Oper, "/"));
    }

    #[test]
    fn slash_with_leading_or_trailing_space_is_not_regex() {
        use TokenKind::*;
        // `= / x` cannot be a regex (body starts with a space): division.
        assert_eq!(lex("let r = / x /")[2], (Oper, "="));
        assert_eq!(lex("let r = / x /")[3], (Oper, "/"));
    }

    #[test]
    fn extended_pound_regex_literal() {
        use TokenKind::*;
        // `#/…/#` is unambiguous: inner `/` and whitespace are literal.
        assert_eq!(lex("let r = #/a\\/b/#")[3], (RegexLiteral, "#/a\\/b/#"));
        assert_eq!(lex("let r = #/ \\d+ /#")[3], (RegexLiteral, "#/ \\d+ /#"));
        // A pound-regex is recognised even right after an identifier.
        assert_eq!(lex("s.contains(#/x/#)")[4], (RegexLiteral, "#/x/#"));
    }

    #[test]
    fn pound_regex_spans_newlines() {
        let src = "let r = #/\n  \\d+\n/#\nlet y = 1";
        let toks = tokenize(src).unwrap();
        let y = toks.iter().find(|t| t.text == "y").unwrap();
        assert_eq!(y.line, 4);
    }

    #[test]
    fn attributes_and_directives_lex_as_single_tokens() {
        assert_eq!(
            lex("@main @available(macOS)"),
            vec![
                (TokenKind::Attribute, "@main"),
                (TokenKind::Attribute, "@available"),
                (TokenKind::LParen, "("),
                (TokenKind::Identifier, "macOS"),
                (TokenKind::RParen, ")"),
            ]
        );
        assert_eq!(
            lex("#if #file #warning"),
            vec![
                (TokenKind::Directive, "#if"),
                (TokenKind::Directive, "#file"),
                (TokenKind::Directive, "#warning"),
            ]
        );
    }

    #[test]
    fn backtick_escapes_reserved_word_as_identifier() {
        // The inner slice (backticks stripped) is the token text, so a
        // backtick-escaped keyword is indistinguishable from a plain name.
        assert_eq!(
            lex("let `default` = `class`"),
            vec![
                (TokenKind::Keyword, "let"),
                (TokenKind::Identifier, "default"),
                (TokenKind::Oper, "="),
                (TokenKind::Identifier, "class"),
            ]
        );
    }

    #[test]
    fn unterminated_backtick_identifier_errors() {
        assert!(tokenize("let `oops = 1").is_err());
        assert!(tokenize("let `` = 1").is_err());
    }
}
