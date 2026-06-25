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
        no_trailing_closure: false,
    };
    p.parse_source_file()?;
    Ok(p.ast)
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    pos: usize,
    ast: Ast,
    /// When set, a `{` after an expression is a control-flow body, not a
    /// trailing closure (true while parsing conditions, iterables, subjects).
    no_trailing_closure: bool,
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
        // Labeled loop/switch: `outer: for ...`.
        if self.peek().kind == TokenKind::Identifier
            && self.tokens[self.pos + 1].kind == TokenKind::Colon
            && is_labelable(self.tokens[self.pos + 2].text)
        {
            let label = self.bump();
            self.bump(); // ':'
            return self.parse_labeled(label.text);
        }
        // Skip declaration modifiers (`static`, `public`, `final`, …) that
        // precede a declaration keyword.
        self.skip_modifiers();
        // Compiler directives as statements (`#if`, `#warning`, `#error`).
        if self.peek().kind == TokenKind::Directive {
            return self.parse_directive_stmt();
        }
        // Custom operator / precedence-group declarations (contextual keywords).
        if self.peek().kind == TokenKind::Identifier {
            let w = self.peek().text;
            if w == "operator"
                || w == "precedencegroup"
                || (matches!(w, "infix" | "prefix" | "postfix")
                    && self.tokens[self.pos + 1].text == "operator")
            {
                return self.parse_operator_or_precedence();
            }
        }
        if self.peek().kind == TokenKind::Keyword {
            match self.peek().text {
                "do" => return self.parse_do(),
                "throw" => return self.parse_throw(),
                "defer" => return self.parse_defer(),
                "let" | "var" => {
                    let decl = self.parse_binding()?;
                    // A same-line `{` introduces computed-property accessors or observers.
                    if self.peek().kind == TokenKind::LBrace && !self.peek().leading_newline {
                        self.parse_accessor_block(decl)?;
                    }
                    return Ok(decl);
                }
                "func" => return self.parse_func(),
                "struct" => return self.parse_nominal(NodeKind::StructDecl),
                "enum" => return self.parse_nominal(NodeKind::EnumDecl),
                "class" => return self.parse_nominal(NodeKind::ClassDecl),
                "protocol" => return self.parse_nominal(NodeKind::ProtocolDecl),
                "extension" => return self.parse_extension(),
                "associatedtype" => return self.parse_associatedtype(),
                "typealias" => return self.parse_typealias(),
                "deinit" => return self.parse_deinit(),
                "init" => return self.parse_init(),
                "subscript" => return self.parse_subscript(),
                "return" => return self.parse_return(),
                "if" => return self.parse_if(),
                "guard" => return self.parse_guard(),
                "while" => return self.parse_while(None),
                "repeat" => return self.parse_repeat(None),
                "for" => return self.parse_for(None),
                "switch" => return self.parse_switch(None),
                "break" => return self.parse_jump(NodeKind::BreakStmt),
                "continue" => return self.parse_jump(NodeKind::ContinueStmt),
                "fallthrough" => {
                    let kw = self.bump();
                    return Ok(self
                        .ast
                        .add(NodeKind::FallthroughStmt, None, kw.line, kw.col));
                }
                _ => {}
            }
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

    fn parse_labeled(&mut self, label: &str) -> Result<NodeId, ParseError> {
        match self.peek().text {
            "while" => self.parse_while(Some(label)),
            "repeat" => self.parse_repeat(Some(label)),
            "for" => self.parse_for(Some(label)),
            "switch" => self.parse_switch(Some(label)),
            _ => self.error("expected a loop or switch after a statement label"),
        }
    }

    /// A braced `{ statements }` block.
    fn parse_block(&mut self) -> Result<NodeId, ParseError> {
        let open = self.expect(TokenKind::LBrace)?;
        let block = self.ast.add(NodeKind::Block, None, open.line, open.col);
        while self.peek().kind != TokenKind::RBrace && !self.at_eof() {
            let stmt = self.parse_statement()?;
            self.ast.append_child(block, stmt);
        }
        self.expect(TokenKind::RBrace)?;
        Ok(block)
    }

    /// `func name(params) [-> Ret] { body }`. Children: params, optional return
    /// `TypeRef`, then the body `Block`.
    fn parse_func(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        // The name is an identifier or an operator (`func == `, `func + `).
        let name_tok = self.peek();
        let name = match name_tok.kind {
            TokenKind::Identifier | TokenKind::Oper => self.bump().text,
            other => return self.error(format!("expected a function name, found {other:?}")),
        };
        let func = self
            .ast
            .add(NodeKind::FuncDecl, Some(name), kw.line, kw.col);
        if self.at_oper("<") {
            self.parse_generic_clause(func);
        }
        self.expect(TokenKind::LParen)?;
        if self.peek().kind != TokenKind::RParen {
            loop {
                let p = self.parse_param()?;
                self.ast.append_child(func, p);
                if self.peek().kind == TokenKind::Comma {
                    self.bump();
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen)?;
        self.skip_effects();
        if self.at_oper("->") {
            self.bump();
            let ret = self.parse_type()?;
            self.ast.append_child(func, ret);
        }
        if self.at_keyword("where") {
            self.skip_where_clause();
        }
        // Protocol method requirements have no body.
        if self.peek().kind == TokenKind::LBrace {
            let body = self.parse_block()?;
            self.ast.append_child(func, body);
        }
        Ok(func)
    }

    /// Consume effect markers (`async`/`throws`/`rethrows`, incl. typed throws).
    fn skip_effects(&mut self) {
        if self.at_keyword("async") {
            self.bump();
        }
        if self.at_keyword("throws") || self.at_keyword("rethrows") {
            self.bump();
            // Typed throws `throws(E)`.
            if self.peek().kind == TokenKind::LParen {
                while !matches!(self.peek().kind, TokenKind::RParen | TokenKind::Eof) {
                    self.bump();
                }
                self.bump();
            }
        }
    }

    /// Consume a generic parameter clause `<T, U: P>` (depth-balanced, robust
    /// to `>>`), recording its source text on a [`NodeKind::GenericParam`].
    fn parse_generic_clause(&mut self, parent: NodeId) {
        let start = self.peek();
        let text = self.consume_angle_group();
        let gp = self
            .ast
            .add(NodeKind::GenericParam, Some(&text), start.line, start.col);
        self.ast.append_child(parent, gp);
    }

    /// Consume a balanced `< ... >` group, returning its concatenated text.
    /// Handles nested groups and merged closers like `>>`.
    fn consume_angle_group(&mut self) -> String {
        let mut out = String::new();
        let mut depth = 0i32;
        loop {
            let t = self.peek();
            match t.kind {
                TokenKind::Eof => break,
                TokenKind::Oper => {
                    for ch in t.text.chars() {
                        if ch == '<' {
                            depth += 1;
                        } else if ch == '>' {
                            depth -= 1;
                        }
                    }
                    out.push_str(t.text);
                    self.bump();
                    if depth <= 0 {
                        break;
                    }
                }
                _ => {
                    out.push_str(t.text);
                    self.bump();
                }
            }
        }
        out
    }

    /// Consume a `where` constraint clause up to the body `{` or end of line.
    fn skip_where_clause(&mut self) {
        self.bump(); // `where`
        while !matches!(self.peek().kind, TokenKind::LBrace | TokenKind::Eof)
            && !self.peek().leading_newline
        {
            self.bump();
        }
    }

    /// `[externalLabel] name: [inout] Type [...] [= default]`.
    fn parse_param(&mut self) -> Result<NodeId, ParseError> {
        let first = self.peek();
        if first.kind != TokenKind::Identifier {
            return self.error(format!("expected a parameter name, found {:?}", first.kind));
        }
        self.bump();
        // A second identifier before the colon means `first` was the label.
        let name = if self.peek().kind == TokenKind::Identifier {
            self.bump().text
        } else {
            first.text
        };
        let param = self
            .ast
            .add(NodeKind::Param, Some(name), first.line, first.col);
        self.expect(TokenKind::Colon)?;
        if self.at_keyword("inout") {
            self.bump();
        }
        let ty = self.parse_type()?;
        self.ast.append_child(param, ty);
        if self.at_oper("...") {
            self.bump(); // variadic marker
        }
        if self.at_oper("=") {
            self.bump();
            let default = self.parse_expr(0)?;
            self.ast.append_child(param, default);
        }
        Ok(param)
    }

    /// `return [expr]`. The value, when present, is on the same line.
    fn parse_return(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::ReturnStmt, None, kw.line, kw.col);
        let next = self.peek();
        let ends = matches!(next.kind, TokenKind::RBrace | TokenKind::Eof);
        if !ends && !next.leading_newline {
            let expr = self.parse_expr(0)?;
            self.ast.append_child(node, expr);
        }
        Ok(node)
    }

    /// `if cond { } [else (if ... | { })]`. Usable as a statement or expression.
    fn parse_if(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::IfStmt, None, kw.line, kw.col);
        self.parse_conditions(node)?;
        let then = self.parse_block()?;
        self.ast.append_child(node, then);
        if self.at_keyword("else") {
            self.bump();
            let else_branch = if self.at_keyword("if") {
                self.parse_if()?
            } else {
                self.parse_block()?
            };
            self.ast.append_child(node, else_branch);
        }
        Ok(node)
    }

    fn parse_guard(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::GuardStmt, None, kw.line, kw.col);
        self.parse_conditions(node)?;
        if !self.at_keyword("else") {
            return self.error("expected 'else' after the guard condition");
        }
        self.bump();
        let body = self.parse_block()?;
        self.ast.append_child(node, body);
        Ok(node)
    }

    fn parse_while(&mut self, label: Option<&str>) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::WhileStmt, label, kw.line, kw.col);
        self.parse_conditions(node)?;
        let body = self.parse_block()?;
        self.ast.append_child(node, body);
        Ok(node)
    }

    fn parse_repeat(&mut self, label: Option<&str>) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::RepeatStmt, label, kw.line, kw.col);
        let body = self.parse_block()?;
        self.ast.append_child(node, body);
        if !self.at_keyword("while") {
            return self.error("expected 'while' after a repeat body");
        }
        self.bump();
        let cond = self.parse_expr(0)?;
        self.ast.append_child(node, cond);
        Ok(node)
    }

    /// `for pattern in iterable [where cond] { body }`. Children: pattern,
    /// iterable, optional where-expr, then the body `Block` (always last).
    fn parse_for(&mut self, label: Option<&str>) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::ForStmt, label, kw.line, kw.col);
        let pattern = self.parse_pattern()?;
        self.ast.append_child(node, pattern);
        if !self.at_keyword("in") {
            return self.error("expected 'in' in a for-loop");
        }
        self.bump();
        let iterable = self.parse_expr_no_trailing(0)?;
        self.ast.append_child(node, iterable);
        if self.at_keyword("where") {
            self.bump();
            let cond = self.parse_expr_no_trailing(0)?;
            self.ast.append_child(node, cond);
        }
        let body = self.parse_block()?;
        self.ast.append_child(node, body);
        Ok(node)
    }

    /// `switch subject { case ... / default ... }`.
    fn parse_switch(&mut self, label: Option<&str>) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::SwitchStmt, label, kw.line, kw.col);
        let subject = self.parse_expr_no_trailing(0)?;
        self.ast.append_child(node, subject);
        self.expect(TokenKind::LBrace)?;
        while self.at_keyword("case") || self.at_keyword("default") {
            let clause = self.parse_case_clause()?;
            self.ast.append_child(node, clause);
        }
        self.expect(TokenKind::RBrace)?;
        Ok(node)
    }

    /// One `case items [where cond]:` or `default:` clause. Children: the case
    /// items, an optional where-expr, then a `Block` of the clause body (last).
    fn parse_case_clause(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let is_default = kw.text == "default";
        let label = if is_default { Some("default") } else { None };
        let clause = self.ast.add(NodeKind::CaseClause, label, kw.line, kw.col);
        if !is_default {
            loop {
                let item = self.parse_case_item()?;
                self.ast.append_child(clause, item);
                if self.peek().kind == TokenKind::Comma {
                    self.bump();
                    continue;
                }
                break;
            }
            if self.at_keyword("where") {
                self.bump();
                let cond = self.parse_expr_no_trailing(0)?;
                self.ast.append_child(clause, cond);
            }
        }
        self.expect(TokenKind::Colon)?;
        let body = self.ast.add(NodeKind::Block, None, kw.line, kw.col);
        while !self.at_keyword("case")
            && !self.at_keyword("default")
            && self.peek().kind != TokenKind::RBrace
            && !self.at_eof()
        {
            let stmt = self.parse_statement()?;
            self.ast.append_child(body, stmt);
        }
        self.ast.append_child(clause, body);
        Ok(clause)
    }

    /// A `case` item: a `let`/`var` binding pattern or a value-pattern expression.
    fn parse_case_item(&mut self) -> Result<NodeId, ParseError> {
        if self.at_keyword("let") || self.at_keyword("var") {
            self.bump();
            return self.parse_pattern();
        }
        self.parse_expr(0)
    }

    /// `break`/`continue` with an optional same-line target label.
    fn parse_jump(&mut self, kind: NodeKind) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let label = if self.peek().kind == TokenKind::Identifier && !self.peek().leading_newline {
            Some(self.bump().text)
        } else {
            None
        };
        Ok(self.ast.add(kind, label, kw.line, kw.col))
    }

    /// Consume any run of leading declaration modifiers that precede a
    /// declaration keyword (so they are accepted but not yet modelled).
    fn skip_modifiers(&mut self) {
        let mut i = self.pos;
        loop {
            let t = self.tokens[i];
            if t.kind == TokenKind::Attribute {
                i += 1;
                if self.tokens[i].kind == TokenKind::LParen {
                    i = self.scan_balanced_parens(i);
                }
            } else if is_modifier_word(t.text) {
                i += 1;
                // Argumented modifier such as `private(set)`.
                if self.tokens[i].kind == TokenKind::LParen {
                    i = self.scan_balanced_parens(i);
                }
            } else {
                break;
            }
        }
        // Only consume the run when it actually precedes a declaration.
        if i > self.pos
            && (is_decl_keyword(self.tokens[i].text) || self.tokens[i].kind == TokenKind::Attribute)
        {
            while self.pos < i {
                self.bump();
            }
        }
    }

    /// Given an index at a `(`, return the index just past the matching `)`.
    fn scan_balanced_parens(&self, mut i: usize) -> usize {
        let mut depth = 0;
        loop {
            match self.tokens[i].kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                TokenKind::Eof => return i,
                _ => {}
            }
            i += 1;
        }
    }

    /// One or more comma-separated conditions for `if`/`guard`/`while`. A
    /// condition is either an optional binding (`let x = e`) or an expression.
    /// `deinit { }`.
    fn parse_deinit(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::DeinitDecl, None, kw.line, kw.col);
        let body = self.parse_block()?;
        self.ast.append_child(node, body);
        Ok(node)
    }

    /// `do { } [catch [pattern] [where ...] { }]...`.
    fn parse_do(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::DoStmt, None, kw.line, kw.col);
        let body = self.parse_block()?;
        self.ast.append_child(node, body);
        while self.at_keyword("catch") {
            let ckw = self.bump();
            let clause = self.ast.add(NodeKind::CatchClause, None, ckw.line, ckw.col);
            // Optional catch pattern(s) before the `{`.
            if self.peek().kind != TokenKind::LBrace {
                let saved = self.no_trailing_closure;
                self.no_trailing_closure = true;
                loop {
                    let pat = if self.at_keyword("let") || self.at_keyword("var") {
                        self.parse_binding()?
                    } else {
                        self.parse_expr(0)?
                    };
                    self.ast.append_child(clause, pat);
                    if self.peek().kind == TokenKind::Comma {
                        self.bump();
                        continue;
                    }
                    break;
                }
                if self.at_keyword("where") {
                    self.skip_where_clause();
                }
                self.no_trailing_closure = saved;
            }
            let cbody = self.parse_block()?;
            self.ast.append_child(clause, cbody);
            self.ast.append_child(node, clause);
        }
        Ok(node)
    }

    /// `throw expr`.
    fn parse_throw(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::ThrowStmt, None, kw.line, kw.col);
        let expr = self.parse_expr(0)?;
        self.ast.append_child(node, expr);
        Ok(node)
    }

    /// `defer { }`.
    fn parse_defer(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::DeferStmt, None, kw.line, kw.col);
        let body = self.parse_block()?;
        self.ast.append_child(node, body);
        Ok(node)
    }

    /// A compiler directive used as a statement: `#warning(...)`, `#error(...)`,
    /// or a `#if` / `#elseif` / `#else` / `#endif` conditional-compilation
    /// block. The active branch's statements are spliced into a directive node.
    fn parse_directive_stmt(&mut self) -> Result<NodeId, ParseError> {
        let d = self.peek();
        if d.text == "#if" {
            return self.parse_conditional_compilation();
        }
        let dir = self.bump();
        let node = self.ast.add(
            NodeKind::CompilerDirective,
            Some(dir.text),
            dir.line,
            dir.col,
        );
        if self.peek().kind == TokenKind::LParen {
            self.bump();
            if self.peek().kind != TokenKind::RParen {
                let arg = self.parse_expr(0)?;
                self.ast.append_child(node, arg);
            }
            self.expect(TokenKind::RParen)?;
        }
        Ok(node)
    }

    /// `#if cond ... [#elseif cond ...] [#else ...] #endif`. Only the first
    /// active branch's statements are parsed; inactive branches are skipped.
    fn parse_conditional_compilation(&mut self) -> Result<NodeId, ParseError> {
        let start = self.bump(); // `#if`
        let node = self.ast.add(
            NodeKind::CompilerDirective,
            Some("#if"),
            start.line,
            start.col,
        );
        let mut taken = false;
        // Evaluate the `#if` condition (rest of the line).
        let mut active = self.eval_directive_condition();
        loop {
            if active && !taken {
                taken = true;
                while !self.at_directive_boundary() {
                    let stmt = self.parse_statement()?;
                    self.ast.append_child(node, stmt);
                }
            } else {
                self.skip_to_directive_boundary();
            }
            match self.peek().text {
                "#elseif" => {
                    self.bump();
                    active = self.eval_directive_condition();
                }
                "#else" => {
                    self.bump();
                    active = true;
                }
                "#endif" => {
                    self.bump();
                    return Ok(node);
                }
                _ => return Ok(node),
            }
        }
    }

    /// Whether the cursor is at a `#elseif`/`#else`/`#endif` boundary or EOF.
    fn at_directive_boundary(&self) -> bool {
        matches!(self.peek().text, "#elseif" | "#else" | "#endif") || self.at_eof()
    }

    /// Skip an inactive conditional-compilation branch, honouring nesting.
    fn skip_to_directive_boundary(&mut self) {
        let mut depth = 0;
        loop {
            let t = self.peek();
            if t.kind == TokenKind::Eof {
                return;
            }
            if t.text == "#if" {
                depth += 1;
            } else if depth == 0 && matches!(t.text, "#elseif" | "#else" | "#endif") {
                return;
            } else if t.text == "#endif" {
                depth -= 1;
            }
            self.bump();
        }
    }

    /// Evaluate a conditional-compilation condition. Unknown custom flags are
    /// false; `true` is true; `os()`/`canImport()`/`swift()`/`compiler()` are
    /// treated as available. Consumes the rest of the directive line.
    fn eval_directive_condition(&mut self) -> bool {
        let mut value = false;
        let mut negate = false;
        let mut saw_and_false = false;
        let mut any = false;
        while !self.peek().leading_newline && !self.at_eof() {
            let t = self.peek();
            match t.kind {
                TokenKind::Identifier => {
                    let avail = matches!(
                        t.text,
                        "os" | "canImport" | "arch" | "swift" | "compiler" | "targetEnvironment"
                    );
                    self.bump();
                    let mut flag = if avail {
                        true
                    } else {
                        t.text == "DEBUG" || t.text == "true"
                    };
                    if self.peek().kind == TokenKind::LParen {
                        let end = self.scan_balanced_parens(self.pos);
                        while self.pos < end {
                            self.bump();
                        }
                    }
                    if negate {
                        flag = !flag;
                        negate = false;
                    }
                    if saw_and_false {
                        // already short-circuited
                    } else if !any {
                        value = flag;
                    }
                    any = true;
                }
                TokenKind::Keyword if t.text == "true" => {
                    self.bump();
                    if !any {
                        value = !negate;
                    }
                    negate = false;
                    any = true;
                }
                TokenKind::Keyword if t.text == "false" => {
                    self.bump();
                    if !any {
                        value = negate;
                    }
                    negate = false;
                    any = true;
                }
                TokenKind::Oper if t.text == "!" => {
                    negate = !negate;
                    self.bump();
                }
                TokenKind::Oper if t.text == "&&" => {
                    self.bump();
                    if !value {
                        saw_and_false = true;
                    }
                    any = false;
                }
                TokenKind::Oper if t.text == "||" => {
                    self.bump();
                    if value {
                        // keep true
                        self.skip_rest_of_line();
                        return true;
                    }
                    any = false;
                }
                TokenKind::LParen | TokenKind::RParen => {
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
        value && !saw_and_false
    }

    fn skip_rest_of_line(&mut self) {
        while !self.peek().leading_newline && !self.at_eof() {
            self.bump();
        }
    }

    /// `[infix|prefix|postfix] operator <op> [: Group]` or
    /// `precedencegroup Name { ... }`.
    fn parse_operator_or_precedence(&mut self) -> Result<NodeId, ParseError> {
        if self.peek().text == "precedencegroup" {
            let kw = self.bump();
            let name = self.expect(TokenKind::Identifier)?;
            let node = self.ast.add(
                NodeKind::PrecedenceGroupDecl,
                Some(name.text),
                kw.line,
                kw.col,
            );
            self.expect(TokenKind::LBrace)?;
            while self.peek().kind != TokenKind::RBrace && !self.at_eof() {
                self.bump();
            }
            self.expect(TokenKind::RBrace)?;
            return Ok(node);
        }
        // Optional fixity word before `operator`.
        if matches!(self.peek().text, "infix" | "prefix" | "postfix") {
            self.bump();
        }
        self.bump(); // `operator`
                     // The operator name may span several adjacent operator tokens (`<>`).
        let first = self.peek();
        let mut name = String::new();
        while self.peek().kind == TokenKind::Oper && !self.peek().leading_newline {
            name.push_str(self.bump().text);
        }
        if name.is_empty() {
            // Fall back to a single token (e.g. an identifier-like operator).
            name.push_str(self.bump().text);
        }
        let node = self
            .ast
            .add(NodeKind::OperatorDecl, Some(&name), first.line, first.col);
        if self.peek().kind == TokenKind::Colon {
            self.bump();
            let group = self.expect(TokenKind::Identifier)?;
            let g = self
                .ast
                .add(NodeKind::IdentExpr, Some(group.text), group.line, group.col);
            self.ast.append_child(node, g);
        }
        Ok(node)
    }

    /// A closure `{ [captures] params in body }`. Capture lists and signatures
    /// are accepted (and skipped); the body statements become the children.
    fn parse_closure(&mut self) -> Result<NodeId, ParseError> {
        let open = self.bump(); // '{'
        let node = self
            .ast
            .add(NodeKind::ClosureExpr, None, open.line, open.col);
        if self.peek().kind == TokenKind::LBracket {
            self.skip_bracketed(); // capture list `[weak self]`
        }
        self.try_closure_signature();
        let saved = self.no_trailing_closure;
        self.no_trailing_closure = false;
        while self.peek().kind != TokenKind::RBrace && !self.at_eof() {
            let stmt = self.parse_statement()?;
            self.ast.append_child(node, stmt);
        }
        self.no_trailing_closure = saved;
        self.expect(TokenKind::RBrace)?;
        Ok(node)
    }

    /// Tentatively consume a closure signature ending in `in`; restore the
    /// cursor and return `false` if the upcoming tokens are not a signature.
    fn try_closure_signature(&mut self) -> bool {
        let save = self.pos;
        loop {
            let t = self.peek();
            if t.kind == TokenKind::Keyword && t.text == "in" {
                self.bump();
                return true;
            }
            let signature_like = matches!(
                t.kind,
                TokenKind::Identifier
                    | TokenKind::Comma
                    | TokenKind::Colon
                    | TokenKind::LParen
                    | TokenKind::RParen
            ) || (t.kind == TokenKind::Oper && t.text == "->")
                || (t.kind == TokenKind::Keyword && t.text == "inout");
            if signature_like {
                self.bump();
            } else {
                self.pos = save;
                return false;
            }
        }
    }

    /// Consume a balanced `[ ... ]` group (e.g. a closure capture list).
    fn skip_bracketed(&mut self) {
        let mut depth = 0;
        loop {
            match self.peek().kind {
                TokenKind::LBracket => depth += 1,
                TokenKind::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                TokenKind::Eof => return,
                _ => {}
            }
            self.bump();
        }
    }

    fn parse_conditions(&mut self, parent: NodeId) -> Result<(), ParseError> {
        let saved = self.no_trailing_closure;
        self.no_trailing_closure = true;
        let result = self.parse_conditions_inner(parent);
        self.no_trailing_closure = saved;
        result
    }

    fn parse_conditions_inner(&mut self, parent: NodeId) -> Result<(), ParseError> {
        loop {
            let cond = if self.at_keyword("let") || self.at_keyword("var") {
                self.parse_binding()?
            } else {
                self.parse_expr(0)?
            };
            self.ast.append_child(parent, cond);
            if self.peek().kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            break;
        }
        Ok(())
    }

    /// `struct`/`enum Name [: Conformances] { members }`.
    fn parse_nominal(&mut self, kind: NodeKind) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let name = self.expect(TokenKind::Identifier)?;
        let node = self.ast.add(kind, Some(name.text), kw.line, kw.col);
        if self.at_oper("<") {
            self.parse_generic_clause(node);
        }
        // Inheritance / conformance / raw-value clause `: A, B`.
        if self.peek().kind == TokenKind::Colon {
            self.bump();
            loop {
                let ty = self.parse_type()?;
                self.ast.append_child(node, ty);
                if self.peek().kind == TokenKind::Comma {
                    self.bump();
                    continue;
                }
                break;
            }
        }
        if self.at_keyword("where") {
            self.skip_where_clause();
        }
        self.expect(TokenKind::LBrace)?;
        while self.peek().kind != TokenKind::RBrace && !self.at_eof() {
            if self.at_keyword("case") {
                self.parse_enum_cases(node)?;
            } else {
                let member = self.parse_statement()?;
                self.ast.append_child(node, member);
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(node)
    }

    /// One `case a, b(Int), c = 1` line, appending an [`NodeKind::EnumCaseDecl`]
    /// per element to `parent`.
    fn parse_enum_cases(&mut self, parent: NodeId) -> Result<(), ParseError> {
        self.bump(); // `case`
        loop {
            let name = self.expect(TokenKind::Identifier)?;
            let case = self
                .ast
                .add(NodeKind::EnumCaseDecl, Some(name.text), name.line, name.col);
            if self.peek().kind == TokenKind::LParen {
                self.bump();
                if self.peek().kind != TokenKind::RParen {
                    loop {
                        let ty = self.parse_type()?;
                        self.ast.append_child(case, ty);
                        if self.peek().kind == TokenKind::Comma {
                            self.bump();
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
            } else if self.at_oper("=") {
                self.bump();
                let raw = self.parse_expr(0)?;
                self.ast.append_child(case, raw);
            }
            self.ast.append_child(parent, case);
            if self.peek().kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            break;
        }
        Ok(())
    }

    /// `init[?]([params]) [throws] { body }`.
    fn parse_init(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::InitDecl, None, kw.line, kw.col);
        if self.peek().kind == TokenKind::Question || self.at_oper("!") {
            self.bump(); // failable `init?` / `init!`
        }
        self.parse_param_list(node)?;
        self.skip_effects();
        // Protocol initializer requirements have no body.
        if self.peek().kind == TokenKind::LBrace {
            let body = self.parse_block()?;
            self.ast.append_child(node, body);
        }
        Ok(node)
    }

    /// `extension Type[: P, Q] [where ...] { members }`.
    fn parse_extension(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let name = self.parse_type_text()?;
        let node = self
            .ast
            .add(NodeKind::ExtensionDecl, Some(&name), kw.line, kw.col);
        if self.peek().kind == TokenKind::Colon {
            self.bump();
            loop {
                let ty = self.parse_type()?;
                self.ast.append_child(node, ty);
                if self.peek().kind == TokenKind::Comma {
                    self.bump();
                    continue;
                }
                break;
            }
        }
        if self.at_keyword("where") {
            self.skip_where_clause();
        }
        self.expect(TokenKind::LBrace)?;
        while self.peek().kind != TokenKind::RBrace && !self.at_eof() {
            if self.at_keyword("case") {
                self.parse_enum_cases(node)?;
            } else {
                let member = self.parse_statement()?;
                self.ast.append_child(node, member);
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(node)
    }

    /// `associatedtype Name[: Constraint] [= Default] [where ...]`.
    fn parse_associatedtype(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let name = self.expect(TokenKind::Identifier)?;
        let node = self.ast.add(
            NodeKind::AssociatedTypeDecl,
            Some(name.text),
            kw.line,
            kw.col,
        );
        if self.peek().kind == TokenKind::Colon {
            self.bump();
            let ty = self.parse_type()?;
            self.ast.append_child(node, ty);
        }
        if self.at_oper("=") {
            self.bump();
            let default = self.parse_type()?;
            self.ast.append_child(node, default);
        }
        if self.at_keyword("where") {
            self.skip_where_clause();
        }
        Ok(node)
    }

    /// `typealias Name[<...>] = Type`.
    fn parse_typealias(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let name = self.expect(TokenKind::Identifier)?;
        let node = self
            .ast
            .add(NodeKind::TypeAliasDecl, Some(name.text), kw.line, kw.col);
        if self.at_oper("<") {
            self.parse_generic_clause(node);
        }
        if self.at_oper("=") {
            self.bump();
            let ty = self.parse_type()?;
            self.ast.append_child(node, ty);
        }
        Ok(node)
    }

    /// `subscript([params]) -> Type { accessors }`.
    fn parse_subscript(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::SubscriptDecl, None, kw.line, kw.col);
        self.parse_param_list(node)?;
        if self.at_oper("->") {
            self.bump();
            let ret = self.parse_type()?;
            self.ast.append_child(node, ret);
        }
        self.parse_accessor_block(node)?;
        Ok(node)
    }

    /// Parse a parenthesised, comma-separated parameter list into `parent`.
    fn parse_param_list(&mut self, parent: NodeId) -> Result<(), ParseError> {
        self.expect(TokenKind::LParen)?;
        if self.peek().kind != TokenKind::RParen {
            loop {
                let p = self.parse_param()?;
                self.ast.append_child(parent, p);
                if self.peek().kind == TokenKind::Comma {
                    self.bump();
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen)?;
        Ok(())
    }

    /// A `{ ... }` accessor block: explicit `get`/`set`/`willSet`/`didSet`
    /// accessors, or a read-only getter written as a bare statement block.
    fn parse_accessor_block(&mut self, parent: NodeId) -> Result<(), ParseError> {
        let open = self.expect(TokenKind::LBrace)?;
        if is_accessor_kw(self.peek().text) {
            while is_accessor_kw(self.peek().text) {
                let kw = self.bump();
                let acc = self
                    .ast
                    .add(NodeKind::Accessor, Some(kw.text), kw.line, kw.col);
                if self.peek().kind == TokenKind::LParen {
                    self.bump();
                    self.expect(TokenKind::Identifier)?;
                    self.expect(TokenKind::RParen)?;
                }
                self.skip_effects(); // `get throws`, `get async` in protocols
                                     // Protocol accessor requirements (`{ get set }`) have no body.
                if self.peek().kind == TokenKind::LBrace {
                    let body = self.parse_block()?;
                    self.ast.append_child(acc, body);
                }
                self.ast.append_child(parent, acc);
            }
            self.expect(TokenKind::RBrace)?;
        } else {
            // Read-only getter shorthand: the block's statements are the getter.
            let getter = self
                .ast
                .add(NodeKind::Accessor, Some("get"), open.line, open.col);
            let block = self.ast.add(NodeKind::Block, None, open.line, open.col);
            while self.peek().kind != TokenKind::RBrace && !self.at_eof() {
                let stmt = self.parse_statement()?;
                self.ast.append_child(block, stmt);
            }
            self.expect(TokenKind::RBrace)?;
            self.ast.append_child(getter, block);
            self.ast.append_child(parent, getter);
        }
        Ok(())
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
        // Existential / opaque prefixes: `any P`, `some P`, `inout T`.
        if (self.at_keyword("any") || self.at_keyword("some") || self.at_keyword("inout"))
            && self.tokens[self.pos + 1].kind != TokenKind::Eof
        {
            let kw = self.bump();
            let rest = self.parse_type_text()?;
            return Ok(format!("{} {}", kw.text, rest));
        }
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
                // Generic arguments `Array<Int>`, `Dictionary<K, V>`.
                if self.at_oper("<") {
                    name.push_str(&self.consume_angle_group());
                }
                while self.peek().kind == TokenKind::Dot
                    && self.tokens[self.pos + 1].kind == TokenKind::Identifier
                {
                    self.bump();
                    name.push('.');
                    name.push_str(self.bump().text);
                    if self.at_oper("<") {
                        name.push_str(&self.consume_angle_group());
                    }
                }
                name
            }
            other => return self.error(format!("expected a type, found {other:?}")),
        };
        // Optional / IUO suffixes.
        while self.peek().kind == TokenKind::Question || self.at_oper("!") {
            text.push_str(self.bump().text);
        }
        // Protocol composition `P & Q`.
        while self.at_oper("&") {
            self.bump();
            let rhs = self.parse_type_text()?;
            text = format!("{text} & {rhs}");
        }
        // Function type `(A) -> B`, with optional effects.
        self.skip_effects();
        if self.at_oper("->") {
            self.bump();
            let ret = self.parse_type_text()?;
            text = format!("{text} -> {ret}");
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
            // Type cast `expr is/as/as?/as! Type`, at casting precedence.
            if self.peek().kind == TokenKind::Keyword
                && matches!(self.peek().text, "is" | "as")
                && CAST_BP >= min_bp
            {
                let kw = self.bump();
                let mut op = kw.text.to_string();
                if kw.text == "as" && (self.peek().kind == TokenKind::Question || self.at_oper("!"))
                {
                    op.push_str(self.bump().text);
                }
                let ty = self.parse_type()?;
                let cast = self.ast.add(NodeKind::CastExpr, Some(&op), kw.line, kw.col);
                self.ast.append_child(cast, lhs);
                self.ast.append_child(cast, ty);
                lhs = cast;
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
        // `try` / `try?` / `try!` — an error-propagation prefix.
        if t.kind == TokenKind::Keyword && t.text == "try" {
            self.bump();
            let mut op = String::from("try");
            if self.peek().kind == TokenKind::Question || self.at_oper("!") {
                op.push_str(self.bump().text);
            }
            let operand = self.parse_prefix()?;
            let node = self.ast.add(NodeKind::TryExpr, Some(&op), t.line, t.col);
            self.ast.append_child(node, operand);
            return Ok(node);
        }
        // `await` simply propagates its operand at the frontend level.
        if t.kind == TokenKind::Identifier && t.text == "await" && !self.is_value_ident_context() {
            self.bump();
            return self.parse_prefix();
        }
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

    /// Heuristic: `await` is a contextual keyword only when followed by an
    /// expression start; treat a bare `await` used as an identifier normally.
    fn is_value_ident_context(&self) -> bool {
        matches!(
            self.tokens[self.pos + 1].kind,
            TokenKind::Oper
                | TokenKind::Dot
                | TokenKind::RParen
                | TokenKind::Comma
                | TokenKind::Colon
                | TokenKind::Eof
        )
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
            TokenKind::Keyword if t.text == "if" => return self.parse_if(),
            TokenKind::Keyword if t.text == "true" || t.text == "false" => {
                self.bump();
                self.ast
                    .add(NodeKind::BoolLiteral, Some(t.text), t.line, t.col)
            }
            TokenKind::Keyword if t.text == "nil" => {
                self.bump();
                self.ast.add(NodeKind::NilLiteral, None, t.line, t.col)
            }
            TokenKind::Keyword if t.text == "self" || t.text == "super" => {
                self.bump();
                self.ast
                    .add(NodeKind::IdentExpr, Some(t.text), t.line, t.col)
            }
            TokenKind::Identifier => {
                self.bump();
                self.ast
                    .add(NodeKind::IdentExpr, Some(t.text), t.line, t.col)
            }
            TokenKind::LBrace => return self.parse_closure(),
            TokenKind::LParen => return self.parse_paren_or_tuple(),
            // Directive expression `#file`, `#line`, `#function`, `#column`.
            TokenKind::Directive => {
                self.bump();
                self.ast
                    .add(NodeKind::CompilerDirective, Some(t.text), t.line, t.col)
            }
            // Implicit member expression `.case` (no base).
            TokenKind::Dot => {
                self.bump();
                let name = self.expect(TokenKind::Identifier)?;
                self.ast
                    .add(NodeKind::MemberExpr, Some(name.text), t.line, t.col)
            }
            other => return self.error(format!("expected an expression, found {other:?}")),
        };
        Ok(node)
    }

    /// `( expr )` collapses to the inner expr; `( a, b, ... )` is a tuple.
    /// Like [`Parser::parse_expr`] but suppressing trailing-closure parsing,
    /// used for control-flow iterables/subjects where a `{` starts the body.
    fn parse_expr_no_trailing(&mut self, min_bp: u8) -> Result<NodeId, ParseError> {
        let saved = self.no_trailing_closure;
        self.no_trailing_closure = true;
        let r = self.parse_expr(min_bp);
        self.no_trailing_closure = saved;
        r
    }

    fn parse_paren_or_tuple(&mut self) -> Result<NodeId, ParseError> {
        let open = self.bump(); // '('
                                // Inside parentheses, trailing closures are allowed again.
        let saved = self.no_trailing_closure;
        self.no_trailing_closure = false;
        let result = self.parse_paren_or_tuple_inner(open);
        self.no_trailing_closure = saved;
        result
    }

    fn parse_paren_or_tuple_inner(&mut self, open: Token<'a>) -> Result<NodeId, ParseError> {
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
                // Forced unwrap `expr!`.
                TokenKind::Oper if self.peek().text == "!" => {
                    let bang = self.bump();
                    let node = self
                        .ast
                        .add(NodeKind::PostfixExpr, Some("!"), bang.line, bang.col);
                    self.ast.append_child(node, expr);
                    expr = node;
                }
                // Optional chaining `expr?.member`: drop the `?`, let `.` handle it.
                TokenKind::Question if self.tokens[self.pos + 1].kind == TokenKind::Dot => {
                    self.bump();
                }
                // Trailing closure: `expr { ... }` (same line, outside a
                // control-flow head) attaches a closure argument to a call.
                TokenKind::LBrace if !self.no_trailing_closure && !self.peek().leading_newline => {
                    let closure = self.parse_closure()?;
                    if self.ast.node(expr).kind() == NodeKind::CallExpr {
                        self.ast.append_child(expr, closure);
                    } else {
                        let line = self.ast.node(closure).line();
                        let call = self.ast.add(NodeKind::CallExpr, None, line, 1);
                        self.ast.append_child(call, expr);
                        self.ast.append_child(call, closure);
                        expr = call;
                    }
                }
                TokenKind::LParen => {
                    let open = self.bump();
                    let call = self.ast.add(NodeKind::CallExpr, None, open.line, open.col);
                    self.ast.append_child(call, expr);
                    let saved = self.no_trailing_closure;
                    self.no_trailing_closure = false;
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
                    self.no_trailing_closure = saved;
                    self.expect(TokenKind::RParen)?;
                    expr = call;
                }
                TokenKind::Dot => {
                    let dot = self.bump();
                    let name = self.peek();
                    // Allow keyword members like `.init`, `.self`, `.Type`.
                    if !matches!(
                        name.kind,
                        TokenKind::Identifier | TokenKind::IntLiteral | TokenKind::Keyword
                    ) {
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

/// Precedence of `is`/`as` casts (Swift `CastingPrecedence`, /10).
const CAST_BP: u8 = 13;

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

/// Declaration modifiers consumed (and currently discarded) before a declaration.
fn is_modifier_word(w: &str) -> bool {
    matches!(
        w,
        "static"
            | "class"
            | "mutating"
            | "nonmutating"
            | "lazy"
            | "final"
            | "override"
            | "required"
            | "convenience"
            | "public"
            | "private"
            | "internal"
            | "fileprivate"
            | "open"
            | "package"
            | "weak"
            | "unowned"
            | "indirect"
            | "dynamic"
    )
}

/// Keywords that introduce a declaration a modifier run may precede.
fn is_decl_keyword(w: &str) -> bool {
    matches!(
        w,
        "struct"
            | "enum"
            | "class"
            | "protocol"
            | "extension"
            | "func"
            | "init"
            | "subscript"
            | "let"
            | "var"
            | "case"
            | "typealias"
            | "associatedtype"
            | "deinit"
    )
}

/// Accessor introducers inside a property/subscript body.
fn is_accessor_kw(w: &str) -> bool {
    matches!(w, "get" | "set" | "willSet" | "didSet")
}

/// Whether `kw` is a statement that may carry a leading `label:`.
fn is_labelable(kw: &str) -> bool {
    matches!(kw, "for" | "while" | "repeat" | "switch")
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

    #[test]
    fn function_declaration() {
        let ast = ast_of("func add(_ a: Int, b: Int = 0) -> Int { return a + b }");
        let func = first_stmt(&ast);
        assert_eq!(func.kind(), NodeKind::FuncDecl);
        assert_eq!(func.text(), Some("add"));
        let kinds: Vec<_> = func.children().map(|c| c.kind()).collect();
        assert_eq!(
            kinds,
            vec![
                NodeKind::Param,
                NodeKind::Param,
                NodeKind::TypeRef, // return type
                NodeKind::Block,
            ]
        );
        // The second param has a default-value child after its type.
        let p2 = func.children().nth(1).unwrap();
        assert_eq!(p2.text(), Some("b"));
        assert_eq!(
            p2.children().nth(1).unwrap().kind(),
            NodeKind::IntegerLiteral
        );
        // The body's lone statement is a return with a value.
        let body = func.children().nth(3).unwrap();
        let ret = body.children().next().unwrap();
        assert_eq!(ret.kind(), NodeKind::ReturnStmt);
        assert_eq!(ret.children().next().unwrap().kind(), NodeKind::BinaryExpr);
    }

    #[test]
    fn variadic_and_inout_params_parse() {
        let ast = ast_of("func f(_ xs: Int..., flag: inout Bool) { }");
        let func = first_stmt(&ast);
        let params: Vec<_> = func
            .children()
            .filter(|c| c.kind() == NodeKind::Param)
            .map(|c| c.text())
            .collect();
        assert_eq!(params, vec![Some("xs"), Some("flag")]);
    }

    #[test]
    fn if_else_if_chain() {
        let ast = ast_of("if a { } else if b { } else { }");
        let iff = first_stmt(&ast);
        assert_eq!(iff.kind(), NodeKind::IfStmt);
        // cond, then-block, else (nested if)
        let else_branch = iff.children().nth(2).unwrap();
        assert_eq!(else_branch.kind(), NodeKind::IfStmt);
    }

    #[test]
    fn if_as_expression_in_binding() {
        let ast = ast_of("let g = if c { 1 } else { 2 }");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::IfStmt);
    }

    #[test]
    fn guard_statement() {
        let ast = ast_of("guard x > 0 else { return }");
        let g = first_stmt(&ast);
        assert_eq!(g.kind(), NodeKind::GuardStmt);
        assert_eq!(g.children().next().unwrap().kind(), NodeKind::BinaryExpr);
        assert_eq!(g.children().nth(1).unwrap().kind(), NodeKind::Block);
    }

    #[test]
    fn while_and_repeat_loops() {
        assert_eq!(
            first_stmt(&ast_of("while n > 0 { }")).kind(),
            NodeKind::WhileStmt
        );
        let r = ast_of("repeat { } while n < 3");
        let node = first_stmt(&r);
        assert_eq!(node.kind(), NodeKind::RepeatStmt);
        // body block first, condition second
        assert_eq!(node.children().next().unwrap().kind(), NodeKind::Block);
        assert_eq!(node.children().nth(1).unwrap().kind(), NodeKind::BinaryExpr);
    }

    #[test]
    fn for_in_with_where_clause() {
        let ast = ast_of("for x in 0 ..< 5 where x > 1 { }");
        let f = first_stmt(&ast);
        assert_eq!(f.kind(), NodeKind::ForStmt);
        let kinds: Vec<_> = f.children().map(|c| c.kind()).collect();
        // pattern, iterable, where-expr, body block
        assert_eq!(
            kinds,
            vec![
                NodeKind::NamePattern,
                NodeKind::BinaryExpr,
                NodeKind::BinaryExpr,
                NodeKind::Block,
            ]
        );
    }

    #[test]
    fn switch_with_cases_and_default() {
        let src = "switch n {\n\
                   case 0: return\n\
                   case 1, 2: break\n\
                   case let x where x < 0: return\n\
                   default: break\n\
                   }";
        let ast = ast_of(src);
        let sw = first_stmt(&ast);
        assert_eq!(sw.kind(), NodeKind::SwitchStmt);
        let clauses: Vec<_> = sw
            .children()
            .filter(|c| c.kind() == NodeKind::CaseClause)
            .collect();
        assert_eq!(clauses.len(), 4);
        // The `case 1, 2:` clause has two value items before its body block.
        let multi = &clauses[1];
        let items = multi
            .children()
            .filter(|c| c.kind() != NodeKind::Block)
            .count();
        assert_eq!(items, 2);
        // The default clause is labelled.
        assert_eq!(clauses[3].text(), Some("default"));
    }

    #[test]
    fn labeled_loop_with_break_label() {
        let src = "outer: for i in xs {\n\
                   for j in ys {\n\
                   break outer\n\
                   }\n\
                   }";
        let ast = ast_of(src);
        let outer = first_stmt(&ast);
        assert_eq!(outer.kind(), NodeKind::ForStmt);
        assert_eq!(outer.text(), Some("outer"));
        // Drill to the inner break and check its captured label.
        let inner_body = outer.children().last().unwrap();
        let inner_for = inner_body.children().next().unwrap();
        let inner_block = inner_for.children().last().unwrap();
        let brk = inner_block.children().next().unwrap();
        assert_eq!(brk.kind(), NodeKind::BreakStmt);
        assert_eq!(brk.text(), Some("outer"));
    }

    #[test]
    fn bare_break_has_no_label() {
        let ast = ast_of("while c { break\nfoo() }");
        let body = first_stmt(&ast).children().nth(1).unwrap();
        let brk = body.children().next().unwrap();
        assert_eq!(brk.kind(), NodeKind::BreakStmt);
        assert_eq!(brk.text(), None); // `foo` on the next line is not its label
    }

    // --- Tier 2: value & nominal types ---

    #[test]
    fn struct_with_members() {
        let src = "struct Point {\n\
                   var x: Int\n\
                   var y: Int\n\
                   func sum() -> Int { return x + y }\n\
                   mutating func move() { x += 1 }\n\
                   }";
        let ast = ast_of(src);
        let s = first_stmt(&ast);
        assert_eq!(s.kind(), NodeKind::StructDecl);
        assert_eq!(s.text(), Some("Point"));
        let members: Vec<_> = s.children().map(|c| c.kind()).collect();
        assert_eq!(
            members,
            vec![
                NodeKind::VarDecl,
                NodeKind::VarDecl,
                NodeKind::FuncDecl,
                NodeKind::FuncDecl, // `mutating` modifier consumed
            ]
        );
    }

    #[test]
    fn enum_cases_simple_associated_and_raw() {
        let src = "enum Token: Int {\n\
                   case comma, dot\n\
                   case number(Int)\n\
                   case eof = 0\n\
                   }";
        let ast = ast_of(src);
        let e = first_stmt(&ast);
        assert_eq!(e.kind(), NodeKind::EnumDecl);
        // First child is the `: Int` raw-type conformance; then the cases.
        let cases: Vec<_> = e
            .children()
            .filter(|c| c.kind() == NodeKind::EnumCaseDecl)
            .map(|c| c.text())
            .collect();
        assert_eq!(
            cases,
            vec![Some("comma"), Some("dot"), Some("number"), Some("eof")]
        );
        // `number(Int)` carries a TypeRef child; `eof = 0` carries a literal.
        let number = e.children().find(|c| c.text() == Some("number")).unwrap();
        assert_eq!(number.children().next().unwrap().kind(), NodeKind::TypeRef);
    }

    #[test]
    fn computed_property_with_get_set() {
        let src = "struct T {\n\
                   var v: Int { get { return 1 } set { } }\n\
                   var ro: Int { 42 }\n\
                   }";
        let ast = ast_of(src);
        let s = first_stmt(&ast);
        let computed = s.children().next().unwrap();
        let accessors: Vec<_> = computed
            .children()
            .filter(|c| c.kind() == NodeKind::Accessor)
            .map(|c| c.text())
            .collect();
        assert_eq!(accessors, vec![Some("get"), Some("set")]);
        // The read-only property gets a synthesised `get` accessor.
        let ro = s.children().nth(1).unwrap();
        assert_eq!(ro.children().last().unwrap().kind(), NodeKind::Accessor);
    }

    #[test]
    fn property_observers() {
        let src = "struct T { var n: Int { willSet { } didSet { } } }";
        let ast = ast_of(src);
        let prop = first_stmt(&ast).children().next().unwrap();
        let accessors: Vec<_> = prop
            .children()
            .filter(|c| c.kind() == NodeKind::Accessor)
            .map(|c| c.text())
            .collect();
        assert_eq!(accessors, vec![Some("willSet"), Some("didSet")]);
    }

    #[test]
    fn init_and_subscript() {
        let src = "struct Grid {\n\
                   init?(n: Int) { }\n\
                   subscript(i: Int) -> Int { return i }\n\
                   }";
        let ast = ast_of(src);
        let s = first_stmt(&ast);
        let kinds: Vec<_> = s.children().map(|c| c.kind()).collect();
        assert_eq!(kinds, vec![NodeKind::InitDecl, NodeKind::SubscriptDecl]);
    }

    #[test]
    fn if_let_optional_binding() {
        let ast = ast_of("if let x = maybe { print(x) }");
        let iff = first_stmt(&ast);
        assert_eq!(iff.kind(), NodeKind::IfStmt);
        // The condition is a `let` binding.
        assert_eq!(iff.children().next().unwrap().kind(), NodeKind::LetDecl);
    }

    #[test]
    fn forced_unwrap_and_optional_chaining() {
        let ast = ast_of("let v = maybe!");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::PostfixExpr);
        assert_eq!(init.text(), Some("!"));
        // Optional chaining `a?.b` parses as member access on `a`.
        let ast = ast_of("let w = a?.b");
        let chain = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(chain.kind(), NodeKind::MemberExpr);
        assert_eq!(chain.text(), Some("b"));
    }

    #[test]
    fn implicit_member_expression() {
        let ast = ast_of("let d = .north");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::MemberExpr);
        assert_eq!(init.text(), Some("north"));
        assert_eq!(init.children().count(), 0); // implicit: no base
    }

    #[test]
    fn modifiers_before_declarations_are_accepted() {
        let ast = ast_of("static let shared = 1");
        assert_eq!(first_stmt(&ast).kind(), NodeKind::LetDecl);
    }

    // --- Tier 3: classes, ARC & closures ---

    #[test]
    fn class_with_superclass_and_members() {
        let src = "class Dog: Animal {\n\
                   override func sound() -> String { return \"woof\" }\n\
                   deinit { }\n\
                   }";
        let ast = ast_of(src);
        let c = first_stmt(&ast);
        assert_eq!(c.kind(), NodeKind::ClassDecl);
        assert_eq!(c.text(), Some("Dog"));
        // First child is the `: Animal` superclass type ref.
        assert_eq!(c.children().next().unwrap().kind(), NodeKind::TypeRef);
        let kinds: Vec<_> = c
            .children()
            .map(|m| m.kind())
            .filter(|k| *k != NodeKind::TypeRef)
            .collect();
        assert_eq!(kinds, vec![NodeKind::FuncDecl, NodeKind::DeinitDecl]);
    }

    #[test]
    fn cast_expressions() {
        let ast = ast_of("let a = shape as? Circle");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::CastExpr);
        assert_eq!(init.text(), Some("as?"));
        assert_eq!(
            first_stmt(&ast_of("let b = x is Int"))
                .children()
                .nth(1)
                .unwrap()
                .text(),
            Some("is")
        );
        assert_eq!(
            first_stmt(&ast_of("let c = x as! String"))
                .children()
                .nth(1)
                .unwrap()
                .text(),
            Some("as!")
        );
    }

    #[test]
    fn closure_shorthand_and_signature() {
        // Shorthand `$0`.
        let ast = ast_of("let f = { $0 * 2 }");
        assert_eq!(
            first_stmt(&ast).children().nth(1).unwrap().kind(),
            NodeKind::ClosureExpr
        );
        // Explicit `x in` signature.
        let ast = ast_of("let g = { x in x + 1 }");
        let clo = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(clo.kind(), NodeKind::ClosureExpr);
        // Body statement present after the signature.
        assert!(clo.children().count() >= 1);
    }

    #[test]
    fn trailing_closure_becomes_a_call() {
        let ast = ast_of("let doubled = numbers.map { $0 * 2 }");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::CallExpr);
        // The closure is the trailing argument.
        assert_eq!(
            init.children().last().unwrap().kind(),
            NodeKind::ClosureExpr
        );
    }

    #[test]
    fn capture_list_is_accepted() {
        let ast = ast_of("let h = { [weak self] in self }");
        assert_eq!(
            first_stmt(&ast).children().nth(1).unwrap().kind(),
            NodeKind::ClosureExpr
        );
    }

    #[test]
    fn for_body_brace_is_not_a_trailing_closure() {
        // Regression: the loop body `{ }` must not be parsed as a trailing
        // closure on the iterable `items`.
        let ast = ast_of("for x in items { print(x) }");
        let f = first_stmt(&ast);
        assert_eq!(f.kind(), NodeKind::ForStmt);
        assert_eq!(f.children().last().unwrap().kind(), NodeKind::Block);
    }

    #[test]
    fn super_and_self_are_expressions() {
        let ast = ast_of("let s = self");
        assert_eq!(
            first_stmt(&ast).children().nth(1).unwrap().text(),
            Some("self")
        );
        let ast = ast_of("func f() { super.init() }");
        let body = first_stmt(&ast).children().last().unwrap();
        let call = body.children().next().unwrap().children().next().unwrap();
        assert_eq!(call.kind(), NodeKind::CallExpr);
    }

    // --- Tier 4: protocols, generics & extensions ---

    #[test]
    fn protocol_with_requirements() {
        let src = "protocol Shape {\n\
                   var area: Double { get }\n\
                   func draw()\n\
                   associatedtype Point\n\
                   }";
        let ast = ast_of(src);
        let p = first_stmt(&ast);
        assert_eq!(p.kind(), NodeKind::ProtocolDecl);
        let kinds: Vec<_> = p.children().map(|m| m.kind()).collect();
        assert_eq!(
            kinds,
            vec![
                NodeKind::VarDecl,
                NodeKind::FuncDecl,
                NodeKind::AssociatedTypeDecl
            ]
        );
        // The method requirement has no body.
        let draw = p.children().nth(1).unwrap();
        assert!(draw.children().all(|c| c.kind() != NodeKind::Block));
    }

    #[test]
    fn generic_function_and_struct() {
        let ast = ast_of("func pick<T>(a: T, b: T) -> T { return a }");
        let f = first_stmt(&ast);
        assert_eq!(f.kind(), NodeKind::FuncDecl);
        assert_eq!(f.children().next().unwrap().kind(), NodeKind::GenericParam);
        assert_eq!(f.children().next().unwrap().text(), Some("<T>"));

        let ast = ast_of("struct Stack<Element> { var items: [Element] }");
        let s = first_stmt(&ast);
        assert_eq!(s.kind(), NodeKind::StructDecl);
        assert_eq!(s.children().next().unwrap().kind(), NodeKind::GenericParam);
    }

    #[test]
    fn constrained_generics_and_where_clause() {
        let ast = ast_of("func sorted<T: Comparable>(xs: T) -> T where T: Equatable { return xs }");
        let f = first_stmt(&ast);
        assert_eq!(f.kind(), NodeKind::FuncDecl);
        assert_eq!(f.children().next().unwrap().text(), Some("<T:Comparable>"));
        // Body still parses after the trailing `where`.
        assert_eq!(f.children().last().unwrap().kind(), NodeKind::Block);
    }

    #[test]
    fn extension_with_conformance() {
        let ast = ast_of("extension Int: Comparable { func double() -> Int { return self * 2 } }");
        let e = first_stmt(&ast);
        assert_eq!(e.kind(), NodeKind::ExtensionDecl);
        assert_eq!(e.text(), Some("Int"));
        let kinds: Vec<_> = e.children().map(|c| c.kind()).collect();
        assert_eq!(kinds, vec![NodeKind::TypeRef, NodeKind::FuncDecl]);
    }

    #[test]
    fn typealias_declaration() {
        let ast = ast_of("typealias Pair = (Int, Int)");
        let t = first_stmt(&ast);
        assert_eq!(t.kind(), NodeKind::TypeAliasDecl);
        assert_eq!(t.text(), Some("Pair"));
        assert_eq!(t.children().next().unwrap().text(), Some("(Int, Int)"));
    }

    #[test]
    fn existential_composition_and_function_types() {
        let ty = |src| {
            let ast = ast_of(src);
            let text = first_stmt(&ast)
                .children()
                .nth(1)
                .unwrap()
                .text()
                .unwrap()
                .to_string();
            text
        };
        assert_eq!(ty("let a: any Shape = s"), "any Shape");
        assert_eq!(ty("let b: Codable & Equatable = z"), "Codable & Equatable");
        assert_eq!(ty("let c: (Int) -> Int = g"), "(Int) -> Int");
        assert_eq!(ty("let d: Array<Int> = e"), "Array<Int>");
    }

    #[test]
    fn operator_requirement_in_type() {
        let ast = ast_of("struct V { static func == (a: V, b: V) -> Bool { return true } }");
        let func = first_stmt(&ast).children().next().unwrap();
        assert_eq!(func.kind(), NodeKind::FuncDecl);
        assert_eq!(func.text(), Some("=="));
    }

    // --- Tier 5/6/9: errors, attributes, operators & directives ---

    #[test]
    fn do_catch_with_pattern() {
        let src = "do {\n\
                   try risky()\n\
                   } catch let error {\n\
                   recover()\n\
                   } catch {\n\
                   fail()\n\
                   }";
        let ast = ast_of(src);
        let d = first_stmt(&ast);
        assert_eq!(d.kind(), NodeKind::DoStmt);
        let kinds: Vec<_> = d.children().map(|c| c.kind()).collect();
        assert_eq!(
            kinds,
            vec![
                NodeKind::Block,
                NodeKind::CatchClause,
                NodeKind::CatchClause
            ]
        );
    }

    #[test]
    fn throw_try_and_defer() {
        let ast = ast_of("func f() { defer { cleanup() }\n throw Err.bad }");
        let body = first_stmt(&ast).children().last().unwrap();
        let kinds: Vec<_> = body.children().map(|c| c.kind()).collect();
        assert_eq!(kinds, vec![NodeKind::DeferStmt, NodeKind::ThrowStmt]);

        let ast = ast_of("let v = try? parse()");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::TryExpr);
        assert_eq!(init.text(), Some("try?"));
    }

    #[test]
    fn attributes_and_private_set_before_declarations() {
        let ast = ast_of("@main struct App { }");
        assert_eq!(first_stmt(&ast).kind(), NodeKind::StructDecl);
        let ast = ast_of("@available(macOS 10.15, *) func feature() { }");
        assert_eq!(first_stmt(&ast).kind(), NodeKind::FuncDecl);
        let ast = ast_of("private(set) var count = 0");
        assert_eq!(first_stmt(&ast).kind(), NodeKind::VarDecl);
    }

    #[test]
    fn custom_operator_and_precedencegroup() {
        let ast = ast_of("infix operator <> : AdditionPrecedence");
        let op = first_stmt(&ast);
        assert_eq!(op.kind(), NodeKind::OperatorDecl);
        assert_eq!(op.text(), Some("<>"));

        let ast = ast_of("precedencegroup MyPrecedence { higherThan: AdditionPrecedence }");
        let pg = first_stmt(&ast);
        assert_eq!(pg.kind(), NodeKind::PrecedenceGroupDecl);
        assert_eq!(pg.text(), Some("MyPrecedence"));
    }

    #[test]
    fn pound_directives() {
        // `#warning` as a statement.
        let ast = ast_of("#warning(\"todo\")");
        assert_eq!(first_stmt(&ast).kind(), NodeKind::CompilerDirective);
        assert_eq!(first_stmt(&ast).text(), Some("#warning"));
        // `#file` as an expression.
        let ast = ast_of("let here = #file");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::CompilerDirective);
        assert_eq!(init.text(), Some("#file"));
    }

    #[test]
    fn conditional_compilation_selects_active_branch() {
        // `DEBUG` is treated as defined -> first branch active.
        let ast = ast_of("#if DEBUG\n let mode = 1\n #else\n let mode = 2\n #endif");
        let dir = first_stmt(&ast);
        assert_eq!(dir.kind(), NodeKind::CompilerDirective);
        let active: Vec<_> = dir.children().collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].kind(), NodeKind::LetDecl);
        // The active binding initializes to 1, not the #else's 2.
        assert_eq!(active[0].children().nth(1).unwrap().text(), Some("1"));
    }

    #[test]
    fn conditional_compilation_false_flag_takes_else() {
        let ast = ast_of("#if UNDEFINED_FLAG\n let x = 1\n #else\n let x = 2\n #endif");
        let dir = first_stmt(&ast);
        let active: Vec<_> = dir.children().collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].children().nth(1).unwrap().text(), Some("2"));
    }
}
