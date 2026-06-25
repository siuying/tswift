//! A recursive-descent + Pratt parser for the quick-swift frontend.
//!
//! [`parse`] turns a [`swift_lexer`] token stream into a [`swift_ast::Ast`].
//! Statements are parsed top-down; expressions use precedence climbing (Pratt)
//! with Swift's operator precedence so `1 + 2 * 3` and `a || b && c` nest
//! correctly. Coverage today is **Tier 0 + Tier 1a**: `let`/`var` bindings
//! (with patterns, type annotations, initializers), assignment statements,
//! tuples, member/tuple-index access, calls, unary and ternary expressions, and
//! the full binary operator set over every literal form.

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

    /// True when the cursor is an operator token whose text equals `op`.
    fn at_oper(&self, op: &str) -> bool {
        let t = self.peek();
        t.kind == TokenKind::Oper && t.text == op
    }

    /// True when the cursor is the keyword `kw`.
    fn at_keyword(&self, kw: &str) -> bool {
        let t = self.peek();
        t.kind == TokenKind::Keyword && t.text == kw
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
            let stmt = self.parse_statement()?;
            let root = self.ast.root();
            self.ast.append_child(root, stmt);
        }
        Ok(())
    }

    fn parse_statement(&mut self) -> Result<NodeId, ParseError> {
        if self.at_keyword("let") || self.at_keyword("var") {
            return self.parse_binding();
        }
        // Expression statement, possibly an assignment `lhs op= rhs`.
        let expr = self.parse_expr(0)?;
        if self.peek().kind == TokenKind::Oper && is_assignment(self.peek().text) {
            let op = self.bump();
            let rhs = self.parse_expr(0)?;
            let assign = self
                .ast
                .add(NodeKind::AssignExpr, Some(op.text), op.line, op.col);
            self.ast.append_child(assign, expr);
            self.ast.append_child(assign, rhs);
            return Ok(assign);
        }
        let e = self.ast.node(expr);
        let (line, col) = (e.line(), e.col());
        let stmt = self.ast.add(NodeKind::ExprStmt, None, line, col);
        self.ast.append_child(stmt, expr);
        Ok(stmt)
    }

    /// `let`/`var` pattern [`:` type] [`=` initializer].
    fn parse_binding(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let kind = if kw.text == "let" {
            NodeKind::LetDecl
        } else {
            NodeKind::VarDecl
        };
        let decl = self.ast.add(kind, None, kw.line, kw.col);

        let pattern = self.parse_pattern()?;
        self.ast.append_child(decl, pattern);

        if self.peek().kind == TokenKind::Colon {
            self.bump();
            let ty = self.parse_type()?;
            self.ast.append_child(decl, ty);
        }

        if self.at_oper("=") {
            self.bump();
            let init = self.parse_expr(0)?;
            self.ast.append_child(decl, init);
        }
        Ok(decl)
    }

    fn parse_pattern(&mut self) -> Result<NodeId, ParseError> {
        let t = self.peek();
        match t.kind {
            TokenKind::Identifier if t.text == "_" => {
                self.bump();
                Ok(self.ast.add(NodeKind::WildcardPattern, None, t.line, t.col))
            }
            TokenKind::Identifier => {
                self.bump();
                Ok(self
                    .ast
                    .add(NodeKind::NamePattern, Some(t.text), t.line, t.col))
            }
            TokenKind::LParen => {
                self.bump();
                let tuple = self.ast.add(NodeKind::TuplePattern, None, t.line, t.col);
                if self.peek().kind != TokenKind::RParen {
                    loop {
                        let sub = self.parse_pattern()?;
                        self.ast.append_child(tuple, sub);
                        if self.peek().kind == TokenKind::Comma {
                            self.bump();
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                Ok(tuple)
            }
            _ => self.error(format!("expected a binding pattern, found {:?}", t.kind)),
        }
    }

    /// Parse a type reference, recording its reconstructed source text. Supports
    /// names (optionally dotted/optional), array/dictionary `[...]`, and tuple
    /// `(...)` types — enough for Tier 1a annotations.
    fn parse_type(&mut self) -> Result<NodeId, ParseError> {
        let start = self.peek();
        let text = self.parse_type_text()?;
        Ok(self
            .ast
            .add(NodeKind::TypeRef, Some(&text), start.line, start.col))
    }

    fn parse_type_text(&mut self) -> Result<String, ParseError> {
        let mut text = match self.peek().kind {
            TokenKind::LBracket => {
                self.bump();
                let key = self.parse_type_text()?;
                if self.peek().kind == TokenKind::Colon {
                    self.bump();
                    let value = self.parse_type_text()?;
                    self.expect(TokenKind::RBracket)?;
                    format!("[{key}: {value}]")
                } else {
                    self.expect(TokenKind::RBracket)?;
                    format!("[{key}]")
                }
            }
            TokenKind::LParen => {
                self.bump();
                let mut parts = Vec::new();
                if self.peek().kind != TokenKind::RParen {
                    loop {
                        parts.push(self.parse_type_text()?);
                        if self.peek().kind == TokenKind::Comma {
                            self.bump();
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                format!("({})", parts.join(", "))
            }
            TokenKind::Identifier => {
                let mut name = self.bump().text.to_string();
                while self.peek().kind == TokenKind::Dot
                    && self.tokens[self.pos + 1].kind == TokenKind::Identifier
                {
                    self.bump();
                    name.push('.');
                    name.push_str(self.bump().text);
                }
                name
            }
            other => return self.error(format!("expected a type, found {other:?}")),
        };
        // Optional / IUO suffixes.
        while self.peek().kind == TokenKind::Question || self.at_oper("!") {
            text.push_str(self.bump().text);
        }
        Ok(text)
    }

    /// Pratt expression parser. `min_bp` is the minimum binding power that may
    /// bind on the left, so higher-precedence operators capture first.
    fn parse_expr(&mut self, min_bp: u8) -> Result<NodeId, ParseError> {
        let mut lhs = self.parse_prefix()?;
        loop {
            // Ternary conditional, right-associative at precedence `TERNARY_BP`.
            if self.peek().kind == TokenKind::Question && TERNARY_BP >= min_bp {
                let q = self.bump();
                let then_branch = self.parse_expr(0)?;
                self.expect(TokenKind::Colon)?;
                let else_branch = self.parse_expr(TERNARY_BP)?;
                let tern = self.ast.add(NodeKind::TernaryExpr, None, q.line, q.col);
                self.ast.append_child(tern, lhs);
                self.ast.append_child(tern, then_branch);
                self.ast.append_child(tern, else_branch);
                lhs = tern;
                continue;
            }
            let op = self.peek();
            if op.kind != TokenKind::Oper || is_assignment(op.text) {
                break;
            }
            let (lbp, rbp) = match binding_power(op.text) {
                Some(bp) => bp,
                None => break,
            };
            if lbp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.parse_expr(rbp)?;
            let bin = self
                .ast
                .add(NodeKind::BinaryExpr, Some(op.text), op.line, op.col);
            self.ast.append_child(bin, lhs);
            self.ast.append_child(bin, rhs);
            lhs = bin;
        }
        Ok(lhs)
    }

    /// A prefix unary operator, else a primary with trailing call/member suffixes.
    fn parse_prefix(&mut self) -> Result<NodeId, ParseError> {
        let t = self.peek();
        if t.kind == TokenKind::Oper && matches!(t.text, "-" | "+" | "!" | "~") {
            self.bump();
            let operand = self.parse_prefix()?;
            let node = self
                .ast
                .add(NodeKind::PrefixExpr, Some(t.text), t.line, t.col);
            self.ast.append_child(node, operand);
            return Ok(node);
        }
        let primary = self.parse_primary()?;
        self.parse_postfix(primary)
    }

    fn parse_primary(&mut self) -> Result<NodeId, ParseError> {
        let t = self.peek();
        let node = match t.kind {
            TokenKind::IntLiteral => {
                self.bump();
                self.ast
                    .add(NodeKind::IntegerLiteral, Some(t.text), t.line, t.col)
            }
            TokenKind::FloatLiteral => {
                self.bump();
                self.ast
                    .add(NodeKind::FloatLiteral, Some(t.text), t.line, t.col)
            }
            TokenKind::StringLiteral => {
                self.bump();
                self.ast
                    .add(NodeKind::StringLiteral, Some(t.text), t.line, t.col)
            }
            TokenKind::Keyword if t.text == "true" || t.text == "false" => {
                self.bump();
                self.ast
                    .add(NodeKind::BoolLiteral, Some(t.text), t.line, t.col)
            }
            TokenKind::Keyword if t.text == "nil" => {
                self.bump();
                self.ast.add(NodeKind::NilLiteral, None, t.line, t.col)
            }
            TokenKind::Identifier => {
                self.bump();
                self.ast
                    .add(NodeKind::IdentExpr, Some(t.text), t.line, t.col)
            }
            TokenKind::LParen => return self.parse_paren_or_tuple(),
            other => return self.error(format!("expected an expression, found {other:?}")),
        };
        Ok(node)
    }

    /// `( expr )` collapses to the inner expr; `( a, b, ... )` is a tuple.
    fn parse_paren_or_tuple(&mut self) -> Result<NodeId, ParseError> {
        let open = self.bump(); // '('
        let first = self.parse_expr(0)?;
        if self.peek().kind != TokenKind::Comma {
            self.expect(TokenKind::RParen)?;
            return Ok(first);
        }
        let tuple = self.ast.add(NodeKind::TupleExpr, None, open.line, open.col);
        self.ast.append_child(tuple, first);
        while self.peek().kind == TokenKind::Comma {
            self.bump();
            if self.peek().kind == TokenKind::RParen {
                break;
            }
            let next = self.parse_expr(0)?;
            self.ast.append_child(tuple, next);
        }
        self.expect(TokenKind::RParen)?;
        Ok(tuple)
    }

    /// Trailing call `(...)` and member/tuple-index `.x` / `.0` suffixes.
    fn parse_postfix(&mut self, mut expr: NodeId) -> Result<NodeId, ParseError> {
        loop {
            match self.peek().kind {
                TokenKind::LParen => {
                    let open = self.bump();
                    let call = self.ast.add(NodeKind::CallExpr, None, open.line, open.col);
                    self.ast.append_child(call, expr);
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
                    expr = call;
                }
                TokenKind::Dot => {
                    let dot = self.bump();
                    let name = self.peek();
                    if !matches!(name.kind, TokenKind::Identifier | TokenKind::IntLiteral) {
                        return self.error(format!(
                            "expected a member name or tuple index after '.', found {:?}",
                            name.kind
                        ));
                    }
                    self.bump();
                    let member =
                        self.ast
                            .add(NodeKind::MemberExpr, Some(name.text), dot.line, dot.col);
                    self.ast.append_child(member, expr);
                    expr = member;
                }
                _ => break,
            }
        }
        Ok(expr)
    }
}

/// Precedence of the ternary conditional (Swift `TernaryPrecedence`, /10).
const TERNARY_BP: u8 = 10;

/// Returns `(left_bp, right_bp)` for an infix operator, encoding precedence and
/// associativity (`right_bp < left_bp` ⇒ right-associative). `None` for tokens
/// that are not infix operators. Values mirror Swift's standard precedence
/// groups (divided by 10).
fn binding_power(op: &str) -> Option<(u8, u8)> {
    let p = match op {
        "<<" | ">>" | "&<<" | "&>>" => 16,
        "*" | "/" | "%" | "&" | "&*" => 15,
        "+" | "-" | "|" | "^" | "&+" | "&-" => 14,
        "..<" | "..." => 13,
        "??" => return Some((12, 11)), // right-associative
        "==" | "!=" | "<" | ">" | "<=" | ">=" | "===" | "!==" => 9,
        "&&" => 8,
        "||" => 7,
        _ => return None,
    };
    Some((p, p + 1))
}

/// Whether `op` is a plain or compound assignment operator.
fn is_assignment(op: &str) -> bool {
    matches!(
        op,
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^="
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ast_of(src: &str) -> Ast {
        parse(src).expect("parse ok")
    }

    fn dump(src: &str) -> String {
        let ast = ast_of(src);
        ast.node(ast.root()).dump()
    }

    /// The first statement under the source file.
    fn first_stmt(ast: &Ast) -> swift_ast::Node<'_> {
        ast.node(ast.root()).children().next().unwrap()
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
    fn logical_binds_looser_than_comparison() {
        // a == b && c  =>  &&(==(a, b), c)
        let ast = ast_of("a == b && c");
        let top = first_stmt(&ast).children().next().unwrap();
        assert_eq!(top.kind(), NodeKind::BinaryExpr);
        assert_eq!(top.text(), Some("&&"));
        assert_eq!(top.children().next().unwrap().text(), Some("=="));
    }

    #[test]
    fn simple_let_binding() {
        let ast = ast_of("let x = 42");
        let decl = first_stmt(&ast);
        assert_eq!(decl.kind(), NodeKind::LetDecl);
        let kids: Vec<_> = decl.children().map(|c| (c.kind(), c.text())).collect();
        assert_eq!(
            kids,
            vec![
                (NodeKind::NamePattern, Some("x")),
                (NodeKind::IntegerLiteral, Some("42")),
            ]
        );
    }

    #[test]
    fn var_with_type_annotation() {
        let ast = ast_of("var ratio: Double = 1.5");
        let decl = first_stmt(&ast);
        assert_eq!(decl.kind(), NodeKind::VarDecl);
        let kids: Vec<_> = decl.children().map(|c| (c.kind(), c.text())).collect();
        assert_eq!(
            kids,
            vec![
                (NodeKind::NamePattern, Some("ratio")),
                (NodeKind::TypeRef, Some("Double")),
                (NodeKind::FloatLiteral, Some("1.5")),
            ]
        );
    }

    #[test]
    fn typed_binding_without_initializer() {
        let ast = ast_of("var maybe: [Int: String]");
        let decl = first_stmt(&ast);
        let ty = decl.children().nth(1).unwrap();
        assert_eq!(ty.kind(), NodeKind::TypeRef);
        assert_eq!(ty.text(), Some("[Int: String]"));
    }

    #[test]
    fn tuple_decomposition_pattern() {
        let ast = ast_of("let (a, b) = (1, 2)");
        let decl = first_stmt(&ast);
        let pattern = decl.children().next().unwrap();
        assert_eq!(pattern.kind(), NodeKind::TuplePattern);
        let names: Vec<_> = pattern.children().map(|c| c.text()).collect();
        assert_eq!(names, vec![Some("a"), Some("b")]);
        let init = decl.children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::TupleExpr);
    }

    #[test]
    fn wildcard_pattern() {
        let ast = ast_of("let _ = 5");
        let pattern = first_stmt(&ast).children().next().unwrap();
        assert_eq!(pattern.kind(), NodeKind::WildcardPattern);
    }

    #[test]
    fn compound_assignment_statement() {
        let ast = ast_of("total += 3");
        let assign = first_stmt(&ast);
        assert_eq!(assign.kind(), NodeKind::AssignExpr);
        assert_eq!(assign.text(), Some("+="));
        assert_eq!(
            assign.children().next().unwrap().kind(),
            NodeKind::IdentExpr
        );
    }

    #[test]
    fn ternary_is_right_associative() {
        // a ? b : c ? d : e  =>  a ? b : (c ? d : e)
        let ast = ast_of("a ? b : c ? d : e");
        let tern = first_stmt(&ast).children().next().unwrap();
        assert_eq!(tern.kind(), NodeKind::TernaryExpr);
        let else_branch = tern.children().nth(2).unwrap();
        assert_eq!(else_branch.kind(), NodeKind::TernaryExpr);
    }

    #[test]
    fn prefix_operators() {
        let ast = ast_of("let n = -x");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::PrefixExpr);
        assert_eq!(init.text(), Some("-"));
        assert_eq!(init.children().next().unwrap().kind(), NodeKind::IdentExpr);
    }

    #[test]
    fn member_and_tuple_index_access() {
        let ast = ast_of("let v = pair.0");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::MemberExpr);
        assert_eq!(init.text(), Some("0"));
        assert_eq!(init.children().next().unwrap().text(), Some("pair"));
    }

    #[test]
    fn bool_and_nil_literals() {
        let ast = ast_of("let a = true");
        assert_eq!(
            first_stmt(&ast).children().nth(1).unwrap().kind(),
            NodeKind::BoolLiteral
        );
        let ast = ast_of("let b: Int? = nil");
        assert_eq!(
            first_stmt(&ast).children().nth(2).unwrap().kind(),
            NodeKind::NilLiteral
        );
    }

    #[test]
    fn parses_multiple_statements() {
        let ast = ast_of("let x = 1\nprint(x)");
        let stmts: Vec<_> = ast.node(ast.root()).children().collect();
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0].kind(), NodeKind::LetDecl);
        assert_eq!(stmts[1].kind(), NodeKind::ExprStmt);
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
