//! A pure-Rust Swift lexer.
//!
//! [`tokenize`] turns UTF-8 Swift source into a flat, zero-copy [`Token`]
//! stream: each token borrows its lexeme directly from the source and carries
//! its 1-based line/column. This is the first stage of the quick-swift frontend
//! pipeline (lexer → ast → parser → sema), the pure-Rust replacement for the
//! vendored C `msf` frontend.
//!
//! Coverage today spans **Tier 0** lexical structure: integer literals in every
//! radix (with `_` separators), floating-point literals (decimal and hex), the
//! full operator set, string literals (with escapes), line/block/nested
//! comments, and unicode identifiers. Keywords are lexed as [`TokenKind::Keyword`].

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
            out.push(self.next_token()?);
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

    fn next_token(&mut self) -> Result<Token<'a>, LexError> {
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
            b'0'..=b'9' => self.number(),
            _ if is_ident_start(c) => {
                while self.peek().is_some_and(is_ident_continue) {
                    self.advance_byte();
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

    /// Lex a double-quoted string literal; `start`/`line`/`col` mark the opening quote.
    fn string(&mut self, start: usize, line: u32, col: u32) -> Result<Token<'a>, LexError> {
        self.advance_byte(); // opening quote
        while let Some(c) = self.peek() {
            match c {
                b'"' => {
                    self.advance_byte();
                    return Ok(self.make(TokenKind::StringLiteral, start, line, col));
                }
                b'\\' => {
                    // Consume the backslash and the escaped char together so an
                    // escaped quote does not terminate the literal.
                    self.advance_byte();
                    if self.peek().is_some() {
                        self.advance_byte();
                    }
                }
                b'\n' => break,
                _ => self.advance_byte(),
            }
        }
        Err(LexError {
            message: "unterminated string literal".to_string(),
            line,
            col,
        })
    }

    /// Longest operator lexeme matching at the cursor, if any (its byte length).
    fn match_operator(&self) -> Option<usize> {
        OPERATORS
            .iter()
            .find(|op| self.starts_with(op))
            .map(|op| op.len())
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

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic() || c >= 0x80
}

fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric() || c >= 0x80
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
    fn unterminated_string_is_an_error() {
        let err = tokenize("\"oops").unwrap_err();
        assert!(err.message.contains("unterminated"), "{}", err.message);
        assert_eq!((err.line, err.col), (1, 1));
    }

    #[test]
    fn unexpected_character_is_an_error() {
        // A bare backslash is neither an operator, identifier, nor structural token.
        let err = tokenize("\\").unwrap_err();
        assert!(err.message.contains("unexpected"), "{}", err.message);
        assert_eq!(err.line, 1);
    }
}
