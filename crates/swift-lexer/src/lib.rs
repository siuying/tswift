//! A pure-Rust Swift lexer.
//!
//! [`tokenize`] turns UTF-8 Swift source into a flat, zero-copy [`Token`]
//! stream: each token borrows its lexeme directly from the source and carries
//! its 1-based line/column. This is the first stage of the quick-swift frontend
//! pipeline (lexer → ast → parser → sema), the pure-Rust replacement for the
//! vendored C `msf` frontend.
//!
//! Scope today is the **walking-skeleton subset** — enough to lex `print("hi")`
//! and integer arithmetic — grown tier-by-tier behind a stable interface.

#![forbid(unsafe_code)]

/// The lexical category of a [`Token`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// An identifier or (currently unclassified) keyword, e.g. `print`, `x`.
    Identifier,
    /// An integer literal, e.g. `42`. Underscores and radices come later.
    IntLiteral,
    /// A double-quoted string literal *including* its quotes, e.g. `"hi"`.
    StringLiteral,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `,`
    Comma,
    /// An operator token (`+`, `-`, `*`, `/`). Widened tier-by-tier.
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
}

/// An error produced while lexing, with the location it was detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

/// Lex `source` into a token stream terminated by a single [`TokenKind::Eof`].
///
/// Whitespace (including newlines) is consumed and used only to advance the
/// line/column counters; it produces no tokens. Returns the first [`LexError`]
/// encountered (e.g. an unterminated string).
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
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn run(mut self) -> Result<Vec<Token<'a>>, LexError> {
        let mut out = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.bytes.len() {
                out.push(Token {
                    kind: TokenKind::Eof,
                    text: "",
                    line: self.line,
                    col: self.col,
                });
                return Ok(out);
            }
            out.push(self.next_token()?);
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'\n' => {
                    self.pos += 1;
                    self.line += 1;
                    self.col = 1;
                }
                b' ' | b'\t' | b'\r' => {
                    self.pos += 1;
                    self.col += 1;
                }
                _ => break,
            }
        }
    }

    fn next_token(&mut self) -> Result<Token<'a>, LexError> {
        let start = self.pos;
        let line = self.line;
        let col = self.col;
        let c = self.bytes[start];

        let kind = match c {
            b'(' => {
                self.advance_byte();
                TokenKind::LParen
            }
            b')' => {
                self.advance_byte();
                TokenKind::RParen
            }
            b',' => {
                self.advance_byte();
                TokenKind::Comma
            }
            b'+' | b'-' | b'*' | b'/' => {
                self.advance_byte();
                TokenKind::Oper
            }
            b'"' => return self.string(start, line, col),
            b'0'..=b'9' => {
                while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                    self.advance_byte();
                }
                TokenKind::IntLiteral
            }
            _ if is_ident_start(c) => {
                while self.pos < self.bytes.len() && is_ident_continue(self.bytes[self.pos]) {
                    self.advance_byte();
                }
                TokenKind::Identifier
            }
            _ => {
                return Err(LexError {
                    message: format!("unexpected character {:?}", c as char),
                    line,
                    col,
                })
            }
        };

        Ok(Token {
            kind,
            text: &self.src[start..self.pos],
            line,
            col,
        })
    }

    /// Lex a double-quoted string literal; `start`/`line`/`col` mark the opening quote.
    fn string(&mut self, start: usize, line: u32, col: u32) -> Result<Token<'a>, LexError> {
        self.advance_byte(); // opening quote
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'"' => {
                    self.advance_byte(); // closing quote
                    return Ok(Token {
                        kind: TokenKind::StringLiteral,
                        text: &self.src[start..self.pos],
                        line,
                        col,
                    });
                }
                b'\\' => {
                    // Consume the backslash and the escaped char as a unit so an
                    // escaped quote does not terminate the literal.
                    self.advance_byte();
                    if self.pos < self.bytes.len() {
                        self.advance_byte();
                    }
                }
                b'\n' => break, // unterminated: a bare newline ends the line
                _ => self.advance_byte(),
            }
        }
        Err(LexError {
            message: "unterminated string literal".to_string(),
            line,
            col,
        })
    }

    /// Advance past one ASCII byte (every token byte in today's subset is ASCII),
    /// keeping the column counter in sync.
    fn advance_byte(&mut self) {
        self.pos += 1;
        self.col += 1;
    }
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
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
    fn empty_input_is_just_eof() {
        let toks = tokenize("").unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Eof);
    }

    #[test]
    fn whitespace_only_is_just_eof() {
        let toks = tokenize("   \t\n  ").unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Eof);
    }

    #[test]
    fn tracks_line_and_column() {
        let toks = tokenize("a\n  bb").unwrap();
        assert_eq!((toks[0].line, toks[0].col), (1, 1)); // a
        assert_eq!((toks[1].line, toks[1].col), (2, 3)); // bb
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
        let err = tokenize("@").unwrap_err();
        assert_eq!((err.line, err.col), (1, 1));
    }

    #[test]
    fn identifiers_allow_digits_and_underscores() {
        use TokenKind::*;
        assert_eq!(lex("_x1 y2"), vec![(Identifier, "_x1"), (Identifier, "y2")]);
    }
}
