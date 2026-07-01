//! A recursive-descent + Pratt parser for the tswift frontend.
//!
//! [`parse`] turns a [`tswift_lexer`] token stream into a [`tswift_ast::Ast`].
//! Statements are parsed top-down; expressions use precedence climbing (Pratt)
//! with Swift's operator precedence so `1 + 2 * 3` and `a || b && c` nest
//! correctly. Coverage today is **Tier 0 + Tier 1a**: `let`/`var` bindings
//! (with patterns, type annotations, initializers), assignment statements,
//! tuples, member/tuple-index access, calls, unary and ternary expressions, and
//! the full binary operator set over every literal form.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use tswift_ast::{Ast, NodeId, NodeKind};
use tswift_lexer::{tokenize, Token, TokenKind};

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
        op_precedence: HashMap::new(),
        pending_siblings: Vec::new(),
    };
    p.collect_operator_precedence();
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
    /// User-declared infix operators → `(precedence, right_associative)`, built
    /// from `operator`/`precedencegroup` declarations in a pre-pass so the Pratt
    /// parser nests custom operators by their real precedence group.
    op_precedence: HashMap<String, (u8, bool)>,
    /// Extra declarations produced by desugaring a single source statement into
    /// several (a multi-name binding `var a, b: T`). Drained by
    /// [`Parser::append_statement`] into the same parent as the primary node.
    pending_siblings: Vec<NodeId>,
}

/// A `precedencegroup` declaration as scanned before parsing: its relation to
/// another group and its associativity. Resolved to a numeric precedence by
/// [`resolve_group_precedence`].
struct RawPrecedenceGroup {
    higher_than: Option<String>,
    lower_than: Option<String>,
    right_associative: bool,
}

/// Declaration modifiers and attributes collected ahead of a declaration by
/// [`Parser::collect_decl_meta`] and applied by [`Parser::attach_decl_meta`].
#[derive(Default)]
struct DeclMeta {
    /// Modifier keywords in source order (`static`, `mutating`, `weak`, \u2026).
    modifiers: Vec<String>,
    /// Attributes as `(name_without_at, line, col)`.
    attributes: Vec<(String, u32, u32)>,
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

    /// True when the cursor sits on an `@unknown` attribute that introduces the
    /// next switch clause (`@unknown default:` / `@unknown case ...:`), as
    /// opposed to an attribute attached to a declaration inside a clause body.
    fn at_unknown_clause(&self) -> bool {
        let t = self.peek();
        if t.kind != TokenKind::Attribute || t.text != "@unknown" {
            return false;
        }
        let next = self.tokens[self.pos + 1];
        next.kind == TokenKind::Keyword && (next.text == "default" || next.text == "case")
    }

    fn error<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        let t = self.peek();
        Err(ParseError {
            message: message.into(),
            line: t.line,
            col: t.col,
        })
    }

    /// Create a node whose span comes from `tok`, with `children` wired in.
    /// Prefer this over `self.ast.add` + repeated `self.ast.append_child` when
    /// all children are already computed before the parent node is created.
    fn node(
        &mut self,
        kind: NodeKind,
        text: Option<&str>,
        tok: Token<'a>,
        children: &[NodeId],
    ) -> NodeId {
        self.ast
            .add_with_children(kind, text, tok.line, tok.col, children)
    }

    fn expect(&mut self, kind: TokenKind) -> Result<Token<'a>, ParseError> {
        if self.peek().kind == kind {
            Ok(self.bump())
        } else {
            self.error(format!("expected {:?}, found {:?}", kind, self.peek().kind))
        }
    }

    /// Pre-scan the token stream for `operator` and `precedencegroup`
    /// declarations and record each infix operator's resolved precedence, so the
    /// expression parser nests user-defined operators correctly (a file-scope
    /// concern: operators may be used before their declaration appears).
    fn collect_operator_precedence(&mut self) {
        let mut groups: HashMap<String, RawPrecedenceGroup> = HashMap::new();
        let mut operators: Vec<(String, Option<String>)> = Vec::new();
        let mut i = 0;
        while i < self.tokens.len() {
            let t = self.tokens[i];
            if t.text == "precedencegroup" && t.kind == TokenKind::Identifier {
                i = self.scan_precedence_group(i, &mut groups);
                continue;
            }
            if t.text == "operator" && t.kind == TokenKind::Identifier {
                if let Some((op, group, next)) = self.scan_operator_decl(i) {
                    operators.push((op, group));
                    i = next;
                    continue;
                }
            }
            i += 1;
        }

        let mut resolved: HashMap<String, (u8, bool)> = HashMap::new();
        for (op, group) in operators {
            let bp = match group {
                Some(g) => resolve_group_precedence(&g, &groups, &mut resolved),
                None => (DEFAULT_BP, false), // operators without a group use DefaultPrecedence
            };
            self.op_precedence.insert(op, bp);
        }
    }

    /// Scan a `precedencegroup Name { … }` starting at `i` (the keyword); record
    /// the group and return the index just past its closing brace.
    fn scan_precedence_group(
        &self,
        i: usize,
        groups: &mut HashMap<String, RawPrecedenceGroup>,
    ) -> usize {
        let name = match self.tokens.get(i + 1) {
            Some(t) if t.kind == TokenKind::Identifier => t.text.to_string(),
            _ => return i + 1,
        };
        let mut j = i + 2;
        while j < self.tokens.len() && self.tokens[j].kind != TokenKind::LBrace {
            j += 1;
        }
        let mut group = RawPrecedenceGroup {
            higher_than: None,
            lower_than: None,
            right_associative: false,
        };
        j += 1; // past `{`
        while j < self.tokens.len() && self.tokens[j].kind != TokenKind::RBrace {
            let attr = self.tokens[j].text;
            let value = self.tokens.get(j + 2).map(|t| t.text);
            match attr {
                "higherThan" => group.higher_than = value.map(str::to_string),
                "lowerThan" => group.lower_than = value.map(str::to_string),
                "associativity" => group.right_associative = value == Some("right"),
                _ => {}
            }
            j += 1;
        }
        groups.insert(name, group);
        j
    }

    /// Scan a `[infix|prefix|postfix] operator <op> [: Group]` at the `operator`
    /// keyword index `i`. Returns `(operator, group, next_index)` for infix
    /// operators (the only fixity that carries precedence), else `None`.
    fn scan_operator_decl(&self, i: usize) -> Option<(String, Option<String>, usize)> {
        let fixity = if i > 0 { self.tokens[i - 1].text } else { "" };
        let mut k = i + 1;
        let mut op = String::new();
        while k < self.tokens.len()
            && self.tokens[k].kind == TokenKind::Oper
            && !self.tokens[k].leading_newline
        {
            op.push_str(self.tokens[k].text);
            k += 1;
        }
        if op.is_empty() || matches!(fixity, "prefix" | "postfix") {
            return None;
        }
        let group = if self.tokens.get(k).map(|t| t.kind) == Some(TokenKind::Colon) {
            self.tokens.get(k + 1).map(|t| t.text.to_string())
        } else {
            None
        };
        Some((op, group, k))
    }

    /// `(left_bp, right_bp)` for an infix operator: a user-declared operator's
    /// resolved group precedence if known, else the built-in table.
    fn binding_power(&self, op: &str) -> Option<(u8, u8)> {
        if let Some(&(p, right_assoc)) = self.op_precedence.get(op) {
            return Some(if right_assoc { (p, p) } else { (p, p + 1) });
        }
        builtin_binding_power(op)
    }

    fn parse_source_file(&mut self) -> Result<(), ParseError> {
        while !self.at_eof() {
            self.skip_semicolons();
            if self.at_eof() {
                break;
            }
            let root = self.ast.root();
            self.append_statement(root)?;
        }
        Ok(())
    }

    /// Parse one statement and require it to be terminated by a statement
    /// separator (a newline or `;`), enforcing Swift's automatic semicolon rule
    /// so `let x = 1 let y = 2` on one line is rejected.
    fn parse_statement_checked(&mut self) -> Result<NodeId, ParseError> {
        let stmt = self.parse_statement()?;
        let t = self.peek();
        let separated = matches!(
            t.kind,
            TokenKind::RBrace | TokenKind::Eof | TokenKind::Semicolon
        ) || t.leading_newline
            || t.kind == TokenKind::Directive;
        if !separated {
            return self.error("consecutive statements on a line must be separated by ';'");
        }
        Ok(stmt)
    }

    /// Parse one statement (checked) and append it to `parent`, along with any
    /// sibling declarations a single source statement desugared into (a
    /// multi-name binding `var a, b: T`).
    fn append_statement(&mut self, parent: NodeId) -> Result<(), ParseError> {
        let stmt = self.parse_statement_checked()?;
        self.ast.append_child(parent, stmt);
        for sibling in std::mem::take(&mut self.pending_siblings) {
            self.ast.append_child(parent, sibling);
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
        // Collect declaration modifiers (`static`, `public`, `final`, …) and
        // attributes (`@main`, `@propertyWrapper`, …) that precede a
        // declaration keyword, then attach them to the parsed declaration.
        let meta = self.collect_decl_meta();
        let node = self.parse_statement_body()?;
        self.attach_decl_meta(node, meta);
        Ok(node)
    }

    fn parse_statement_body(&mut self) -> Result<NodeId, ParseError> {
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
            // `actor Name { … }` (a contextual keyword, not a reserved word).
            if w == "actor" && self.tokens[self.pos + 1].kind == TokenKind::Identifier {
                return self.parse_nominal(NodeKind::ActorDecl);
            }
            // `discard self` / `discard expr` — ends a value's lifetime without
            // running its deinit. A no-op in the tree-walker: parse the operand
            // as a discarded expression statement so it is evaluated and dropped.
            if w == "discard"
                && !self.tokens[self.pos + 1].leading_newline
                && (self.tokens[self.pos + 1].kind == TokenKind::Identifier
                    || (self.tokens[self.pos + 1].kind == TokenKind::Keyword
                        && self.tokens[self.pos + 1].text == "self"))
            {
                let kw = self.bump();
                let operand = self.parse_expr(0)?;
                let stmt = self.ast.add(NodeKind::ExprStmt, None, kw.line, kw.col);
                self.ast.append_child(stmt, operand);
                return Ok(stmt);
            }
        }
        if self.peek().kind == TokenKind::Keyword {
            match self.peek().text {
                "import" => return self.parse_import(),
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

        // A multi-name binding (`var a, b, c: T` / `let x = 1, y = 2`) declares
        // several bindings on one line. Each is parsed independently; a binding
        // without its own type annotation inherits a later sibling's annotation
        // (Swift propagates a trailing `: T` to the preceding bare names).
        let mut entries: Vec<(NodeId, Option<NodeId>, Option<NodeId>)> = Vec::new();
        loop {
            let pattern = self.parse_pattern()?;
            let mut ty = None;
            if self.peek().kind == TokenKind::Colon {
                self.bump();
                ty = Some(self.parse_type()?);
            }
            let mut init = None;
            if self.at_oper("=") {
                self.bump();
                init = Some(self.parse_expr(0)?);
            }
            entries.push((pattern, ty, init));
            if self.peek().kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            break;
        }

        // Fast path: a single binding keeps the original (kind, [pattern, type?,
        // init?]) shape exactly.
        if entries.len() == 1 {
            let (pattern, ty, init) = entries.pop().unwrap();
            self.ast.append_child(decl, pattern);
            if let Some(ty) = ty {
                self.ast.append_child(decl, ty);
            }
            if let Some(init) = init {
                self.ast.append_child(decl, init);
            }
            return Ok(decl);
        }

        // Propagate a trailing type annotation backward to the bare names that
        // precede it and lack their own annotation/initializer.
        let mut inherited: Option<NodeId> = None;
        for entry in entries.iter_mut().rev() {
            match entry.1 {
                Some(ty) => inherited = Some(ty),
                None if entry.2.is_none() => entry.1 = inherited,
                None => {}
            }
        }

        // Emit one decl per entry. The first reuses `decl`; the rest are queued
        // as pending siblings for the enclosing parent.
        for (idx, (pattern, ty, init)) in entries.into_iter().enumerate() {
            let target = if idx == 0 {
                decl
            } else {
                let p = self.ast.node(pattern);
                self.ast.add(kind, None, p.line(), p.col())
            };
            self.ast.append_child(target, pattern);
            if let Some(ty) = ty {
                // Each binding needs its own annotation subtree; a shared
                // inherited annotation is deep-copied for every reuse beyond the
                // first.
                let ty = if idx == 0 {
                    ty
                } else {
                    self.ast.clone_subtree(ty)
                };
                self.ast.append_child(target, ty);
            }
            if let Some(init) = init {
                self.ast.append_child(target, init);
            }
            if idx != 0 {
                self.pending_siblings.push(target);
            }
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
        loop {
            self.skip_semicolons();
            if self.peek().kind == TokenKind::RBrace || self.at_eof() {
                break;
            }
            self.append_statement(block)?;
        }
        self.expect(TokenKind::RBrace)?;
        Ok(block)
    }

    /// `func name(params) [-> Ret] { body }`. Children: params, optional return
    /// `TypeRef`, then the body `Block`.
    fn parse_func(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        // The name is an identifier or an operator (`func == `, `func + `). A
        // custom operator may span several adjacent operator tokens (`^^`, `<>`).
        let name_tok = self.peek();
        let name = match name_tok.kind {
            TokenKind::Identifier => self.bump().text.to_string(),
            TokenKind::Oper => {
                let mut s = self.bump().text.to_string();
                while self.peek().kind == TokenKind::Oper && !self.peek().leading_newline {
                    s.push_str(self.bump().text);
                }
                s
            }
            other => return self.error(format!("expected a function name, found {other:?}")),
        };
        let func = self
            .ast
            .add(NodeKind::FuncDecl, Some(&name), kw.line, kw.col);
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
        // `async` is a contextual keyword (lexed as an identifier).
        if self.at_keyword("async")
            || (self.peek().kind == TokenKind::Identifier && self.peek().text == "async")
        {
            self.bump();
        }
        if self.at_keyword("throws") || self.at_keyword("rethrows") {
            self.bump();
            // Typed throws `throws(E)` — depth-balanced so an error type that
            // itself contains parentheses is consumed whole.
            if self.peek().kind == TokenKind::LParen {
                self.skip_balanced_parens();
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

    /// `[attributes] [externalLabel] name: [inout] Type [...] [= default]`.
    fn parse_param(&mut self) -> Result<NodeId, ParseError> {
        let mut attrs = Vec::new();
        while self.peek().kind == TokenKind::Attribute {
            let t = self.bump();
            attrs.push((t.text.trim_start_matches('@').to_string(), t.line, t.col));
        }

        let first = self.peek();
        if first.kind != TokenKind::Identifier {
            return self.error(format!("expected a parameter name, found {:?}", first.kind));
        }
        self.bump();
        // A second identifier before the colon means `first` was the external
        // label and the second token is the local binding name.
        let (label, name) = if self.peek().kind == TokenKind::Identifier {
            (Some(first.text), self.bump().text)
        } else {
            (None, first.text)
        };
        let param = self
            .ast
            .add(NodeKind::Param, Some(name), first.line, first.col);
        if let Some(label) = label {
            if label != "_" {
                self.ast.set_arg_label(param, label);
            }
        }
        for (name, line, col) in attrs {
            let attr = self.ast.add(NodeKind::Attribute, Some(&name), line, col);
            self.ast.append_child(param, attr);
        }
        self.expect(TokenKind::Colon)?;
        if self.at_keyword("inout") {
            self.bump();
            self.ast.add_modifier(param, "inout");
        }
        // Ownership parameter modifiers (`borrowing`/`consuming`, and the older
        // `__shared`/`__owned`). They do not change tree-walk evaluation, so
        // accept and discard them before the parameter type.
        while matches!(
            self.peek().text,
            "borrowing" | "consuming" | "__shared" | "__owned"
        ) && self.peek().kind == TokenKind::Identifier
        {
            self.bump();
        }
        // Type attributes on the parameter type (`@autoclosure`, `@escaping`).
        // They are recorded as modifiers on the parameter so the runtime can
        // defer an `@autoclosure` argument and treat an `@escaping` closure as
        // long-lived. `parse_type` would otherwise discard them.
        while self.peek().kind == TokenKind::Attribute {
            let name = self.peek().text.trim_start_matches('@').to_string();
            if name == "autoclosure" || name == "escaping" {
                self.ast.add_modifier(param, &name);
            }
            self.bump();
        }
        let ty = self.parse_type()?;
        self.ast.append_child(param, ty);
        if self.at_oper("...") {
            self.bump(); // variadic marker
            self.ast.add_modifier(param, "variadic");
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
        let next = self.peek();
        let ends = matches!(next.kind, TokenKind::RBrace | TokenKind::Eof);
        if !ends && !next.leading_newline {
            let expr = self.parse_expr(0)?;
            Ok(self.node(NodeKind::ReturnStmt, None, kw, &[expr]))
        } else {
            Ok(self.node(NodeKind::ReturnStmt, None, kw, &[]))
        }
    }

    /// `if cond { } [else (if ... | { })]`. Usable as a statement or expression.
    fn parse_if(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let mut children = self.parse_conditions()?;
        let then = self.parse_block()?;
        children.push(then);
        if self.at_keyword("else") {
            self.bump();
            let else_branch = if self.at_keyword("if") {
                self.parse_if()?
            } else {
                self.parse_block()?
            };
            children.push(else_branch);
        }
        Ok(self.node(NodeKind::IfStmt, None, kw, &children))
    }

    fn parse_guard(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let mut children = self.parse_conditions()?;
        if !self.at_keyword("else") {
            return self.error("expected 'else' after the guard condition");
        }
        self.bump();
        let body = self.parse_block()?;
        children.push(body);
        Ok(self.node(NodeKind::GuardStmt, None, kw, &children))
    }

    fn parse_while(&mut self, label: Option<&str>) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let mut children = self.parse_conditions()?;
        let body = self.parse_block()?;
        children.push(body);
        Ok(self.node(NodeKind::WhileStmt, label, kw, &children))
    }

    fn parse_repeat(&mut self, label: Option<&str>) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let body = self.parse_block()?;
        if !self.at_keyword("while") {
            return self.error("expected 'while' after a repeat body");
        }
        self.bump();
        let cond = self.parse_expr(0)?;
        Ok(self.node(NodeKind::RepeatStmt, label, kw, &[body, cond]))
    }

    /// `for pattern in iterable [where cond] { body }`. Children: pattern,
    /// iterable, optional where-expr, then the body `Block` (always last).
    fn parse_for(&mut self, label: Option<&str>) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let node = self.ast.add(NodeKind::ForStmt, label, kw.line, kw.col);
        // `for [try] [await] x in seq` — asynchronous iteration over an
        // `AsyncSequence` (`await`, contextual) that may throw (`try`). Swift
        // spells the effects `for try await`; accept either order defensively.
        loop {
            if self.peek().kind == TokenKind::Identifier && self.peek().text == "await" {
                self.bump();
                self.ast.add_modifier(node, "async");
            } else if self.at_keyword("try") {
                self.bump();
                self.ast.add_modifier(node, "throws");
            } else {
                break;
            }
        }
        // `for case <pattern> in seq` — pattern-matching iteration.
        let pattern = if self.at_keyword("case") {
            self.bump();
            self.parse_case_pattern(false)?
        } else {
            self.parse_pattern()?
        };
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
        while self.at_keyword("case") || self.at_keyword("default") || self.at_unknown_clause() {
            let clause = self.parse_case_clause()?;
            self.ast.append_child(node, clause);
        }
        self.expect(TokenKind::RBrace)?;
        Ok(node)
    }

    /// One `case items [where cond]:` or `default:` clause. Children: the case
    /// items, an optional where-expr, then a `Block` of the clause body (last).
    fn parse_case_clause(&mut self) -> Result<NodeId, ParseError> {
        // `@unknown default:` / `@unknown case ...:` — accept (and discard) the
        // `@unknown` attribute that may precede the clause in an exhaustive
        // switch. Attributes attached to declarations in a clause body are left
        // for statement parsing.
        while self.at_unknown_clause() {
            self.bump();
        }
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
                let w = self.bump();
                let cond = self.parse_expr_no_trailing(0)?;
                let where_node = self.ast.add(NodeKind::WhereClause, None, w.line, w.col);
                self.ast.append_child(where_node, cond);
                self.ast.append_child(clause, where_node);
            }
        }
        self.expect(TokenKind::Colon)?;
        let body = self.ast.add(NodeKind::Block, None, kw.line, kw.col);
        loop {
            self.skip_semicolons();
            if self.at_keyword("case")
                || self.at_keyword("default")
                || self.peek().kind == TokenKind::RBrace
                || self.at_eof()
                // An `@unknown` attribute begins the next clause (`@unknown
                // default:`), not a statement in this clause's body. Other
                // attributes belong to a declaration statement in this body.
                || self.at_unknown_clause()
            {
                break;
            }
            self.append_statement(body)?;
        }
        self.ast.append_child(clause, body);
        Ok(clause)
    }

    /// A `case` item: a `let`/`var` binding pattern or a value-pattern expression.
    fn parse_case_item(&mut self) -> Result<NodeId, ParseError> {
        self.parse_case_pattern(false)
    }

    /// Parse one `switch`/`for-case` pattern. `binding` is `true` when an
    /// enclosing `let`/`var` makes bare identifiers bind values rather than
    /// match by equality. Produces runtime-facing pattern nodes:
    /// `NamePattern`, `WildcardPattern`, `TuplePattern`, `EnumCasePattern`,
    /// `RangePattern`, or a value/expression pattern node.
    fn parse_case_pattern(&mut self, binding: bool) -> Result<NodeId, ParseError> {
        let pat = self.parse_case_pattern_inner(binding)?;
        // `<pattern> as Type` — a cast pattern (`case let x as String`,
        // `catch let e as MyError`): match only when the subject is of `Type`.
        if self.at_keyword("as") {
            let kw = self.bump();
            let mut op = "as".to_string();
            if self.peek().kind == TokenKind::Question || self.at_oper("!") {
                op.push_str(self.bump().text);
            }
            let ty = self.parse_type()?;
            let cast = self.ast.add(NodeKind::CastExpr, Some(&op), kw.line, kw.col);
            self.ast.append_child(cast, pat);
            self.ast.append_child(cast, ty);
            return Ok(cast);
        }
        Ok(pat)
    }

    fn parse_case_pattern_inner(&mut self, binding: bool) -> Result<NodeId, ParseError> {
        // `let`/`var` introduce (or re-enter) a binding context.
        if self.at_keyword("let") || self.at_keyword("var") {
            self.bump();
            return self.parse_case_pattern(true);
        }
        // Wildcard `_`.
        if self.peek().kind == TokenKind::Identifier && self.peek().text == "_" {
            let t = self.bump();
            return Ok(self.ast.add(NodeKind::WildcardPattern, None, t.line, t.col));
        }
        // `is Type` — a type-check pattern matching any subject of that type.
        if self.at_keyword("is") {
            let kw = self.bump();
            let ty = self.parse_type()?;
            let wild = self
                .ast
                .add(NodeKind::WildcardPattern, None, kw.line, kw.col);
            let cast = self
                .ast
                .add(NodeKind::CastExpr, Some("is"), kw.line, kw.col);
            self.ast.append_child(cast, wild);
            self.ast.append_child(cast, ty);
            return Ok(cast);
        }
        // Enum-case pattern: `.case[(subpatterns)]` or `Type.case[(...)]`.
        if self.at_enum_case_pattern() {
            return self.parse_enum_case_pattern(binding);
        }
        // Tuple pattern `(p, q, ...)`.
        if self.peek().kind == TokenKind::LParen {
            return self.parse_tuple_pattern(binding);
        }
        // A bare identifier in a binding context binds the value.
        if binding && self.peek().kind == TokenKind::Identifier {
            let t = self.bump();
            let name = self
                .ast
                .add(NodeKind::NamePattern, Some(t.text), t.line, t.col);
            // `name?` is the optional-pattern shorthand: it matches only a
            // non-`nil` value and binds the unwrapped payload, so it lowers to
            // a refutable `.some(name)` enum-case pattern.
            if self.peek().kind == TokenKind::Question {
                self.bump();
                let some = self
                    .ast
                    .add(NodeKind::EnumCasePattern, Some("some"), t.line, t.col);
                self.ast.append_child(some, name);
                return Ok(some);
            }
            return Ok(name);
        }
        // One-sided range patterns with a leading range operator:
        // `case ..<n:` (PartialRangeUpTo) and `case ...n:` (PartialRangeThrough).
        // Each lowers to a single-bound `RangePattern` tagged by direction.
        if self.at_oper("..<") || self.at_oper("...") {
            let op = self.bump();
            let bound = self.parse_expr(RANGE_RBP)?;
            let marker = if op.text == "..<" { "upTo" } else { "through" };
            let node = self
                .ast
                .add(NodeKind::RangePattern, Some(marker), op.line, op.col);
            self.ast.append_child(node, bound);
            return Ok(node);
        }
        // Otherwise a value/expression pattern (a literal, range, etc.). Parse
        // the leading operand at range precedence so a trailing range operator
        // is left for the one-sided / two-sided handling below.
        let lhs = self.parse_expr(RANGE_RBP)?;
        // Postfix one-sided range `case n...:` (PartialRangeFrom): a `...` with
        // no upper operand following it.
        if self.at_oper("...") && self.at_one_sided_range_end() {
            let op = self.bump();
            let node = self
                .ast
                .add(NodeKind::RangePattern, Some("from"), op.line, op.col);
            self.ast.append_child(node, lhs);
            return Ok(node);
        }
        // Two-sided range `case lo...hi:` / `case lo..<hi:` matches by
        // containment.
        if self.at_oper("...") || self.at_oper("..<") {
            let op = self.bump();
            let hi = self.parse_expr(RANGE_RBP)?;
            let node = self
                .ast
                .add(NodeKind::RangePattern, Some(op.text), op.line, op.col);
            self.ast.append_child(node, lhs);
            self.ast.append_child(node, hi);
            return Ok(node);
        }
        Ok(lhs)
    }

    /// After a `...` at the cursor, whether the following token cannot begin an
    /// upper-bound expression — i.e. the range is the postfix one-sided form
    /// `n...` (PartialRangeFrom) rather than a two-sided `lo...hi`.
    fn at_one_sided_range_end(&self) -> bool {
        let next = &self.tokens[self.pos + 1];
        next.leading_newline
            || matches!(
                next.kind,
                TokenKind::Colon
                    | TokenKind::Comma
                    | TokenKind::RBrace
                    | TokenKind::RParen
                    | TokenKind::RBracket
                    | TokenKind::Eof
            )
            || (next.kind == TokenKind::Keyword && next.text == "where")
    }

    /// Whether the cursor begins an enum-case pattern (`.case` or `Type.case`).
    fn at_enum_case_pattern(&self) -> bool {
        if self.peek().kind == TokenKind::Dot {
            return true;
        }
        // `Type.case`: an identifier whose next token is a dot.
        self.peek().kind == TokenKind::Identifier
            && self.tokens[self.pos + 1].kind == TokenKind::Dot
    }

    fn parse_enum_case_pattern(&mut self, binding: bool) -> Result<NodeId, ParseError> {
        let start = self.peek();
        // Optional `Type` prefix (consumed but not required by the runtime).
        if self.peek().kind == TokenKind::Identifier {
            self.bump();
        }
        self.expect(TokenKind::Dot)?;
        // The case name is usually an identifier, but `.some` uses the
        // contextual keyword `some`, and `.none` is an identifier.
        let case = match self.peek().kind {
            TokenKind::Identifier | TokenKind::Keyword => self.bump(),
            other => return self.error(format!("expected a case name, found {other:?}")),
        };
        let node = self.ast.add(
            NodeKind::EnumCasePattern,
            Some(case.text),
            start.line,
            start.col,
        );
        if self.peek().kind == TokenKind::LParen {
            self.bump();
            if self.peek().kind != TokenKind::RParen {
                loop {
                    // Allow `label: pattern` payload labels (label ignored).
                    if self.peek().kind == TokenKind::Identifier
                        && self.tokens[self.pos + 1].kind == TokenKind::Colon
                    {
                        self.bump();
                        self.bump();
                    }
                    let sub = self.parse_case_pattern(binding)?;
                    self.ast.append_child(node, sub);
                    if self.peek().kind == TokenKind::Comma {
                        self.bump();
                        continue;
                    }
                    break;
                }
            }
            self.expect(TokenKind::RParen)?;
        }
        Ok(node)
    }

    fn parse_tuple_pattern(&mut self, binding: bool) -> Result<NodeId, ParseError> {
        let open = self.expect(TokenKind::LParen)?;
        let node = self
            .ast
            .add(NodeKind::TuplePattern, None, open.line, open.col);
        if self.peek().kind != TokenKind::RParen {
            loop {
                let sub = self.parse_case_pattern(binding)?;
                self.ast.append_child(node, sub);
                if self.peek().kind == TokenKind::Comma {
                    self.bump();
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen)?;
        Ok(node)
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

    /// Collect any run of leading declaration modifiers (`static`, `public`,
    /// `final`, …) and attributes (`@main`, `@propertyWrapper`, …) that precede
    /// a declaration keyword, consuming them and returning them so the parsed
    /// declaration can be annotated via [`attach_decl_meta`].
    fn collect_decl_meta(&mut self) -> DeclMeta {
        // First confirm the run actually precedes a declaration (mirrors the
        // old `skip_modifiers` guard), so a bare `weak`/`@x` used elsewhere is
        // not mistaken for a modifier run.
        let mut i = self.pos;
        let mut saw_objc_attr = false;
        loop {
            let t = self.tokens[i];
            // `class` is a modifier (`class func`) only before another token; a
            // following identifier means it is the `class Name` declaration keyword.
            let is_mod = if t.text == "class" {
                self.tokens[i + 1].kind != TokenKind::Identifier
            } else if t.text == "optional" {
                saw_objc_attr && matches!(self.tokens[i + 1].text, "func" | "var" | "subscript")
            } else {
                is_modifier_word(t.text)
            };
            if t.kind == TokenKind::Attribute || is_mod {
                if t.kind == TokenKind::Attribute && t.text == "@objc" {
                    saw_objc_attr = true;
                }
                i += 1;
                // Argumented attribute/modifier such as `@available(...)` or
                // `private(set)`.
                if self.tokens[i].kind == TokenKind::LParen {
                    i = self.scan_balanced_parens(i);
                }
            } else {
                break;
            }
        }
        let mut meta = DeclMeta::default();
        if i == self.pos
            || !(is_decl_keyword(self.tokens[i].text)
                || self.tokens[i].kind == TokenKind::Attribute)
        {
            return meta;
        }
        while self.pos < i {
            let t = self.peek();
            if t.kind == TokenKind::Attribute {
                let name = t.text.trim_start_matches('@').to_string();
                meta.attributes.push((name, t.line, t.col));
                self.bump();
                if self.peek().kind == TokenKind::LParen {
                    self.skip_balanced_parens();
                }
            } else {
                meta.modifiers.push(t.text.to_string());
                self.bump();
                if self.peek().kind == TokenKind::LParen {
                    self.skip_balanced_parens();
                }
            }
        }
        meta
    }

    /// Attach collected modifiers and attribute child nodes to a parsed
    /// declaration. A no-op when `meta` is empty (the common statement case).
    fn attach_decl_meta(&mut self, node: NodeId, meta: DeclMeta) {
        for m in &meta.modifiers {
            self.ast.add_modifier(node, m);
        }
        for (name, line, col) in meta.attributes {
            let attr = self.ast.add(NodeKind::Attribute, Some(&name), line, col);
            self.ast.append_child(node, attr);
        }
    }

    /// Consume a balanced `( ... )` run starting at the current `(`.
    fn skip_balanced_parens(&mut self) {
        let end = self.scan_balanced_parens(self.pos);
        while self.pos < end {
            self.bump();
        }
    }

    /// Skip any run of statement-separating semicolons.
    fn skip_semicolons(&mut self) {
        while self.peek().kind == TokenKind::Semicolon {
            self.bump();
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
        // Swift 6 `do throws(E) { ... }`: the (typed) effect marker is
        // accepted and skipped; catch clauses drive runtime behaviour.
        self.skip_effects();
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
                    let pat = self.parse_case_pattern(false)?;
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

    /// `import [kind] Module[.submodule...]`.
    fn parse_import(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        // Optional import kind specifier (`import func Foo.bar`).
        if self.peek().kind == TokenKind::Keyword
            && is_import_kind(self.peek().text)
            && self.tokens[self.pos + 1].kind == TokenKind::Identifier
        {
            self.bump();
        }
        let mut path = String::new();
        while self.peek().kind == TokenKind::Identifier {
            path.push_str(self.bump().text);
            if self.peek().kind == TokenKind::Dot {
                self.bump();
                path.push('.');
            } else {
                break;
            }
        }
        Ok(self
            .ast
            .add(NodeKind::ImportDecl, Some(&path), kw.line, kw.col))
    }

    /// `throw expr`.
    fn parse_throw(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let expr = self.parse_expr(0)?;
        Ok(self.node(NodeKind::ThrowStmt, None, kw, &[expr]))
    }

    /// `defer { }`.
    fn parse_defer(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump();
        let body = self.parse_block()?;
        Ok(self.node(NodeKind::DeferStmt, None, kw, &[body]))
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
        // `#sourceLocation(file:line:)` / `#sourceLocation()` controls the
        // reported source position. Its arguments are labelled, not an
        // expression, and it is a no-op for the tree-walker, so skip the whole
        // balanced argument list.
        if dir.text == "#sourceLocation" {
            if self.peek().kind == TokenKind::LParen {
                self.skip_balanced_parens();
            }
            return Ok(node);
        }
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
                    self.append_statement(node)?;
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
            self.parse_capture_list(node)?;
        }
        self.try_closure_signature(node);
        let saved = self.no_trailing_closure;
        self.no_trailing_closure = false;
        loop {
            self.skip_semicolons();
            if self.peek().kind == TokenKind::RBrace || self.at_eof() {
                break;
            }
            self.append_statement(node)?;
        }
        self.no_trailing_closure = saved;
        self.expect(TokenKind::RBrace)?;
        Ok(node)
    }

    /// A closure capture list `[weak self, base = 100, x]`, appending a
    /// `ClosureCapture` child per entry (text = name, optional child = its
    /// initializer expression). Ownership keywords (`weak`/`unowned`) are
    /// recorded as modifiers.
    fn parse_capture_list(&mut self, closure: NodeId) -> Result<(), ParseError> {
        self.expect(TokenKind::LBracket)?;
        if self.peek().kind == TokenKind::RBracket {
            self.bump();
            return Ok(());
        }
        loop {
            let mut ownership = None;
            if matches!(self.peek().text, "weak" | "unowned")
                && self.peek().kind == TokenKind::Keyword
            {
                ownership = Some(self.bump().text);
                // `unowned(unsafe)` / `unowned(safe)`.
                if self.peek().kind == TokenKind::LParen {
                    self.skip_balanced_parens();
                }
            }
            let name = match self.peek().kind {
                TokenKind::Identifier | TokenKind::Keyword => self.bump(),
                other => return self.error(format!("expected a capture name, found {other:?}")),
            };
            let cap = self.ast.add(
                NodeKind::ClosureCapture,
                Some(name.text),
                name.line,
                name.col,
            );
            if let Some(kw) = ownership {
                self.ast.add_modifier(cap, kw);
            }
            if self.at_oper("=") {
                self.bump();
                let init = self.parse_expr(0)?;
                self.ast.append_child(cap, init);
            }
            self.ast.append_child(closure, cap);
            if self.peek().kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            break;
        }
        self.expect(TokenKind::RBracket)?;
        Ok(())
    }

    /// Tentatively consume a closure signature ending in `in`; restore the
    /// cursor and return `false` if the upcoming tokens are not a signature.
    fn try_closure_signature(&mut self, node: NodeId) -> bool {
        let save = self.pos;
        // Identifiers in name position become the closure's `Param` children;
        // tokens after `:` (a type) or `->` (the return type) are skipped.
        let mut names: Vec<(&'a str, u32, u32, bool)> = Vec::new();
        let mut expect_name = true;
        let mut in_type = false;
        // Parenthesis depth: the closure's `in` separator sits at depth 0. An
        // `in:` argument label inside a nested call (e.g. `Slider(value: x, in:
        // 0...1)`) is at depth > 0 and must not be mistaken for the separator.
        let mut depth = 0i32;
        loop {
            let t = self.peek();
            if depth == 0 && t.kind == TokenKind::Keyword && t.text == "in" {
                self.bump();
                for (name, line, col, is_inout) in names {
                    let p = self.ast.add(NodeKind::Param, Some(name), line, col);
                    if is_inout {
                        self.ast.add_modifier(p, "inout");
                    }
                    self.ast.append_child(node, p);
                }
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
                || (t.kind == TokenKind::Keyword
                    && matches!(t.text, "inout" | "throws" | "rethrows" | "async"));
            if !signature_like {
                self.pos = save;
                return false;
            }
            match t.kind {
                TokenKind::Identifier if expect_name && !in_type && t.text != "_" => {
                    names.push((t.text, t.line, t.col, false));
                    expect_name = false;
                }
                // `inout` after the colon marks the current parameter.
                TokenKind::Keyword if t.text == "inout" => {
                    if let Some(last) = names.last_mut() {
                        last.3 = true;
                    }
                }
                // Effect keywords (`throws`/`rethrows`/`async`) in the closure
                // signature are consumed and ignored; the body's actual
                // throwing/async-ness is inferred during evaluation. A typed
                // throws `throws(E)` consumes its parenthesized error type
                // wholesale so `E` is not mistaken for a parameter name.
                TokenKind::Keyword if matches!(t.text, "throws" | "rethrows" | "async") => {
                    self.bump();
                    if self.peek().kind == TokenKind::LParen {
                        let mut d = 0i32;
                        loop {
                            match self.peek().kind {
                                TokenKind::LParen => d += 1,
                                TokenKind::RParen => {
                                    if d == 1 {
                                        self.bump();
                                        break;
                                    }
                                    d -= 1;
                                }
                                TokenKind::Eof => {
                                    self.pos = save;
                                    return false;
                                }
                                _ => {}
                            }
                            self.bump();
                        }
                    }
                    continue;
                }
                TokenKind::Comma => {
                    expect_name = true;
                    in_type = false;
                }
                TokenKind::LParen => {
                    depth += 1;
                    expect_name = true;
                }
                TokenKind::RParen => {
                    depth -= 1;
                    in_type = false;
                }
                TokenKind::Colon => in_type = true,
                TokenKind::Oper => in_type = true, // `->`
                _ => {}
            }
            self.bump();
        }
    }

    /// Parse a comma-separated condition list (`if`/`guard`/`while`) and return
    /// the condition nodes. Callers wire them into the parent with `self.node()`
    /// or explicit `append_child` calls.
    fn parse_conditions(&mut self) -> Result<Vec<NodeId>, ParseError> {
        let saved = self.no_trailing_closure;
        self.no_trailing_closure = true;
        let result = self.parse_conditions_inner();
        self.no_trailing_closure = saved;
        result
    }

    fn parse_conditions_inner(&mut self) -> Result<Vec<NodeId>, ParseError> {
        let mut conditions = Vec::new();
        loop {
            let cond = if self.at_keyword("let") || self.at_keyword("var") {
                self.parse_condition_binding()?
            } else if self.at_keyword("case") {
                self.parse_case_condition()?
            } else {
                self.parse_expr(0)?
            };
            conditions.push(cond);
            if self.peek().kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            break;
        }
        Ok(conditions)
    }

    /// One optional-binding condition (`let x`, `let x = e`, `var y = e`) inside
    /// an `if`/`guard`/`while` condition list. Unlike a statement-level binding,
    /// it parses exactly one binding so a following comma separates the next
    /// *condition* (`if let a = a, let b = b`) rather than another name.
    fn parse_condition_binding(&mut self) -> Result<NodeId, ParseError> {
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

    /// A `case <pattern> = <expr>` condition (`if case let x? = optional`).
    /// Lowered like an optional binding: the bound pattern plus the matched
    /// expression become a `LetDecl`, so a `case let x?` binds `x` just as
    /// `if let x` would.
    fn parse_case_condition(&mut self) -> Result<NodeId, ParseError> {
        let kw = self.bump(); // `case`
        let pattern = self.parse_case_pattern(false)?;
        // `if case let v? = e` keeps the optional-binding shape (`if let v = e`):
        // unwrap the `.some(v)` shorthand back to its inner name pattern so the
        // condition lowers to a plain `let v` rather than a refutable match.
        let pattern = self.unwrap_some_name_pattern(pattern);
        if !self.at_oper("=") {
            return self.error("expected '=' after a 'case' condition pattern");
        }
        self.bump();
        let expr = self.parse_expr_no_trailing(0)?;
        let decl = self.ast.add(NodeKind::LetDecl, None, kw.line, kw.col);
        self.ast.append_child(decl, pattern);
        self.ast.append_child(decl, expr);
        Ok(decl)
    }

    /// Unwrap a `.some(name)` optional shorthand (`EnumCasePattern "some"` over
    /// a single `NamePattern`) back to that inner name pattern; any other
    /// pattern is returned unchanged.
    fn unwrap_some_name_pattern(&self, id: NodeId) -> NodeId {
        let node = self.ast.node(id);
        if node.kind() == NodeKind::EnumCasePattern && node.text() == Some("some") {
            let mut kids = node.children();
            if let Some(child) = kids.next() {
                if kids.next().is_none() && child.kind() == NodeKind::NamePattern {
                    return child.id();
                }
            }
        }
        id
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
        loop {
            self.skip_semicolons();
            if self.peek().kind == TokenKind::RBrace || self.at_eof() {
                break;
            }
            if self.at_keyword("case") {
                self.parse_enum_cases(node)?;
            } else {
                self.append_statement(node)?;
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
            let name = match self.peek().kind {
                TokenKind::Identifier => self.bump(),
                TokenKind::Keyword if matches!(self.peek().text, "some" | "any") => self.bump(),
                other => return self.error(format!("expected enum case name, found {other:?}")),
            };
            let case = self
                .ast
                .add(NodeKind::EnumCaseDecl, Some(name.text), name.line, name.col);
            if self.peek().kind == TokenKind::LParen {
                self.bump();
                if self.peek().kind != TokenKind::RParen {
                    loop {
                        // Optional associated-value label `name:` before the type.
                        if self.peek().kind == TokenKind::Identifier
                            && self.tokens[self.pos + 1].kind == TokenKind::Colon
                        {
                            self.bump();
                            self.bump();
                        }
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
        loop {
            self.skip_semicolons();
            if self.peek().kind == TokenKind::RBrace || self.at_eof() {
                break;
            }
            if self.at_keyword("case") {
                self.parse_enum_cases(node)?;
            } else {
                self.append_statement(node)?;
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
        // Generic subscripts: `subscript<T>(...) -> ...`.
        if self.at_oper("<") {
            self.parse_generic_clause(node);
        }
        self.parse_param_list(node)?;
        if self.at_oper("->") {
            self.bump();
            let ret = self.parse_type()?;
            self.ast.append_child(node, ret);
        }
        // Generic constraint clause: `subscript<T>(...) -> ... where T: P`.
        if self.at_keyword("where") {
            self.skip_where_clause();
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
        if is_accessor_start(self.peek().text) {
            while is_accessor_start(self.peek().text) {
                // An optional `mutating`/`nonmutating` modifier precedes the
                // accessor keyword (`nonmutating set` on a `Binding`-style
                // setter that writes through a reference).
                let mut mutation: Option<&'static str> = None;
                if matches!(self.peek().text, "mutating" | "nonmutating") {
                    mutation = Some(if self.peek().text == "nonmutating" {
                        "nonmutating"
                    } else {
                        "mutating"
                    });
                    self.bump();
                }
                let kw = self.bump();
                let acc = self
                    .ast
                    .add(NodeKind::Accessor, Some(kw.text), kw.line, kw.col);
                // Explicit accessor parameter `set(newValue)` / `willSet(nv)`.
                if self.peek().kind == TokenKind::LParen {
                    self.bump();
                    let pname = self.expect(TokenKind::Identifier)?;
                    let param =
                        self.ast
                            .add(NodeKind::Param, Some(pname.text), pname.line, pname.col);
                    self.ast.append_child(acc, param);
                    self.expect(TokenKind::RParen)?;
                }
                if let Some(m) = mutation {
                    self.ast.add_modifier(acc, m);
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
            loop {
                self.skip_semicolons();
                if self.peek().kind == TokenKind::RBrace || self.at_eof() {
                    break;
                }
                self.append_statement(block)?;
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
        // Type attributes `@escaping`, `@autoclosure`, `@Sendable`, … prefix a
        // type. A following `(` belongs to the type (e.g. `@escaping () -> Void`),
        // not the attribute, so it is left for the type grammar to consume.
        while self.peek().kind == TokenKind::Attribute {
            self.bump();
        }
        // Suppressed constraint `~Copyable` / `~Escapable`: a tilde prefix that
        // removes an implicit conformance. It is a no-op for the tree-walker, so
        // keep the marker in the type text and let the runtime ignore it.
        if self.at_oper("~") {
            self.bump();
            let rest = self.parse_type_text()?;
            return Ok(format!("~{rest}"));
        }
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
                        // Optional tuple-element label `name: Type`.
                        if self.peek().kind == TokenKind::Identifier
                            && self.tokens[self.pos + 1].kind == TokenKind::Colon
                        {
                            let label = self.bump().text;
                            self.bump(); // ':'
                            let ty = self.parse_type_text()?;
                            parts.push(format!("{label}: {ty}"));
                        } else {
                            parts.push(self.parse_type_text()?);
                        }
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
                lhs = self.node(
                    NodeKind::TernaryExpr,
                    None,
                    q,
                    &[lhs, then_branch, else_branch],
                );
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
                lhs = self.node(NodeKind::CastExpr, Some(&op), kw, &[lhs, ty]);
                continue;
            }
            let op = self.peek();
            if op.kind != TokenKind::Oper || is_assignment(op.text) {
                break;
            }
            let (lbp, rbp) = match self.binding_power(op.text) {
                Some(bp) => bp,
                None => break,
            };
            if lbp < min_bp {
                break;
            }
            // Postfix one-sided range `lhs...` (`PartialRangeFrom`): a `...`
            // whose following token cannot begin an upper-bound expression.
            if op.text == "..." && self.at_one_sided_range_end() {
                self.bump();
                let node = self
                    .ast
                    .add(NodeKind::PostfixExpr, Some("..."), op.line, op.col);
                self.ast.append_child(node, lhs);
                lhs = node;
                continue;
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
        // A key-path expression `\Root.path` / `\.path`.
        if t.kind == TokenKind::Oper && t.text == "\\" {
            return self.parse_keypath();
        }
        // A bare operator used as a value — an operator function reference such
        // as the `+` in `reduce(0, +)` or the `>` in `sorted(by: >)`.
        if t.kind == TokenKind::Oper
            && matches!(
                self.tokens[self.pos + 1].kind,
                TokenKind::RParen | TokenKind::Comma
            )
        {
            self.bump();
            return Ok(self
                .ast
                .add(NodeKind::IdentExpr, Some(t.text), t.line, t.col));
        }
        // One-sided range prefix `..<n` / `...n` (`PartialRangeUpTo`/`Through`).
        if t.kind == TokenKind::Oper && matches!(t.text, "..<" | "...") {
            self.bump();
            let operand = self.parse_prefix()?;
            let node = self
                .ast
                .add(NodeKind::PrefixExpr, Some(t.text), t.line, t.col);
            self.ast.append_child(node, operand);
            return Ok(node);
        }
        // `try` / `try?` / `try!` — an error-propagation prefix.
        if t.kind == TokenKind::Keyword && t.text == "try" {
            self.bump();
            // The runtime-facing payload is the variant marker alone: `?` for
            // `try?`, `!` for `try!`, and `try` for the transparent form.
            let op = if self.peek().kind == TokenKind::Question || self.at_oper("!") {
                self.bump().text.to_string()
            } else {
                String::from("try")
            };
            let operand = self.parse_prefix()?;
            return Ok(self.node(NodeKind::TryExpr, Some(&op), t, &[operand]));
        }
        // `await expr` — suspends until the operand's task completes. Wrapped
        // in an `AwaitExpr` so the runtime can resolve task/async-let values.
        if t.kind == TokenKind::Identifier && t.text == "await" && !self.is_value_ident_context() {
            self.bump();
            let operand = self.parse_prefix()?;
            return Ok(self.node(NodeKind::AwaitExpr, Some("await"), t, &[operand]));
        }
        // Ownership prefix operators `consume`/`copy`/`borrow expr`. They are
        // contextual keywords; treat them as a transparent prefix only when an
        // expression follows (otherwise the word is an ordinary identifier).
        if t.kind == TokenKind::Identifier
            && matches!(t.text, "consume" | "copy" | "borrow")
            && !self.tokens[self.pos + 1].leading_newline
            && (self.tokens[self.pos + 1].kind == TokenKind::Identifier
                || (self.tokens[self.pos + 1].kind == TokenKind::Keyword
                    && self.tokens[self.pos + 1].text == "self"))
        {
            self.bump();
            let operand = self.parse_prefix()?;
            return Ok(self.node(NodeKind::PrefixExpr, Some(t.text), t, &[operand]));
        }
        // `&place` — an inout argument (write-back location at a call site).
        if t.kind == TokenKind::Oper && t.text == "&" {
            self.bump();
            let operand = self.parse_prefix()?;
            return Ok(self.node(NodeKind::InoutExpr, Some("&"), t, &[operand]));
        }
        if t.kind == TokenKind::Oper && matches!(t.text, "-" | "+" | "!" | "~") {
            self.bump();
            let operand = self.parse_prefix()?;
            return Ok(self.node(NodeKind::PrefixExpr, Some(t.text), t, &[operand]));
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

    /// `\Root.path.subpath` or `\.path` (the root type is inferred). The result
    /// is a `KeyPathExpr` whose optional leading `TypeRef` child names the root
    /// type, followed by one `IdentExpr` child per path component.
    fn parse_keypath(&mut self) -> Result<NodeId, ParseError> {
        let bs = self.bump(); // the `\` sigil
        let node = self.ast.add(NodeKind::KeyPathExpr, None, bs.line, bs.col);
        // An optional root type precedes the first `.component`. `\.path` omits
        // it (the root is inferred from context).
        if self.peek().kind == TokenKind::Identifier {
            let root = self.bump();
            let r = self
                .ast
                .add(NodeKind::TypeRef, Some(root.text), root.line, root.col);
            self.ast.append_child(node, r);
        }
        // Path components: `.name` (a property) repeated. `self` is a valid
        // component naming the whole value.
        while self.peek().kind == TokenKind::Dot {
            self.bump();
            let comp = self.peek();
            if comp.kind != TokenKind::Identifier
                && !(comp.kind == TokenKind::Keyword && comp.text == "self")
            {
                return self.error(format!(
                    "expected a key-path component, found {:?}",
                    comp.kind
                ));
            }
            self.bump();
            let c = self
                .ast
                .add(NodeKind::IdentExpr, Some(comp.text), comp.line, comp.col);
            self.ast.append_child(node, c);
            // Optional-chaining marker in a key path (`\.a?.b`): ignored, the
            // runtime treats nil access as nil.
            if self.peek().kind == TokenKind::Question {
                self.bump();
            }
        }
        Ok(node)
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
            TokenKind::RegexLiteral => {
                self.bump();
                self.ast
                    .add(NodeKind::RegexLiteral, Some(t.text), t.line, t.col)
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
            TokenKind::LBracket => return self.parse_array_or_dict(),
            // Directive expression `#file`, `#line`, `#function`, `#column`, or
            // an availability check `#available(...)` / `#unavailable(...)` whose
            // argument list is not an ordinary expression, so it is consumed
            // verbatim rather than parsed as call arguments.
            TokenKind::Directive => {
                self.bump();
                let node = self
                    .ast
                    .add(NodeKind::CompilerDirective, Some(t.text), t.line, t.col);
                // `#selector(Type.member)` / `#keyPath(Type.path)` reference a
                // member; keep the operand as a child so the runtime can read
                // its name. An optional `getter:`/`setter:` label is skipped.
                if matches!(t.text, "#selector" | "#keyPath")
                    && self.peek().kind == TokenKind::LParen
                {
                    self.bump(); // `(`
                    if self.peek().kind == TokenKind::Identifier
                        && matches!(self.peek().text, "getter" | "setter")
                        && self.tokens[self.pos + 1].kind == TokenKind::Colon
                    {
                        self.bump(); // label
                        self.bump(); // `:`
                    }
                    // Parse a dotted member path `Root.a.b` directly (rather than
                    // a general expression) so a selector signature suffix like
                    // `update(_:)` does not confuse the expression grammar.
                    let root = self.expect(TokenKind::Identifier)?;
                    let mut path =
                        self.ast
                            .add(NodeKind::IdentExpr, Some(root.text), root.line, root.col);
                    while self.peek().kind == TokenKind::Dot {
                        self.bump();
                        let name = self.expect(TokenKind::Identifier)?;
                        let member = self.ast.add(
                            NodeKind::MemberExpr,
                            Some(name.text),
                            name.line,
                            name.col,
                        );
                        self.ast.append_child(member, path);
                        path = member;
                    }
                    // Skip a trailing selector signature `(_:)` / `(label:)`.
                    if self.peek().kind == TokenKind::LParen {
                        self.skip_balanced_parens();
                    }
                    self.ast.append_child(node, path);
                    self.expect(TokenKind::RParen)?;
                } else if self.peek().kind == TokenKind::LParen {
                    self.skip_balanced_parens();
                }
                node
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

    /// `[a, b, ...]` array literal or `[k: v, ...]` dictionary literal (`[:]`
    /// is the empty dictionary). Children are the elements, or alternating
    /// key/value pairs for a dictionary.
    fn parse_array_or_dict(&mut self) -> Result<NodeId, ParseError> {
        let open = self.bump(); // '['
        let saved = self.no_trailing_closure;
        self.no_trailing_closure = false;
        // Empty dictionary `[:]`.
        if self.peek().kind == TokenKind::Colon
            && self.tokens[self.pos + 1].kind == TokenKind::RBracket
        {
            self.bump();
            self.bump();
            self.no_trailing_closure = saved;
            return Ok(self
                .ast
                .add(NodeKind::DictLiteral, Some("["), open.line, open.col));
        }
        // Empty array `[]`.
        if self.peek().kind == TokenKind::RBracket {
            self.bump();
            self.no_trailing_closure = saved;
            return Ok(self
                .ast
                .add(NodeKind::ArrayLiteral, Some("["), open.line, open.col));
        }
        let first = self.parse_expr(0)?;
        let is_dict = self.peek().kind == TokenKind::Colon;
        let kind = if is_dict {
            NodeKind::DictLiteral
        } else {
            NodeKind::ArrayLiteral
        };
        let node = self.ast.add(kind, Some("["), open.line, open.col);
        self.ast.append_child(node, first);
        if is_dict {
            self.bump(); // ':'
            let v = self.parse_expr(0)?;
            self.ast.append_child(node, v);
        }
        while self.peek().kind == TokenKind::Comma {
            self.bump();
            if self.peek().kind == TokenKind::RBracket {
                break; // trailing comma
            }
            let key = self.parse_expr(0)?;
            self.ast.append_child(node, key);
            if is_dict {
                self.expect(TokenKind::Colon)?;
                let v = self.parse_expr(0)?;
                self.ast.append_child(node, v);
            }
        }
        self.no_trailing_closure = saved;
        self.expect(TokenKind::RBracket)?;
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
        let (first_label, first) = self.parse_tuple_element()?;
        // `( expr )` collapses to the inner expression. A single labeled element
        // `(min: 1)` is not a one-tuple in Swift either — the label is dropped.
        if self.peek().kind != TokenKind::Comma {
            self.expect(TokenKind::RParen)?;
            return Ok(first);
        }
        let tuple = self.ast.add(NodeKind::TupleExpr, None, open.line, open.col);
        if let Some(label) = first_label {
            self.ast.set_arg_label(first, label);
        }
        self.ast.append_child(tuple, first);
        while self.peek().kind == TokenKind::Comma {
            self.bump();
            if self.peek().kind == TokenKind::RParen {
                break;
            }
            let (label, next) = self.parse_tuple_element()?;
            if let Some(label) = label {
                self.ast.set_arg_label(next, label);
            }
            self.ast.append_child(tuple, next);
        }
        self.expect(TokenKind::RParen)?;
        Ok(tuple)
    }

    /// A tuple-literal element: an optional `name:` label followed by an
    /// expression. The label uses the same `identifier :` shape as a call
    /// argument label, distinct from the `?:` ternary.
    fn parse_tuple_element(&mut self) -> Result<(Option<&'a str>, NodeId), ParseError> {
        let label = if matches!(self.peek().kind, TokenKind::Identifier | TokenKind::Keyword)
            && self.tokens[self.pos + 1].kind == TokenKind::Colon
        {
            let name = self.bump().text;
            self.bump(); // ':'
            Some(name)
        } else {
            None
        };
        let expr = self.parse_expr(0)?;
        Ok((label, expr))
    }

    /// Trailing call `(...)` and member/tuple-index `.x` / `.0` suffixes.
    /// If the current `<` begins a generic argument clause for a call or member
    /// access (`Type<Args>(…)` / `Type<Args>.member`), return the token index
    /// just past the closing `>`. Swift's heuristic: the angle group must be
    /// balanced, contain only type-like tokens, and be immediately followed by
    /// `(` (same line), `.`, or a same-line trailing-closure `{` (outside a
    /// control-flow head). This disambiguates specialization from the comparison
    /// chain `a < b > c` — which never has a matching `>` before a `{`/`(`.
    fn generic_call_args(&self) -> Option<usize> {
        let first = self.tokens.get(self.pos)?;
        if first.kind != TokenKind::Oper || !first.text.starts_with('<') {
            return None;
        }
        let mut i = self.pos;
        let mut depth = 0i32;
        loop {
            let t = self.tokens.get(i)?;
            match t.kind {
                // Only the angle operators may open/close the group. Multi-char
                // operators that merely contain `<`/`>` (`<=`, `>=`, `->`) and
                // logical/ternary operators (`&&`, `??`) disqualify the scan so
                // genuine comparison and ternary expressions are never swallowed.
                TokenKind::Oper if matches!(t.text, "?" | "&") => i += 1,
                TokenKind::Oper if t.text.chars().all(|c| c == '<' || c == '>') => {
                    for ch in t.text.chars() {
                        if ch == '<' {
                            depth += 1;
                        } else {
                            depth -= 1;
                        }
                        // Over-closing (`A<B>>(`) is not a balanced clause.
                        if depth < 0 {
                            return None;
                        }
                    }
                    i += 1;
                    if depth == 0 {
                        break;
                    }
                }
                // Type-list interior: names, qualified names, optionals,
                // protocol compositions, nested array/dictionary types, and tuples.
                TokenKind::Identifier
                | TokenKind::Question
                | TokenKind::Comma
                | TokenKind::Dot
                | TokenKind::LBracket
                | TokenKind::RBracket
                | TokenKind::Colon => i += 1,
                TokenKind::Keyword
                    if matches!(t.text, "some" | "any" | "inout" | "protocol" | "class") =>
                {
                    i += 1
                }
                _ => return None,
            }
        }
        let next = self.tokens.get(i)?;
        match next.kind {
            TokenKind::LParen if !next.leading_newline => Some(i),
            // `Type<Args> { }` — a trailing-closure call (e.g. `AsyncStream<Int>
            // { }`). Suppressed in a control-flow head, where `{` opens a block.
            TokenKind::LBrace
                if !next.leading_newline
                    && !self.no_trailing_closure
                    && !is_accessor_kw(self.tokens[i + 1].text) =>
            {
                Some(i)
            }
            TokenKind::Dot => Some(i),
            _ => None,
        }
    }

    fn parse_postfix(&mut self, mut expr: NodeId) -> Result<NodeId, ParseError> {
        loop {
            match self.peek().kind {
                // Generic specialization at a call/member site: `Type<Args>(…)`
                // or `Type<Args>.member`. The runtime infers type arguments
                // from values, so the `<…>` clause is skipped here.
                TokenKind::Oper
                    if self.peek().text.starts_with('<')
                        && matches!(
                            self.ast.node(expr).kind(),
                            NodeKind::IdentExpr | NodeKind::MemberExpr
                        ) =>
                {
                    match self.generic_call_args() {
                        Some(end) => {
                            // Most generics infer their arguments from values, so
                            // the `<…>` clause is discarded. `MemoryLayout<T>`,
                            // however, is parameterised purely by a written type,
                            // so record that type as a `TypeIdent` child of the
                            // `MemoryLayout` identifier for the runtime to read.
                            if self.ast.node(expr).kind() == NodeKind::IdentExpr
                                && self.ast.node(expr).text() == Some("MemoryLayout")
                            {
                                let inner: String = self.tokens[self.pos..end]
                                    .iter()
                                    .map(|t| t.text)
                                    .collect::<String>()
                                    .trim()
                                    .trim_start_matches('<')
                                    .trim_end_matches('>')
                                    .trim()
                                    .to_string();
                                let tok = self.peek();
                                let ty = self.ast.add(
                                    NodeKind::TypeRef,
                                    Some(&inner),
                                    tok.line,
                                    tok.col,
                                );
                                self.ast.append_child(expr, ty);
                            }
                            self.pos = end;
                        }
                        None => break,
                    }
                }
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
                // control-flow head) attaches a closure argument to a call. A
                // `{` introducing accessor keywords (`get`/`set`/`willSet`/
                // `didSet`) is a property accessor block, not a closure.
                TokenKind::LBrace
                    if !self.no_trailing_closure
                        && !self.peek().leading_newline
                        && !is_accessor_kw(self.tokens[self.pos + 1].text) =>
                {
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
                // A call argument list must begin on the same line as the
                // callee; a `(` after a newline starts a new (parenthesized /
                // tuple) statement, e.g. `var b = 1` then `(a, b) = (b, a + b)`.
                TokenKind::LParen if !self.peek().leading_newline => {
                    let open = self.bump();
                    let call = self.ast.add(NodeKind::CallExpr, None, open.line, open.col);
                    self.ast.append_child(call, expr);
                    let saved = self.no_trailing_closure;
                    self.no_trailing_closure = false;
                    if self.peek().kind != TokenKind::RParen {
                        loop {
                            // Argument label `name:` (an identifier or keyword
                            // followed by `:`, distinct from the `?:` ternary).
                            let label = if matches!(
                                self.peek().kind,
                                TokenKind::Identifier | TokenKind::Keyword
                            ) && self.tokens[self.pos + 1].kind == TokenKind::Colon
                            {
                                let name = self.bump().text;
                                self.bump(); // ':'
                                Some(name)
                            } else {
                                None
                            };
                            let arg = self.parse_expr(0)?;
                            if let Some(label) = label {
                                self.ast.set_arg_label(arg, label);
                            }
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
                // Subscript access `base[index, ...]` (same line as the base).
                TokenKind::LBracket if !self.peek().leading_newline => {
                    let open = self.bump();
                    let sub = self
                        .ast
                        .add(NodeKind::SubscriptExpr, Some("["), open.line, open.col);
                    self.ast.append_child(sub, expr);
                    let saved = self.no_trailing_closure;
                    self.no_trailing_closure = false;
                    if self.peek().kind != TokenKind::RBracket {
                        loop {
                            // Optional subscript argument label `name:` (`m[tag: x]`).
                            let label = if matches!(
                                self.peek().kind,
                                TokenKind::Identifier | TokenKind::Keyword
                            ) && self.tokens[self.pos + 1].kind == TokenKind::Colon
                            {
                                let name = self.bump().text;
                                self.bump(); // ':'
                                Some(name)
                            } else {
                                None
                            };
                            let idx = self.parse_expr(0)?;
                            if let Some(label) = label {
                                self.ast.set_arg_label(idx, label);
                            }
                            self.ast.append_child(sub, idx);
                            if self.peek().kind == TokenKind::Comma {
                                self.bump();
                                continue;
                            }
                            break;
                        }
                    }
                    self.no_trailing_closure = saved;
                    self.expect(TokenKind::RBracket)?;
                    expr = sub;
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
const TERNARY_BP: u8 = 6;

/// Right binding power of the range operators (`..<` / `...`, precedence 13).
/// Parsing a range operand at this power stops before another range operator,
/// leaving it for one-sided / two-sided range-pattern handling.
const RANGE_RBP: u8 = 14;

/// Precedence of `is`/`as` casts (Swift `CastingPrecedence`, /10).
const CAST_BP: u8 = 13;

/// Swift's `DefaultPrecedence`, used for operators declared without a group.
const DEFAULT_BP: u8 = 14;

/// `(left_bp, right_bp)` for a built-in infix operator, encoding precedence and
/// associativity (`right_bp < left_bp` ⇒ right-associative). `None` for tokens
/// that are not infix operators. Values mirror Swift's standard precedence
/// groups (divided by 10). User-declared operators are resolved separately by
/// [`Parser::binding_power`] before this fallback is consulted.
fn builtin_binding_power(op: &str) -> Option<(u8, u8)> {
    let p = match op {
        "<<" | ">>" | "&<<" | "&>>" => 16,
        "*" | "/" | "%" | "&" | "&*" => 15,
        "+" | "-" | "|" | "^" | "&+" | "&-" => 14,
        "..<" | "..." => 13,
        "??" => return Some((12, 11)), // right-associative
        "==" | "!=" | "<" | ">" | "<=" | ">=" | "===" | "!==" => 9,
        "&&" => 8,
        "||" => 7,
        // An undeclared custom operator: parse it as a left-associative infix at
        // the default precedence so the runtime can still dispatch it.
        _ => DEFAULT_BP,
    };
    Some((p, p + 1))
}

/// Numeric precedence and associativity of a built-in precedence group, mirroring
/// the Swift standard library's group ordering (divided by 10).
fn builtin_group_precedence(name: &str) -> Option<(u8, bool)> {
    Some(match name {
        "BitwiseShiftPrecedence" => (16, false),
        "MultiplicationPrecedence" => (15, false),
        "AdditionPrecedence" => (14, false),
        "RangeFormationPrecedence" => (13, false),
        "CastingPrecedence" => (13, false),
        "NilCoalescingPrecedence" => (12, true),
        "ComparisonPrecedence" => (9, false),
        "LogicalConjunctionPrecedence" => (8, false),
        "LogicalDisjunctionPrecedence" => (7, false),
        "DefaultPrecedence" => (DEFAULT_BP, false),
        "TernaryPrecedence" => (6, true),
        "AssignmentPrecedence" => (5, true),
        _ => return None,
    })
}

/// Resolve a (possibly custom) precedence group to `(precedence, right_assoc)`,
/// following `higherThan`/`lowerThan` relations to a built-in anchor. Memoised,
/// and self-referential cycles fall back to `DefaultPrecedence`.
fn resolve_group_precedence(
    name: &str,
    groups: &HashMap<String, RawPrecedenceGroup>,
    memo: &mut HashMap<String, (u8, bool)>,
) -> (u8, bool) {
    if let Some(builtin) = builtin_group_precedence(name) {
        return builtin;
    }
    if let Some(&cached) = memo.get(name) {
        return cached;
    }
    // Provisional entry guards against cyclic `higherThan`/`lowerThan` chains.
    memo.insert(name.to_string(), (DEFAULT_BP, false));
    let resolved = match groups.get(name) {
        Some(group) => {
            let right = group.right_associative;
            if let Some(higher) = &group.higher_than {
                let (p, _) = resolve_group_precedence(higher, groups, memo);
                (p.saturating_add(1), right)
            } else if let Some(lower) = &group.lower_than {
                let (p, _) = resolve_group_precedence(lower, groups, memo);
                (p.saturating_sub(1), right)
            } else {
                (DEFAULT_BP, right)
            }
        }
        None => (DEFAULT_BP, false),
    };
    memo.insert(name.to_string(), resolved);
    resolved
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
            // Ownership method modifiers (`consuming func`, `borrowing func`).
            | "consuming"
            | "borrowing"
            // Operator fixity words form a modifier run before `func` (`prefix
            // func -`); a bare `prefix operator …` is handled separately and is
            // not treated as a modifier because `operator` is not a decl keyword.
            | "prefix"
            | "postfix"
            | "infix"
            // `async` only forms a leading modifier run before `let`/`var`
            // (an `async let` binding); the post-params effect is handled
            // separately by `skip_effects`.
            | "async"
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
            // `actor` is a contextual keyword; `import` introduces a module
            // declaration. Both may carry leading attributes
            // (`@globalActor actor …`, `@preconcurrency import …`).
            | "actor"
            | "import"
    )
}

/// Accessor introducers inside a property/subscript body.
fn is_accessor_kw(w: &str) -> bool {
    matches!(w, "get" | "set" | "willSet" | "didSet")
}

/// An accessor block entry may start with a `mutating`/`nonmutating` modifier
/// before the accessor keyword.
fn is_accessor_start(w: &str) -> bool {
    is_accessor_kw(w) || matches!(w, "mutating" | "nonmutating")
}

/// Declaration kinds that may follow `import` to import a single symbol.
fn is_import_kind(w: &str) -> bool {
    matches!(
        w,
        "typealias" | "struct" | "class" | "enum" | "protocol" | "let" | "var" | "func"
    )
}

/// Whether `kw` is a statement that may carry a leading `label:`.
fn is_labelable(kw: &str) -> bool {
    matches!(kw, "for" | "while" | "repeat" | "switch")
}

/// Whether `op` is a plain or compound assignment operator.
fn is_assignment(op: &str) -> bool {
    matches!(
        op,
        "=" | "+="
            | "-="
            | "*="
            | "/="
            | "%="
            | "&="
            | "|="
            | "^="
            | "<<="
            | ">>="
            // Overflow-wrapping compound assignments.
            | "&+="
            | "&-="
            | "&*="
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
    fn first_stmt(ast: &Ast) -> tswift_ast::Node<'_> {
        ast.node(ast.root()).children().next().unwrap()
    }

    #[test]
    fn closure_in_label_is_not_mistaken_for_the_in_separator() {
        // A nested call's `in:` argument label inside a trailing closure must not
        // be parsed as the closure's `in` parameter separator.
        ast_of("f { Slider(value: x, in: 0...1) }");
        // A genuine closure signature still parses.
        ast_of("let c = { x, y in x + y }");
        ast_of("let c = { (a: Int) in a }");
    }

    #[test]
    fn parses_nonmutating_accessor_modifier() {
        // `nonmutating set` parses and the modifier lands on the Accessor node.
        let ast = ast_of(
            "struct B { let box: Box \n var v: Int { get { box.value } nonmutating set { box.value = newValue } } }",
        );
        fn has_nonmutating_setter(node: tswift_ast::Node<'_>) -> bool {
            if node.kind() == NodeKind::Accessor
                && node.text().as_deref() == Some("set")
                && node.modifiers().iter().any(|m| m == "nonmutating")
            {
                return true;
            }
            node.children().any(has_nonmutating_setter)
        }
        assert!(
            has_nonmutating_setter(ast.node(ast.root())),
            "set accessor should record the nonmutating modifier"
        );
    }

    #[test]
    fn parses_mutating_accessor_modifier() {
        // `mutating set` is also accepted (and distinguishable).
        ast_of("struct B { var v: Int { get { 0 } mutating set { } } }");
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
    fn parses_regex_literal() {
        assert_eq!(
            dump("let r = /\\d+/"),
            "source_file L1\n  \
             let_decl L1\n    \
             name_pattern \"r\" L1\n    \
             regex_literal \"/\\\\d+/\" L1\n"
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
    fn labeled_tuple_literal_keeps_element_labels() {
        let ast = ast_of("let p = (min: 1, max: 9)");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::TupleExpr);
        let labels: Vec<_> = init.children().map(|c| c.arg_label()).collect();
        assert_eq!(labels, vec![Some("min"), Some("max")]);
    }

    #[test]
    fn single_labeled_paren_collapses_to_inner_expr() {
        // `(min: 1)` is not a one-element tuple in Swift; the label is dropped.
        let ast = ast_of("let x = (min: 1)");
        let init = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(init.kind(), NodeKind::IntegerLiteral);
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
    fn binary_condition_binds_before_ternary() {
        let ast = ast_of("n == 0 ? 1 : 2");
        let tern = first_stmt(&ast).children().next().unwrap();
        assert_eq!(tern.kind(), NodeKind::TernaryExpr);
        assert_eq!(tern.children().next().unwrap().kind(), NodeKind::BinaryExpr);
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
    fn unknown_default_parses_as_clause() {
        let src = "switch s {\n\
                   case .ok: break\n\
                   @unknown default: break\n\
                   }";
        let ast = ast_of(src);
        let sw = first_stmt(&ast);
        let clauses: Vec<_> = sw
            .children()
            .filter(|c| c.kind() == NodeKind::CaseClause)
            .collect();
        assert_eq!(clauses.len(), 2);
        assert_eq!(clauses[1].text(), Some("default"));
    }

    #[test]
    fn attribute_in_case_body_is_not_a_clause_boundary() {
        // A `@discardableResult func` declaration inside a case body must stay
        // part of that body, not be misread as the start of the next clause.
        let src = "switch n {\n\
                   case 0:\n\
                   @discardableResult func f() -> Int { return 1 }\n\
                   f()\n\
                   default: break\n\
                   }";
        let ast = ast_of(src);
        let sw = first_stmt(&ast);
        let clauses: Vec<_> = sw
            .children()
            .filter(|c| c.kind() == NodeKind::CaseClause)
            .collect();
        // Two clauses only: the attributed func did not open a third clause.
        assert_eq!(clauses.len(), 2);
        let body = clauses[0].children().last().unwrap();
        assert_eq!(body.kind(), NodeKind::Block);
        // The body holds the func declaration plus the call statement.
        assert!(body.children().any(|c| c.kind() == NodeKind::FuncDecl));
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
    fn objc_optional_protocol_requirements_parse() {
        let src = "@objc protocol Delegate {\n  @objc optional func willLoad()\n  @objc optional var badge: Int { get }\n  @objc optional subscript(i: Int) -> Int { get }\n}";
        let ast = ast_of(src);
        let proto = first_stmt(&ast);
        assert_eq!(proto.kind(), NodeKind::ProtocolDecl);
        let members: Vec<_> = proto
            .children()
            .filter(|node| {
                matches!(
                    node.kind(),
                    NodeKind::FuncDecl | NodeKind::VarDecl | NodeKind::SubscriptDecl
                )
            })
            .collect();
        assert_eq!(members[0].kind(), NodeKind::FuncDecl);
        assert!(members[0].modifiers().iter().any(|m| m == "optional"));
        assert_eq!(members[1].kind(), NodeKind::VarDecl);
        assert!(members[1].modifiers().iter().any(|m| m == "optional"));
        assert_eq!(members[2].kind(), NodeKind::SubscriptDecl);
        assert!(members[2].modifiers().iter().any(|m| m == "optional"));
    }

    #[test]
    fn optional_without_objc_stays_an_identifier() {
        assert!(parse("optional func f() {}").is_err());
        assert!(parse("protocol P { optional func f() }").is_err());
        assert!(parse("@objc optional let x = 1").is_err());
        let ast = ast_of("func optional() {}\nvar optional = 1");
        let stmts: Vec<_> = ast.node(ast.root()).children().collect();
        assert_eq!(stmts[0].text(), Some("optional"));
        assert_eq!(stmts[1].children().next().unwrap().text(), Some("optional"));
    }

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
    fn generic_subscript() {
        // A `<T>` clause and a `where` constraint on a subscript parse into a
        // `GenericParam` child plus the usual params/return/accessors.
        let src = "struct Box {\n\
                   var items: [Int]\n\
                   subscript<T>(map f: (Int) -> T) -> [T] where T: P { return items.map(f) }\n\
                   }";
        let ast = ast_of(src);
        let s = first_stmt(&ast);
        let sub = s
            .children()
            .find(|c| c.kind() == NodeKind::SubscriptDecl)
            .expect("subscript decl");
        assert_eq!(
            sub.children().next().unwrap().kind(),
            NodeKind::GenericParam
        );
        // The parameter and accessor still parse after the generic clause.
        let kinds: Vec<_> = sub.children().map(|c| c.kind()).collect();
        assert!(kinds.contains(&NodeKind::Param));
    }

    #[test]
    fn key_path_expression() {
        // `\Person.name` → KeyPathExpr with a TypeRef root and one IdentExpr.
        let ast = ast_of("let k = \\Person.address.city");
        let decl = first_stmt(&ast);
        let kp = decl
            .children()
            .find(|c| c.kind() == NodeKind::KeyPathExpr)
            .expect("key-path expr");
        let kinds: Vec<_> = kp.children().map(|c| c.kind()).collect();
        assert_eq!(
            kinds,
            vec![NodeKind::TypeRef, NodeKind::IdentExpr, NodeKind::IdentExpr]
        );
        let comps: Vec<_> = kp
            .children()
            .filter(|c| c.kind() == NodeKind::IdentExpr)
            .filter_map(|c| c.text())
            .collect();
        assert_eq!(comps, vec!["address", "city"]);
    }

    #[test]
    fn inferred_root_key_path() {
        // `\.count` omits the root type: no leading TypeRef child.
        let ast = ast_of("let k = \\.count");
        let decl = first_stmt(&ast);
        let kp = decl
            .children()
            .find(|c| c.kind() == NodeKind::KeyPathExpr)
            .expect("key-path expr");
        let kinds: Vec<_> = kp.children().map(|c| c.kind()).collect();
        assert_eq!(kinds, vec![NodeKind::IdentExpr]);
    }

    #[test]
    fn memory_layout_records_type_argument() {
        // `MemoryLayout<Int>.size` records the written type `Int` as a `TypeRef`
        // child of the `MemoryLayout` identifier, so the runtime can read it.
        let ast = ast_of("let s = MemoryLayout<Int>.size");
        let decl = first_stmt(&ast);
        // let s = <member>.size  →  MemberExpr "size" over IdentExpr "MemoryLayout".
        let init = decl
            .children()
            .find(|c| c.kind() == NodeKind::MemberExpr)
            .expect("member expr initializer");
        let base = init.children().next().expect("member base");
        assert_eq!(base.kind(), NodeKind::IdentExpr);
        assert_eq!(base.text(), Some("MemoryLayout"));
        let ty = base.children().next().expect("recorded type argument");
        assert_eq!(ty.kind(), NodeKind::TypeRef);
        assert_eq!(ty.text(), Some("Int"));
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
    fn closure_signature_accepts_inout_and_throws() {
        // `throws` after the parameter list no longer aborts signature parsing;
        // the `inout` parameter is still recorded.
        let ast = ast_of("let g = { (n: inout Int) throws in n += 1 }");
        let clo = first_stmt(&ast).children().nth(1).unwrap();
        assert_eq!(clo.kind(), NodeKind::ClosureExpr);
        let param = clo
            .children()
            .find(|c| c.kind() == NodeKind::Param)
            .expect("closure has a Param");
        assert_eq!(param.text(), Some("n"));
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
    fn generic_specialization_rejects_expression_keywords() {
        // `self` is a value expression, not a type-argument keyword. The angle
        // scanner must leave this as comparison syntax instead of swallowing
        // `< self.x >` as a generic-argument list.
        let ast = ast_of(
            "struct S { var x: Int\n func f(_ a: Int, _ c: Int) { let y = a < self.x > (c) } }",
        );
        assert_eq!(first_stmt(&ast).kind(), NodeKind::StructDecl);
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
        // The runtime-facing payload is the bare variant marker (`?`), not `try?`.
        assert_eq!(init.text(), Some("?"));
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

    // --- Tier 2 nominal: semicolons, modifiers, attributes, arg labels ---

    #[test]
    fn semicolons_separate_statements_and_members() {
        // Semicolons are accepted as statement separators at top level, inside
        // blocks, and inside a type body.
        let ast = ast_of("let a = 1; let b = 2;\nstruct P { var x: Int; var y: Int }");
        let top: Vec<_> = ast.node(ast.root()).children().collect();
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].kind(), NodeKind::LetDecl);
        assert_eq!(top[1].kind(), NodeKind::LetDecl);
        let strukt = top[2];
        assert_eq!(strukt.kind(), NodeKind::StructDecl);
        let members: Vec<_> = strukt
            .children()
            .filter(|c| c.kind() == NodeKind::VarDecl)
            .collect();
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn enum_cases_separated_by_semicolons() {
        let ast = ast_of("enum E { case a; case b }");
        let e = first_stmt(&ast);
        assert_eq!(e.kind(), NodeKind::EnumDecl);
        let cases: Vec<_> = e
            .children()
            .filter(|c| c.kind() == NodeKind::EnumCaseDecl)
            .collect();
        assert_eq!(cases.len(), 2);
    }

    #[test]
    fn modifiers_are_recorded_on_declarations() {
        let ast = ast_of("struct S {\n  static let n = 1\n  mutating func go() {}\n}");
        let s = first_stmt(&ast);
        let members: Vec<_> = s.children().collect();
        let n = members
            .iter()
            .find(|c| c.kind() == NodeKind::LetDecl)
            .unwrap();
        assert_eq!(n.modifiers(), &["static".to_string()]);
        let go = members
            .iter()
            .find(|c| c.kind() == NodeKind::FuncDecl)
            .unwrap();
        assert_eq!(go.modifiers(), &["mutating".to_string()]);
    }

    #[test]
    fn attributes_become_child_nodes() {
        let ast = ast_of("@main\nstruct App { }");
        let app = first_stmt(&ast);
        assert_eq!(app.kind(), NodeKind::StructDecl);
        let attr = app
            .children()
            .find(|c| c.kind() == NodeKind::Attribute)
            .expect("@main attribute child");
        // The leading `@` is stripped, matching the runtime-facing payload.
        assert_eq!(attr.text(), Some("main"));
    }

    #[test]
    fn calls_subscripts_inout_variadic_operators_parse() {
        // Array literal, subscript expr, inout arg, variadic param, custom op.
        let ast = ast_of(
            "let a = [1, 2, 3]\n\
             let x = a[0]\n\
             func sum(_ xs: Int...) -> Int { return xs.count }\n\
             func bump(_ n: inout Int) { n += 1 }\n\
             bump(&count)\n\
             func ^^ (l: Int, r: Int) -> Int { return l }\n\
             let y = 2 ^^ 8",
        );
        let top: Vec<_> = ast.node(ast.root()).children().collect();
        // Array literal initializer.
        assert_eq!(
            top[0].children().last().unwrap().kind(),
            NodeKind::ArrayLiteral
        );
        // Subscript expression.
        assert_eq!(
            top[1].children().last().unwrap().kind(),
            NodeKind::SubscriptExpr
        );
        // Variadic parameter modifier.
        let xs = top[2].children().next().unwrap();
        assert!(xs.modifiers().contains(&"variadic".to_string()));
        // inout parameter modifier.
        let n = top[3].children().next().unwrap();
        assert!(n.modifiers().contains(&"inout".to_string()));
        // `&count` lowers to an InoutExpr argument.
        let call = top[4].children().next().unwrap();
        assert_eq!(call.children().nth(1).unwrap().kind(), NodeKind::InoutExpr);
        // Custom operator function name spans both `^` tokens.
        assert_eq!(top[5].text(), Some("^^"));
        // `2 ^^ 8` parses as a binary expression with the custom operator.
        let bin = top[6].children().last().unwrap();
        assert_eq!(bin.kind(), NodeKind::BinaryExpr);
        assert_eq!(bin.text(), Some("^^"));
    }

    #[test]
    fn computed_property_and_observers_parse_accessors() {
        let ast = ast_of(
            "struct S {\n\
             var stored: Int = 0\n\
             var computed: Int { return stored * 2 }\n\
             var watched: Int = 0 { willSet { } didSet { } }\n\
             }",
        );
        // In the raw AST the binding name is a NamePattern child of the VarDecl.
        let binding_name = |v: &tswift_ast::Node<'_>| -> Option<String> {
            v.children()
                .find(|c| c.kind() == NodeKind::NamePattern)
                .and_then(|c| c.text())
                .map(str::to_string)
        };
        let members: Vec<_> = first_stmt(&ast).children().collect();
        let computed = members
            .iter()
            .find(|c| binding_name(c).as_deref() == Some("computed"))
            .unwrap();
        assert!(computed
            .children()
            .any(|c| c.kind() == NodeKind::Accessor && c.text() == Some("get")));
        let watched = members
            .iter()
            .find(|c| binding_name(c).as_deref() == Some("watched"))
            .unwrap();
        let accs: Vec<_> = watched
            .children()
            .filter(|c| c.kind() == NodeKind::Accessor)
            .filter_map(|c| c.text())
            .collect();
        assert!(accs.contains(&"willSet"));
        assert!(accs.contains(&"didSet"));
    }

    #[test]
    fn concurrency_syntax_parses() {
        // `actor` decl, `async` effect, `await` expr, `async let`, `for await`.
        let ast = ast_of(
            "actor Counter { var n = 0 }\n\
             func run() async {\n\
             async let a = fetch()\n\
             let v = await a\n\
             for await x in stream { use(x) }\n\
             }",
        );
        let top: Vec<_> = ast.node(ast.root()).children().collect();
        assert_eq!(top[0].kind(), NodeKind::ActorDecl);
        let body = top[1]
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap();
        let stmts: Vec<_> = body.children().collect();
        // `async let a` carries the async modifier.
        assert_eq!(stmts[0].modifiers(), &["async".to_string()]);
        // `await a` is an AwaitExpr wrapping its operand.
        let v_init = stmts[1].children().last().unwrap();
        assert_eq!(v_init.kind(), NodeKind::AwaitExpr);
        // `for await x` carries the async modifier and binds `x`.
        let for_stmt = stmts[2];
        assert_eq!(for_stmt.kind(), NodeKind::ForStmt);
        assert_eq!(for_stmt.modifiers(), &["async".to_string()]);
    }

    #[test]
    fn generic_type_with_trailing_closure_is_a_call() {
        // `AsyncStream<Int> { … }` is a specialization + trailing-closure call,
        // not the comparison chain `AsyncStream < Int > { … }`.
        let ast = ast_of("let s = AsyncStream<Int> { c in c.finish() }");
        let init = ast
            .node(ast.root())
            .children()
            .next()
            .unwrap()
            .children()
            .last()
            .unwrap();
        assert_eq!(init.kind(), NodeKind::CallExpr);
        let callee = init.children().next().unwrap();
        assert_eq!(callee.kind(), NodeKind::IdentExpr);
        assert_eq!(callee.text().as_deref(), Some("AsyncStream"));
    }

    #[test]
    fn for_try_await_carries_async_and_throws() {
        // `for try await` over a throwing async sequence carries both effects.
        let ast = ast_of(
            "func run() async {\n\
             for try await x in stream { use(x) }\n\
             }",
        );
        let body = ast
            .node(ast.root())
            .children()
            .next()
            .unwrap()
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap();
        let for_stmt = body.children().next().unwrap();
        assert_eq!(for_stmt.kind(), NodeKind::ForStmt);
        assert_eq!(
            for_stmt.modifiers(),
            &["throws".to_string(), "async".to_string()]
        );
    }

    #[test]
    fn switch_case_patterns_lower_to_pattern_nodes() {
        let ast = ast_of(
            "switch s {\n\
             case .circle(let r): break\n\
             case (let x, 0): break\n\
             case .none: break\n\
             case 0...9 where x > 1: break\n\
             default: break\n\
             }",
        );
        let sw = first_stmt(&ast);
        let clauses: Vec<_> = sw
            .children()
            .filter(|c| c.kind() == NodeKind::CaseClause)
            .collect();
        // .circle(let r) -> EnumCasePattern with a NamePattern child.
        let enum_pat = clauses[0].children().next().unwrap();
        assert_eq!(enum_pat.kind(), NodeKind::EnumCasePattern);
        assert_eq!(enum_pat.text(), Some("circle"));
        assert_eq!(
            enum_pat.children().next().unwrap().kind(),
            NodeKind::NamePattern
        );
        // (let x, 0) -> TuplePattern: NamePattern + value pattern.
        let tuple_pat = clauses[1].children().next().unwrap();
        assert_eq!(tuple_pat.kind(), NodeKind::TuplePattern);
        let subs: Vec<_> = tuple_pat.children().map(|c| c.kind()).collect();
        assert_eq!(subs[0], NodeKind::NamePattern);
        assert_eq!(subs[1], NodeKind::IntegerLiteral);
        // .none -> EnumCasePattern with no children.
        let none_pat = clauses[2].children().next().unwrap();
        assert_eq!(none_pat.kind(), NodeKind::EnumCasePattern);
        assert_eq!(none_pat.text(), Some("none"));
        // 0...9 where x > 1 -> RangePattern + a WhereClause child.
        assert_eq!(
            clauses[3].children().next().unwrap().kind(),
            NodeKind::RangePattern
        );
        assert!(clauses[3]
            .children()
            .any(|c| c.kind() == NodeKind::WhereClause));
        // default -> labelled clause with only a body block.
        assert_eq!(clauses[4].text(), Some("default"));
    }

    #[test]
    fn async_let_records_async_modifier() {
        let ast = ast_of("func f() {\n  async let a = g()\n  let b = 1\n}");
        let body = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap();
        let lets: Vec<_> = body
            .children()
            .filter(|c| c.kind() == NodeKind::LetDecl)
            .collect();
        assert_eq!(lets[0].modifiers(), &["async".to_string()]);
        assert!(lets[1].modifiers().is_empty());
    }

    #[test]
    fn call_argument_labels_are_recorded() {
        let ast = ast_of("f(x: 1, 2, y: 3)");
        let call = first_stmt(&ast).children().next().unwrap();
        assert_eq!(call.kind(), NodeKind::CallExpr);
        let args: Vec<_> = call.children().skip(1).collect();
        assert_eq!(args[0].arg_label(), Some("x"));
        assert_eq!(args[1].arg_label(), None);
        assert_eq!(args[2].arg_label(), Some("y"));
    }

    #[test]
    fn consecutive_statements_on_one_line_are_rejected() {
        let err = parse("let x = 1 let y = 2").unwrap_err();
        assert!(err.message.contains("separated by ';'"), "{}", err.message);
        // A newline or `;` between statements is accepted.
        assert!(parse("let x = 1\nlet y = 2").is_ok());
        assert!(parse("let x = 1; let y = 2").is_ok());
    }

    #[test]
    fn custom_operator_uses_its_precedence_group() {
        // `**` (ExponentPrecedence, higher than `*`) binds tighter than `*`,
        // so `2 * 3 ** 2` parses as `2 * (3 ** 2)`.
        let src = "precedencegroup ExpPrec { higherThan: MultiplicationPrecedence }\n\
                   infix operator ** : ExpPrec\n\
                   let r = 2 * 3 ** 2";
        let ast = ast_of(src);
        let decl = ast.node(ast.root()).children().last().unwrap();
        let top = decl.children().last().unwrap();
        assert_eq!(top.text(), Some("*"));
        assert_eq!(top.children().nth(1).unwrap().text(), Some("**"));
    }

    #[test]
    fn one_sided_range_prefix_parses() {
        let ast = ast_of("let r = ..<5");
        let expr = first_stmt(&ast).children().last().unwrap();
        assert_eq!(expr.kind(), NodeKind::PrefixExpr);
        assert_eq!(expr.text(), Some("..<"));
    }

    #[test]
    fn one_sided_range_patterns_in_switch() {
        // `case n...:` (from), `case ..<n:` (upTo), `case ...n:` (through) each
        // lower to a single-bound RangePattern tagged by direction.
        let ast = ast_of(
            "switch x {\ncase 90...: break\ncase ..<60: break\ncase ...0: break\ndefault: break\n}",
        );
        let switch_stmt = first_stmt(&ast);
        let clauses: Vec<_> = switch_stmt
            .children()
            .filter(|c| c.kind() == NodeKind::CaseClause)
            .collect();
        let from = clauses[0].children().next().unwrap();
        assert_eq!(from.kind(), NodeKind::RangePattern);
        assert_eq!(from.text(), Some("from"));
        assert_eq!(from.children().count(), 1);
        let upto = clauses[1].children().next().unwrap();
        assert_eq!(upto.text(), Some("upTo"));
        assert_eq!(upto.children().count(), 1);
        let through = clauses[2].children().next().unwrap();
        assert_eq!(through.text(), Some("through"));
        assert_eq!(through.children().count(), 1);
    }

    #[test]
    fn two_sided_range_pattern_keeps_two_bounds() {
        let ast = ast_of("switch x {\ncase 80..<90: break\ndefault: break\n}");
        let clause = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::CaseClause)
            .unwrap();
        let pat = clause.children().next().unwrap();
        assert_eq!(pat.kind(), NodeKind::RangePattern);
        assert_eq!(pat.text(), Some("..<"));
        assert_eq!(pat.children().count(), 2);
    }

    #[test]
    fn multi_name_binding_shares_type_annotation() {
        // `var a, b, c: Double` desugars to three VarDecls, each with its own
        // NamePattern + a Double type annotation.
        let ast = ast_of("var a, b, c: Double");
        let decls: Vec<_> = ast
            .node(ast.root())
            .children()
            .filter(|c| c.kind() == NodeKind::VarDecl)
            .collect();
        assert_eq!(decls.len(), 3);
        for decl in &decls {
            let kids: Vec<_> = decl.children().collect();
            assert_eq!(kids[0].kind(), NodeKind::NamePattern);
            assert_eq!(kids[1].kind(), NodeKind::TypeRef);
            assert_eq!(kids[1].text(), Some("Double"));
        }
    }

    #[test]
    fn multi_name_binding_with_initializers() {
        // `var x = 1, y = 2` desugars to two VarDecls, each with its own init.
        let ast = ast_of("var x = 1, y = 2");
        let decls: Vec<_> = ast
            .node(ast.root())
            .children()
            .filter(|c| c.kind() == NodeKind::VarDecl)
            .collect();
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].children().next().unwrap().text(), Some("x"));
        assert_eq!(decls[1].children().next().unwrap().text(), Some("y"));
    }

    #[test]
    fn single_binding_keeps_original_shape() {
        let ast = ast_of("let x: Int = 1");
        let decls: Vec<_> = ast
            .node(ast.root())
            .children()
            .filter(|c| c.kind() == NodeKind::LetDecl)
            .collect();
        assert_eq!(decls.len(), 1);
        let kids: Vec<_> = decls[0].children().collect();
        assert_eq!(kids.len(), 3);
        assert_eq!(kids[0].kind(), NodeKind::NamePattern);
        assert_eq!(kids[1].kind(), NodeKind::TypeRef);
    }

    #[test]
    fn paren_on_next_line_is_not_a_call() {
        // `var b = 1` then `(a, b) = (b, a + b)`: the `(` after the newline
        // starts a tuple-assignment statement, not a call `1(a, b)`.
        let ast = ast_of("var b = 1\n(a, b) = (b, b + 1)");
        let stmts: Vec<_> = ast.node(ast.root()).children().collect();
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0].kind(), NodeKind::VarDecl);
        assert_eq!(stmts[1].kind(), NodeKind::AssignExpr);
        assert_eq!(
            stmts[1].children().next().unwrap().kind(),
            NodeKind::TupleExpr
        );
    }

    #[test]
    fn same_line_paren_is_still_a_call() {
        let ast = ast_of("f(1, 2)");
        let call = first_stmt(&ast).children().next().unwrap();
        assert_eq!(call.kind(), NodeKind::CallExpr);
    }

    #[test]
    fn bare_operator_is_an_argument_reference() {
        let ast = ast_of("xs.reduce(0, +)");
        let call = first_stmt(&ast).children().next().unwrap();
        let last = call.children().last().unwrap();
        assert_eq!(last.kind(), NodeKind::IdentExpr);
        assert_eq!(last.text(), Some("+"));
    }

    #[test]
    fn tuple_return_type_keeps_element_labels() {
        let ast = ast_of("func f() -> (min: Int, max: Int) { (0, 0) }");
        let ret = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::TypeRef)
            .unwrap();
        assert_eq!(ret.text(), Some("(min: Int, max: Int)"));
    }

    #[test]
    fn subscript_argument_label_is_recorded() {
        let ast = ast_of(r#"Lookup[tag: "x"]"#);
        let sub = first_stmt(&ast).children().next().unwrap();
        assert_eq!(sub.kind(), NodeKind::SubscriptExpr);
        assert_eq!(sub.children().nth(1).unwrap().arg_label(), Some("tag"));
    }

    #[test]
    fn condition_list_parses_multiple_optional_bindings() {
        let ast = ast_of("if let a = a, let b = b, a < b { print(a) }");
        let if_stmt = first_stmt(&ast);
        let conds: Vec<_> = if_stmt
            .children()
            .take_while(|c| c.kind() != NodeKind::Block)
            .collect();
        assert_eq!(conds.len(), 3);
        assert_eq!(conds[0].kind(), NodeKind::LetDecl);
        assert_eq!(conds[1].kind(), NodeKind::LetDecl);
        assert_eq!(conds[2].kind(), NodeKind::BinaryExpr);
    }

    #[test]
    fn guard_condition_mixes_binding_and_boolean() {
        let ast = ast_of("func f() { guard let x = x, x > 0 else { return } }");
        let body = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap();
        let guard = body
            .children()
            .find(|c| c.kind() == NodeKind::GuardStmt)
            .unwrap();
        let conds: Vec<_> = guard
            .children()
            .take_while(|c| c.kind() != NodeKind::Block)
            .collect();
        assert_eq!(conds.len(), 2);
        assert_eq!(conds[0].kind(), NodeKind::LetDecl);
        assert_eq!(conds[1].kind(), NodeKind::BinaryExpr);
    }

    #[test]
    fn catch_binding_with_as_cast_is_a_cast_pattern() {
        let ast = ast_of("do { } catch let e as MyError { }");
        let do_stmt = first_stmt(&ast);
        let clause = do_stmt
            .children()
            .find(|c| c.kind() == NodeKind::CatchClause)
            .unwrap();
        let pat = clause.children().next().unwrap();
        assert_eq!(pat.kind(), NodeKind::CastExpr);
        assert_eq!(pat.text(), Some("as"));
        let inner = pat.children().next().unwrap();
        assert_eq!(inner.kind(), NodeKind::NamePattern);
        assert_eq!(inner.text(), Some("e"));
    }

    #[test]
    fn switch_is_pattern_is_a_cast_pattern() {
        let ast = ast_of("switch v { case is String: break; default: break }");
        let case = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::CaseClause)
            .unwrap();
        let pat = case.children().next().unwrap();
        assert_eq!(pat.kind(), NodeKind::CastExpr);
        assert_eq!(pat.text(), Some("is"));
    }

    #[test]
    fn switch_optional_shorthand_is_a_some_pattern() {
        let ast = ast_of("switch v { case let x?: print(x); default: break }");
        let case = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::CaseClause)
            .unwrap();
        let pat = case.children().next().unwrap();
        assert_eq!(pat.kind(), NodeKind::EnumCasePattern);
        assert_eq!(pat.text(), Some("some"));
        assert_eq!(pat.children().next().unwrap().text(), Some("x"));
    }

    #[test]
    fn for_case_optional_shorthand_is_a_some_pattern() {
        let ast = ast_of("for case let x? in xs { print(x) }");
        let pat = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::EnumCasePattern)
            .unwrap();
        assert_eq!(pat.text(), Some("some"));
        assert_eq!(pat.children().next().unwrap().text(), Some("x"));
    }

    #[test]
    fn if_case_optional_binding_lowers_to_a_let() {
        let ast = ast_of("if case let v? = maybe { }");
        let if_stmt = first_stmt(&ast);
        let cond = if_stmt.children().next().unwrap();
        assert_eq!(cond.kind(), NodeKind::LetDecl);
        assert_eq!(cond.children().next().unwrap().text(), Some("v"));
    }

    #[test]
    fn import_declaration_records_module_path() {
        let ast = ast_of("import Foundation");
        let decl = first_stmt(&ast);
        assert_eq!(decl.kind(), NodeKind::ImportDecl);
        assert_eq!(decl.text(), Some("Foundation"));
    }

    #[test]
    fn open_class_is_a_class_declaration() {
        let ast = ast_of("open class Service { }");
        let decl = first_stmt(&ast);
        assert_eq!(decl.kind(), NodeKind::ClassDecl);
        assert_eq!(decl.text(), Some("Service"));
        assert_eq!(decl.modifiers(), &["open".to_string()]);
    }

    #[test]
    fn static_prefix_func_collects_both_modifiers() {
        let ast = ast_of("struct V { static prefix func - (x: V) -> V { x } }");
        let func = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::FuncDecl)
            .unwrap();
        assert_eq!(func.text(), Some("-"));
        assert!(func.modifiers().iter().any(|m| m == "static"));
        assert!(func.modifiers().iter().any(|m| m == "prefix"));
    }

    #[test]
    fn type_attribute_prefixing_a_function_type() {
        let ast = ast_of("func run(_ work: @escaping () -> Void) { }");
        let param = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::Param)
            .unwrap();
        let ty = param.children().next().unwrap();
        assert_eq!(ty.text(), Some("() -> Void"));
    }

    #[test]
    fn autoclosure_attribute_is_recorded_on_the_parameter() {
        let ast = ast_of("func f(_ p: @autoclosure () -> Bool) { }");
        let param = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::Param)
            .unwrap();
        assert!(param.modifiers().iter().any(|m| m == "autoclosure"));
        let ty = param.children().next().unwrap();
        assert_eq!(ty.text(), Some("() -> Bool"));
    }

    #[test]
    fn result_builder_attribute_is_recorded_on_parameter() {
        let ast = ast_of("func wrap(@StringBuilder _ content: () -> String) { }");
        let param = first_stmt(&ast)
            .children()
            .find(|c| c.kind() == NodeKind::Param)
            .unwrap();
        assert_eq!(param.text(), Some("content"));
        let attr = param
            .children()
            .find(|c| c.kind() == NodeKind::Attribute)
            .unwrap();
        assert_eq!(attr.text(), Some("StringBuilder"));
    }
}
