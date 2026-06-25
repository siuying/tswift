//! A real Rust enum over msf's `ASTNodeKind` constants.
//!
//! Only the kinds the runtime currently distinguishes are named; every other
//! kind maps to [`NodeKind::Other`] carrying msf's raw discriminant, so nothing
//! is lost and matches stay exhaustive. New milestones promote `Other` values to
//! named variants as the evaluator learns to handle them.

/// What kind of syntax an AST [`Node`](crate::Node) represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    SourceFile,
    Block,
    ExprStmt,
    ReturnStmt,
    VarDecl,
    LetDecl,
    FuncDecl,
    Param,
    CallExpr,
    /// Identifier reference (msf calls this `unresolved_decl_ref_expr`).
    IdentExpr,
    MemberExpr,
    BinaryExpr,
    UnaryExpr,
    AssignExpr,
    ParenExpr,
    TupleExpr,
    TernaryExpr,
    /// A type annotation identifier, e.g. the `Int8` in `let x: Int8`.
    TypeIdent,
    IntegerLiteral,
    FloatLiteral,
    StringLiteral,
    BoolLiteral,
    NilLiteral,
    /// Any kind the runtime does not yet name, carrying msf's raw discriminant.
    Other(u32),
}

impl NodeKind {
    pub(crate) fn from_raw(raw: msf_sys::ASTNodeKind::Type) -> NodeKind {
        use msf_sys::ASTNodeKind as K;
        match raw {
            K::AST_SOURCE_FILE => NodeKind::SourceFile,
            K::AST_BLOCK => NodeKind::Block,
            K::AST_EXPR_STMT => NodeKind::ExprStmt,
            K::AST_RETURN_STMT => NodeKind::ReturnStmt,
            K::AST_VAR_DECL => NodeKind::VarDecl,
            K::AST_LET_DECL => NodeKind::LetDecl,
            K::AST_FUNC_DECL => NodeKind::FuncDecl,
            K::AST_PARAM => NodeKind::Param,
            K::AST_CALL_EXPR => NodeKind::CallExpr,
            K::AST_IDENT_EXPR => NodeKind::IdentExpr,
            K::AST_MEMBER_EXPR => NodeKind::MemberExpr,
            K::AST_BINARY_EXPR => NodeKind::BinaryExpr,
            K::AST_UNARY_EXPR => NodeKind::UnaryExpr,
            K::AST_ASSIGN_EXPR => NodeKind::AssignExpr,
            K::AST_PAREN_EXPR => NodeKind::ParenExpr,
            K::AST_TUPLE_EXPR => NodeKind::TupleExpr,
            K::AST_TERNARY_EXPR => NodeKind::TernaryExpr,
            K::AST_TYPE_IDENT => NodeKind::TypeIdent,
            K::AST_INTEGER_LITERAL => NodeKind::IntegerLiteral,
            K::AST_FLOAT_LITERAL => NodeKind::FloatLiteral,
            K::AST_STRING_LITERAL => NodeKind::StringLiteral,
            K::AST_BOOL_LITERAL => NodeKind::BoolLiteral,
            K::AST_NIL_LITERAL => NodeKind::NilLiteral,
            other => NodeKind::Other(other),
        }
    }
}
