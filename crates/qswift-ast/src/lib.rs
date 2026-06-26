//! The owned Swift AST for the quick-swift frontend.
//!
//! Nodes live in a flat arena ([`Ast`]) addressed by [`NodeId`]; a [`Node`]
//! cursor borrows the arena to walk it ergonomically. This is the pure-Rust
//! counterpart to msf's arena AST — the parser builds it and sema annotates it
//! with [`Type`]s, and the frontend adapter exposes it through the same
//! `Node`/`NodeKind`/`type_name` surface the runtime already consumes.
//!
//! Scope today is the walking-skeleton subset; [`NodeKind`] grows tier by tier.

#![forbid(unsafe_code)]

/// The syntactic category of a [`Node`]. Names mirror the frontend's existing
/// `NodeKind` vocabulary so AST dumps line up across backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// The top-level file node; root of every [`Ast`].
    SourceFile,
    /// An expression used as a statement.
    ExprStmt,
    /// A function/method call, `callee(args...)`.
    CallExpr,
    /// A bare identifier reference, e.g. `print`, `x`.
    IdentExpr,
    /// A binary operator application, `lhs op rhs`.
    BinaryExpr,
    /// A prefix unary operator application, `op operand` (`-x`, `!flag`, `~bits`).
    PrefixExpr,
    /// An assignment `lhs op= rhs` (plain `=` or compound), used as a statement.
    AssignExpr,
    /// A ternary conditional `cond ? then : else`.
    TernaryExpr,
    /// A tuple literal `(a, b, ...)`.
    TupleExpr,
    /// An array literal `[a, b, ...]`.
    ArrayLiteral,
    /// A dictionary literal `[k: v, ...]` (alternating key/value children).
    DictLiteral,
    /// A subscript access `base[index...]`.
    SubscriptExpr,
    /// A member or tuple-index access `base.member` / `base.0`.
    MemberExpr,
    /// A `let` binding declaration.
    LetDecl,
    /// A `var` binding declaration.
    VarDecl,
    /// A function declaration, `func name(params) -> Ret { body }`.
    FuncDecl,
    /// A `struct` declaration.
    StructDecl,
    /// An `enum` declaration.
    EnumDecl,
    /// A `class` declaration.
    ClassDecl,
    /// An `actor` declaration (a reference type with actor isolation).
    ActorDecl,
    /// A `protocol` declaration.
    ProtocolDecl,
    /// An `extension` declaration.
    ExtensionDecl,
    /// An `associatedtype` requirement.
    AssociatedTypeDecl,
    /// A `typealias` declaration.
    TypeAliasDecl,
    /// An `import` declaration (text is the imported module path).
    ImportDecl,
    /// A generic parameter `T` (optionally constrained).
    GenericParam,
    /// A `deinit { }` declaration.
    DeinitDecl,
    /// A `do { } catch { }` statement.
    DoStmt,
    /// A `catch` clause of a `do` statement.
    CatchClause,
    /// A `throw expr` statement.
    ThrowStmt,
    /// A `defer { }` statement.
    DeferStmt,
    /// A `try` / `try?` / `try!` expression (text is the operator).
    TryExpr,
    /// An `await expr` expression (suspends until the operand's task completes).
    AwaitExpr,
    /// An `&place` inout argument expression at a call site.
    InoutExpr,
    /// An `operator` declaration (`infix operator <> : Group`).
    OperatorDecl,
    /// A `precedencegroup` declaration.
    PrecedenceGroupDecl,
    /// A compiler directive used as a statement or expression (`#warning`,
    /// `#error`, `#file`, `#line`, …); text is the directive (with `#`).
    CompilerDirective,
    /// An attribute such as `@main` (text includes the `@`).
    Attribute,
    /// A closure expression `{ [captures] params in body }`.
    ClosureExpr,
    /// One entry of a closure capture list (`[weak self]`, `[base = 100]`).
    /// Text is the captured name; an optional child is the capture initializer.
    ClosureCapture,
    /// A type-cast expression `operand is/as/as?/as! Type` (text is the operator).
    CastExpr,
    /// One `case` of an enum (text is the case name; children are associated
    /// type refs or a raw-value expression).
    EnumCaseDecl,
    /// An initializer declaration `init(...) { }`.
    InitDecl,
    /// A `subscript(...) -> T { ... }` declaration.
    SubscriptDecl,
    /// A property/subscript accessor (`get`/`set`/`willSet`/`didSet`), text is its kind.
    Accessor,
    /// A postfix unary expression `operand op` (`x!`).
    PostfixExpr,
    /// A single function parameter.
    Param,
    /// A braced statement block `{ ... }`.
    Block,
    /// A `return [expr]` statement.
    ReturnStmt,
    /// An `if`/`else` statement (or expression).
    IfStmt,
    /// A `guard ... else { }` statement.
    GuardStmt,
    /// A `while` loop.
    WhileStmt,
    /// A `repeat { } while ...` loop.
    RepeatStmt,
    /// A `for ... in ... [where ...] { }` loop.
    ForStmt,
    /// A `switch` statement.
    SwitchStmt,
    /// One `case`/`default` clause of a `switch`.
    CaseClause,
    /// A `break [label]` statement.
    BreakStmt,
    /// A `continue [label]` statement.
    ContinueStmt,
    /// A `fallthrough` statement.
    FallthroughStmt,
    /// A written type annotation, e.g. the `Int` in `let x: Int`.
    TypeRef,
    /// An array type `[Element]` (single child: the element type).
    TypeArray,
    /// A dictionary type `[Key: Value]` (children: key type, value type).
    TypeDict,
    /// A binding pattern that names a value, e.g. the `x` in `let x = 1`.
    NamePattern,
    /// The wildcard binding pattern `_`.
    WildcardPattern,
    /// A tuple destructuring pattern, e.g. `(a, b)` in `let (a, b) = pair`.
    TuplePattern,
    /// An enum-case pattern, e.g. `.some(let n)` or `Shape.rect(let w, let h)`.
    /// Text is the case name; children are the payload sub-patterns.
    EnumCasePattern,
    /// A range pattern in a `switch` case, e.g. `0...9` (text is the operator).
    RangePattern,
    /// A `where` guard clause attached to a `switch` case (single child: the
    /// guard expression).
    WhereClause,
    /// An integer literal.
    IntegerLiteral,
    /// A floating-point literal.
    FloatLiteral,
    /// A boolean literal `true` / `false`.
    BoolLiteral,
    /// The `nil` literal.
    NilLiteral,
    /// A string literal.
    StringLiteral,
}

impl NodeKind {
    /// The stable `snake_case` name used in AST dumps.
    pub fn name(self) -> &'static str {
        match self {
            NodeKind::SourceFile => "source_file",
            NodeKind::ExprStmt => "expr_stmt",
            NodeKind::CallExpr => "call_expr",
            NodeKind::IdentExpr => "ident_expr",
            NodeKind::BinaryExpr => "binary_expr",
            NodeKind::PrefixExpr => "prefix_expr",
            NodeKind::AssignExpr => "assign_expr",
            NodeKind::TernaryExpr => "ternary_expr",
            NodeKind::TupleExpr => "tuple_expr",
            NodeKind::ArrayLiteral => "array_literal",
            NodeKind::DictLiteral => "dict_literal",
            NodeKind::SubscriptExpr => "subscript_expr",
            NodeKind::MemberExpr => "member_expr",
            NodeKind::LetDecl => "let_decl",
            NodeKind::VarDecl => "var_decl",
            NodeKind::FuncDecl => "func_decl",
            NodeKind::StructDecl => "struct_decl",
            NodeKind::EnumDecl => "enum_decl",
            NodeKind::ClassDecl => "class_decl",
            NodeKind::ActorDecl => "actor_decl",
            NodeKind::ProtocolDecl => "protocol_decl",
            NodeKind::ExtensionDecl => "extension_decl",
            NodeKind::AssociatedTypeDecl => "associatedtype_decl",
            NodeKind::TypeAliasDecl => "typealias_decl",
            NodeKind::ImportDecl => "import_decl",
            NodeKind::GenericParam => "generic_param",
            NodeKind::DeinitDecl => "deinit_decl",
            NodeKind::DoStmt => "do_stmt",
            NodeKind::CatchClause => "catch_clause",
            NodeKind::ThrowStmt => "throw_stmt",
            NodeKind::DeferStmt => "defer_stmt",
            NodeKind::TryExpr => "try_expr",
            NodeKind::AwaitExpr => "await_expr",
            NodeKind::InoutExpr => "inout_expr",
            NodeKind::OperatorDecl => "operator_decl",
            NodeKind::PrecedenceGroupDecl => "precedencegroup_decl",
            NodeKind::CompilerDirective => "compiler_directive",
            NodeKind::Attribute => "attribute",
            NodeKind::ClosureExpr => "closure_expr",
            NodeKind::ClosureCapture => "closure_capture",
            NodeKind::CastExpr => "cast_expr",
            NodeKind::EnumCaseDecl => "enum_case_decl",
            NodeKind::InitDecl => "init_decl",
            NodeKind::SubscriptDecl => "subscript_decl",
            NodeKind::Accessor => "accessor",
            NodeKind::PostfixExpr => "postfix_expr",
            NodeKind::Param => "param",
            NodeKind::Block => "block",
            NodeKind::ReturnStmt => "return_stmt",
            NodeKind::IfStmt => "if_stmt",
            NodeKind::GuardStmt => "guard_stmt",
            NodeKind::WhileStmt => "while_stmt",
            NodeKind::RepeatStmt => "repeat_stmt",
            NodeKind::ForStmt => "for_stmt",
            NodeKind::SwitchStmt => "switch_stmt",
            NodeKind::CaseClause => "case_clause",
            NodeKind::BreakStmt => "break_stmt",
            NodeKind::ContinueStmt => "continue_stmt",
            NodeKind::FallthroughStmt => "fallthrough_stmt",
            NodeKind::TypeRef => "type_ref",
            NodeKind::TypeArray => "type_array",
            NodeKind::TypeDict => "type_dict",
            NodeKind::NamePattern => "name_pattern",
            NodeKind::WildcardPattern => "wildcard_pattern",
            NodeKind::TuplePattern => "tuple_pattern",
            NodeKind::EnumCasePattern => "enum_case_pattern",
            NodeKind::RangePattern => "range_pattern",
            NodeKind::WhereClause => "where_clause",
            NodeKind::IntegerLiteral => "integer_literal",
            NodeKind::FloatLiteral => "float_literal",
            NodeKind::BoolLiteral => "bool_literal",
            NodeKind::NilLiteral => "nil_literal",
            NodeKind::StringLiteral => "string_literal",
        }
    }
}

/// A resolved Swift type, as far as the skeleton models it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Int,
    Double,
    String,
    Bool,
    Void,
}

impl Type {
    /// The Swift surface name (`Int`, `String`, …) used in dumps and tests.
    pub fn name(self) -> &'static str {
        match self {
            Type::Int => "Int",
            Type::Double => "Double",
            Type::String => "String",
            Type::Bool => "Bool",
            Type::Void => "Void",
        }
    }
}

/// An index into an [`Ast`]'s node arena. Opaque: only an [`Ast`] can mint or
/// resolve one, so a `NodeId` always refers to a live node in its arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

impl NodeId {
    fn index(self) -> usize {
        self.0 as usize
    }
}

/// The stored data for one AST node. Construct via [`Ast::add`].
#[derive(Debug, Clone)]
struct NodeData {
    kind: NodeKind,
    /// The node's primary lexeme (identifier name, literal text, operator), if any.
    text: Option<String>,
    line: u32,
    col: u32,
    ty: Option<Type>,
    /// Declaration modifier/effect keywords written before this node
    /// (`static`, `mutating`, `weak`, `throws`, `public`, …), in source order.
    modifiers: Vec<String>,
    /// For a call argument, its written label (`x` in `f(x: 1)`), if any.
    arg_label: Option<String>,
    children: Vec<NodeId>,
}

/// An owned AST arena. Every node is reachable from [`Ast::root`].
#[derive(Debug, Clone)]
pub struct Ast {
    nodes: Vec<NodeData>,
    root: NodeId,
}

impl Ast {
    /// Start a new arena whose root is a [`NodeKind::SourceFile`] at line 1.
    pub fn new() -> Ast {
        let nodes = vec![NodeData {
            kind: NodeKind::SourceFile,
            text: None,
            line: 1,
            col: 1,
            ty: None,
            modifiers: Vec::new(),
            arg_label: None,
            children: Vec::new(),
        }];
        Ast {
            nodes,
            root: NodeId(0),
        }
    }

    /// The root `source_file` node.
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Add a node and return its id. Attach it to a parent with [`Ast::append_child`].
    pub fn add(&mut self, kind: NodeKind, text: Option<&str>, line: u32, col: u32) -> NodeId {
        let index = u32::try_from(self.nodes.len()).expect("AST node count exceeds u32::MAX");
        let id = NodeId(index);
        self.nodes.push(NodeData {
            kind,
            text: text.map(str::to_string),
            line,
            col,
            ty: None,
            modifiers: Vec::new(),
            arg_label: None,
            children: Vec::new(),
        });
        id
    }

    /// Append `child` to `parent`'s child list (in source order).
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        self.nodes[parent.index()].children.push(child);
    }

    /// Record a declaration modifier/effect keyword on `id` (in source order).
    pub fn add_modifier(&mut self, id: NodeId, modifier: &str) {
        self.nodes[id.index()].modifiers.push(modifier.to_string());
    }

    /// Record the call-argument label written for `id` (`x` in `f(x: 1)`).
    pub fn set_arg_label(&mut self, id: NodeId, label: &str) {
        self.nodes[id.index()].arg_label = Some(label.to_string());
    }

    /// Set the resolved [`Type`] of `id` (called by sema).
    pub fn set_type(&mut self, id: NodeId, ty: Type) {
        self.nodes[id.index()].ty = Some(ty);
    }

    /// Re-tag the [`NodeKind`] of `id` (used by the parser to reinterpret a
    /// parsed expression as a pattern, e.g. a range expression as a
    /// [`NodeKind::RangePattern`]).
    pub fn set_kind(&mut self, id: NodeId, kind: NodeKind) {
        self.nodes[id.index()].kind = kind;
    }

    /// Deep-copy the subtree rooted at `id`, returning the new root's id.
    ///
    /// Used to share a parsed type annotation across the desugared bindings of
    /// a multi-name declaration (`var a, b, c: Double`), where each binding
    /// needs its own copy of the annotation subtree.
    pub fn clone_subtree(&mut self, id: NodeId) -> NodeId {
        let data = self.nodes[id.index()].clone();
        let index = u32::try_from(self.nodes.len()).expect("AST node count exceeds u32::MAX");
        let new = NodeId(index);
        let children = data.children.clone();
        self.nodes.push(NodeData {
            children: Vec::new(),
            ..data
        });
        for child in children {
            let cloned = self.clone_subtree(child);
            self.nodes[new.index()].children.push(cloned);
        }
        new
    }

    /// A read cursor over `id`.
    pub fn node(&self, id: NodeId) -> Node<'_> {
        Node { ast: self, id }
    }

    fn data(&self, id: NodeId) -> &NodeData {
        &self.nodes[id.index()]
    }
}

impl Default for Ast {
    fn default() -> Self {
        Ast::new()
    }
}

/// A borrowed cursor over one node of an [`Ast`]. Cheap to copy; tied to the
/// arena by lifetime so it cannot dangle.
#[derive(Debug, Clone, Copy)]
pub struct Node<'a> {
    ast: &'a Ast,
    id: NodeId,
}

impl<'a> Node<'a> {
    /// This node's id within its arena.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// The node's syntactic category.
    pub fn kind(&self) -> NodeKind {
        self.ast.data(self.id).kind
    }

    /// The node's primary lexeme, if any.
    pub fn text(&self) -> Option<&'a str> {
        self.ast.data(self.id).text.as_deref()
    }

    /// 1-based source line.
    pub fn line(&self) -> u32 {
        self.ast.data(self.id).line
    }

    /// 1-based source column.
    pub fn col(&self) -> u32 {
        self.ast.data(self.id).col
    }

    /// The node's resolved [`Type`], if sema set one.
    pub fn ty(&self) -> Option<Type> {
        self.ast.data(self.id).ty
    }

    /// The node's resolved type's surface name (`Int`, `String`, …), if sema set one.
    pub fn type_name(&self) -> Option<&'static str> {
        self.ast.data(self.id).ty.map(Type::name)
    }

    /// The declaration modifier/effect keywords recorded on this node.
    pub fn modifiers(&self) -> &'a [String] {
        &self.ast.data(self.id).modifiers
    }

    /// The call-argument label recorded on this node, if any.
    pub fn arg_label(&self) -> Option<&'a str> {
        self.ast.data(self.id).arg_label.as_deref()
    }

    /// Iterator over the node's direct children, in source order.
    pub fn children(&self) -> impl Iterator<Item = Node<'a>> + 'a {
        let ast = self.ast;
        self.ast
            .data(self.id)
            .children
            .iter()
            .map(move |&cid| Node { ast, id: cid })
    }

    /// A recursive, indented dump of this subtree: `kind "text" Lline :Type`.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.dump_into(&mut out, 0);
        out
    }

    fn dump_into(&self, out: &mut String, depth: usize) {
        use std::fmt::Write as _;
        let indent = "  ".repeat(depth);
        let _ = write!(out, "{indent}{}", self.kind().name());
        if let Some(t) = self.text() {
            let _ = write!(out, " {t:?}");
        }
        let _ = write!(out, " L{}", self.line());
        if let Some(ty) = self.type_name() {
            let _ = write!(out, " :{ty}");
        }
        let _ = writeln!(out);
        for child in self.children() {
            child.dump_into(out, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build `print("hi")` by hand and walk it through the cursor API.
    #[test]
    fn builds_and_walks_a_call() {
        let mut ast = Ast::new();
        let stmt = ast.add(NodeKind::ExprStmt, None, 1, 1);
        let call = ast.add(NodeKind::CallExpr, None, 1, 1);
        let callee = ast.add(NodeKind::IdentExpr, Some("print"), 1, 1);
        let arg = ast.add(NodeKind::StringLiteral, Some("\"hi\""), 1, 7);
        ast.append_child(ast.root(), stmt);
        ast.append_child(stmt, call);
        ast.append_child(call, callee);
        ast.append_child(call, arg);

        let root = ast.node(ast.root());
        assert_eq!(root.kind(), NodeKind::SourceFile);
        let call = root.children().next().unwrap().children().next().unwrap();
        assert_eq!(call.kind(), NodeKind::CallExpr);
        let kids: Vec<_> = call.children().map(|c| (c.kind(), c.text())).collect();
        assert_eq!(
            kids,
            vec![
                (NodeKind::IdentExpr, Some("print")),
                (NodeKind::StringLiteral, Some("\"hi\"")),
            ]
        );
    }

    #[test]
    fn sema_can_annotate_types_and_dump_shows_them() {
        let mut ast = Ast::new();
        let lit = ast.add(NodeKind::IntegerLiteral, Some("42"), 1, 1);
        ast.append_child(ast.root(), lit);
        ast.set_type(lit, Type::Int);

        let dump = ast.node(ast.root()).dump();
        assert_eq!(dump, "source_file L1\n  integer_literal \"42\" L1 :Int\n");
    }

    #[test]
    fn clone_subtree_deep_copies_independently() {
        let mut ast = Ast::new();
        let dict = ast.add(NodeKind::TypeDict, None, 2, 5);
        let key = ast.add(NodeKind::TypeRef, Some("String"), 2, 6);
        let val = ast.add(NodeKind::TypeRef, Some("Int"), 2, 14);
        ast.append_child(dict, key);
        ast.append_child(dict, val);

        let copy = ast.clone_subtree(dict);
        assert_ne!(copy, dict, "clone is a distinct node");

        let copied = ast.node(copy);
        assert_eq!(copied.kind(), NodeKind::TypeDict);
        let kids: Vec<_> = copied.children().map(|c| (c.kind(), c.text())).collect();
        assert_eq!(
            kids,
            vec![
                (NodeKind::TypeRef, Some("String")),
                (NodeKind::TypeRef, Some("Int")),
            ]
        );
        // Distinct child node ids: the clone does not alias the original.
        let orig_children: Vec<_> = ast.node(dict).children().map(|c| c.id()).collect();
        let copy_children: Vec<_> = ast.node(copy).children().map(|c| c.id()).collect();
        assert_ne!(orig_children, copy_children);
    }

    #[test]
    fn node_kind_and_type_names_are_stable() {
        assert_eq!(NodeKind::CallExpr.name(), "call_expr");
        assert_eq!(NodeKind::IdentExpr.name(), "ident_expr");
        assert_eq!(Type::String.name(), "String");
    }
}
