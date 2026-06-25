//! A recursive-descent + Pratt parser for the quick-swift frontend.
//!
//! [`parse`] turns a [`swift_lexer`] token stream into a [`swift_ast::Ast`].
//! Statements are parsed top-down; expressions use precedence climbing (Pratt)
//! so `1 + 2 * 3` nests correctly. Scope today is the walking-skeleton subset:
//! expression statements over calls, identifiers, literals, and the four
//! arithmetic operators.

#![forbid(unsafe_code)]

use swift_ast::{Ast, NodeId, NodeKind};
use swift_lexer::{tokenize, Token, TokenKind};

/// A syntax error: a human-readable message and the 1-based location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

/// Lex and parse `source` into an [`Ast`], or return the first error.
pub fn parse(source: &str) -> Result<Ast, ParseError> {
    let tokens = tokenize(source).map_err(|e| ParseError {
        message: e.message,
        line: e.line,
        col: e.col,
    })?;
    let mut p = Parser {
        tokens,
        pos: 0,
        ast: Ast::new(),
    };
    p.parse_source_file()?;
    Ok(p.ast)
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    pos: usize,
    ast: Ast,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Token<'a> {
        self.tokens[self.pos]
    }

    fn bump(&mut self) -> Token<'a> {
        let t = self.tokens[self.pos];
        if t.kind != TokenKind::Eof {
            self.pos += 1;
        }
        t
    }

    fn at_eof(&self) -> bool {
        self.peek().kind == TokenKind::Eof
    }

    fn error<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        let t = self.peek();
        Err(ParseError {
            message: message.into(),
            line: t.line,
            col: t.col,
        })
    }

    fn expect(&mut self, kind: TokenKind) -> Result<Token<'a>, ParseError> {
        if self.peek().kind == kind {
            Ok(self.bump())
        } else {
            self.error(format!("expected {:?}, found {:?}", kind, self.peek().kind))
        }
    }

    fn parse_source_file(&mut self) -> Result<(), ParseError> {
        while !self.at_eof() {
            let expr = self.parse_expr(0)?;
            let t = self.ast.node(expr);
            let (line, col) = (t.line(), t.col());
            let stmt = self.ast.add(NodeKind::ExprStmt, None, line, col);
            self.ast.append_child(stmt, expr);
            let root = self.ast.root();
            self.ast.append_child(root, stmt);
        }
        Ok(())
    }

    /// Pratt expression parser. `min_bp` is the minimum binding power that may
    /// bind on the left, so higher-precedence operators capture first.
    fn parse_expr(&mut self, min_bp: u8) -> Result<NodeId, ParseError> {
        let mut lhs = self.parse_prefix()?;
        while self.peek().kind == TokenKind::Oper {
            let op = self.peek();
            let bp = binding_power(op.text);
            if bp < min_bp {
                break;
            }
            self.bump(); // operator
            let rhs = self.parse_expr(bp + 1)?;
            let bin = self
                .ast
                .add(NodeKind::BinaryExpr, Some(op.text), op.line, op.col);
            self.ast.append_child(bin, lhs);
            self.ast.append_child(bin, rhs);
            lhs = bin;
        }
        Ok(lhs)
    }

    /// A primary expression plus any trailing call suffix `(...)`.
    fn parse_prefix(&mut self) -> Result<NodeId, ParseError> {
        let t = self.peek();
        let primary = match t.kind {
            TokenKind::IntLiteral => {
                self.bump();
                self.ast
                    .add(NodeKind::IntegerLiteral, Some(t.text), t.line, t.col)
            }
            TokenKind::StringLiteral => {
                self.bump();
                self.ast
                    .add(NodeKind::StringLiteral, Some(t.text), t.line, t.col)
            }
            TokenKind::Identifier => {
                self.bump();
                self.ast
                    .add(NodeKind::IdentExpr, Some(t.text), t.line, t.col)
            }
            TokenKind::LParen => {
                self.bump();
                let inner = self.parse_expr(0)?;
                self.expect(TokenKind::RParen)?;
                inner
            }
            other => return self.error(format!("expected an expression, found {other:?}")),
        };
        self.parse_call_suffix(primary)
    }

    /// If a `(` follows, parse a call whose callee is `callee`.
    fn parse_call_suffix(&mut self, callee: NodeId) -> Result<NodeId, ParseError> {
        if self.peek().kind != TokenKind::LParen {
            return Ok(callee);
        }
        let lparen = self.bump();
        let call = self
            .ast
            .add(NodeKind::CallExpr, None, lparen.line, lparen.col);
        self.ast.append_child(call, callee);
        if self.peek().kind != TokenKind::RParen {
            loop {
                let arg = self.parse_expr(0)?;
                self.ast.append_child(call, arg);
                if self.peek().kind == TokenKind::Comma {
                    self.bump();
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen)?;
        // A call expression can itself be called again: `f()()`.
        self.parse_call_suffix(call)
    }
}

/// Binding power for an infix operator: multiplicative binds tighter than additive.
fn binding_power(op: &str) -> u8 {
    match op {
        "*" | "/" => 20,
        "+" | "-" => 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(src: &str) -> String {
        parse(src).expect("parse ok").node_root_dump()
    }

    trait RootDump {
        fn node_root_dump(&self) -> String;
    }
    impl RootDump for Ast {
        fn node_root_dump(&self) -> String {
            self.node(self.root()).dump()
        }
    }

    #[test]
    fn parses_print_string_call() {
        assert_eq!(
            dump(r#"print("hi")"#),
            "source_file L1\n  \
             expr_stmt L1\n    \
             call_expr L1\n      \
             ident_expr \"print\" L1\n      \
             string_literal \"\\\"hi\\\"\" L1\n"
        );
    }

    #[test]
    fn parses_arithmetic_with_precedence() {
        // 1 + 2 * 3  =>  +(1, *(2, 3))
        assert_eq!(
            dump("1 + 2 * 3"),
            "source_file L1\n  \
             expr_stmt L1\n    \
             binary_expr \"+\" L1\n      \
             integer_literal \"1\" L1\n      \
             binary_expr \"*\" L1\n        \
             integer_literal \"2\" L1\n        \
             integer_literal \"3\" L1\n"
        );
    }

    #[test]
    fn parens_override_precedence() {
        // (1 + 2) * 3  =>  *(+(1, 2), 3)
        assert_eq!(
            dump("(1 + 2) * 3"),
            "source_file L1\n  \
             expr_stmt L1\n    \
             binary_expr \"*\" L1\n      \
             binary_expr \"+\" L1\n        \
             integer_literal \"1\" L1\n        \
             integer_literal \"2\" L1\n      \
             integer_literal \"3\" L1\n"
        );
    }

    #[test]
    fn parses_multiple_statements() {
        let ast = parse("print(1)\nprint(2)").unwrap();
        let stmts: Vec<_> = ast.node(ast.root()).children().collect();
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0].line(), 1);
        assert_eq!(stmts[1].line(), 2);
    }

    #[test]
    fn missing_close_paren_is_an_error() {
        let err = parse("print(1").unwrap_err();
        assert!(err.message.contains("RParen"), "{}", err.message);
    }

    #[test]
    fn trailing_operator_is_an_error() {
        let err = parse("1 +").unwrap_err();
        assert!(err.message.contains("expression"), "{}", err.message);
    }
}
