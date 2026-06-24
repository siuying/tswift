//! The `eval(node, env) -> Completion` dispatcher — skeleton edition.
//!
//! The full plan splits this into `interp`/`call`/`frame`/`pattern`/… with a
//! `Completion` enum that unwinds `Return`/`Break`/`Throw` without panics. Here
//! we keep just enough to walk a `source_file` and evaluate an integer-literal
//! call to a registered native (`print`).

use std::collections::HashMap;
use std::io::Write;

use msf::{Analysis, Node, NodeKind};

use crate::value::SwiftValue;

/// A native (Rust-implemented) Swift function. It receives the output sink and
/// the already-evaluated arguments, and returns its result value.
pub type NativeFn = fn(&mut dyn Write, &[SwiftValue]) -> SwiftValue;

/// A failure while evaluating the AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// A construct the skeleton evaluator does not implement yet.
    Unsupported(String),
    /// A call to a function name with no registered native.
    UnknownFunction(String),
    /// The source failed to analyze; carries msf's diagnostics, joined.
    Analysis(String),
    /// Writing to the output sink failed.
    Io(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Unsupported(what) => write!(f, "unsupported construct: {what}"),
            EvalError::UnknownFunction(name) => write!(f, "unknown function: {name}"),
            EvalError::Analysis(diags) => write!(f, "analysis failed:\n{diags}"),
            EvalError::Io(e) => write!(f, "output error: {e}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// The tree-walking interpreter. Owns the native function table and borrows an
/// output sink for the duration of a run.
pub struct Interpreter<'w> {
    out: &'w mut dyn Write,
    natives: HashMap<String, NativeFn>,
}

impl<'w> Interpreter<'w> {
    /// Create an interpreter that writes program output to `out`.
    pub fn new(out: &'w mut dyn Write) -> Self {
        Interpreter {
            out,
            natives: HashMap::new(),
        }
    }

    /// Register a native function callable from Swift source by `name`.
    pub fn register_native(&mut self, name: &str, f: NativeFn) {
        self.natives.insert(name.to_string(), f);
    }

    /// Evaluate a fully-analyzed program. Refuses to run if analysis reported
    /// errors, surfacing them as an [`EvalError::Analysis`].
    pub fn run(&mut self, analysis: &Analysis) -> Result<(), EvalError> {
        if !analysis.is_ok() {
            let diags = analysis
                .diagnostics()
                .iter()
                .map(|d| format!("  {}:{}: {}", d.line, d.col, d.message))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(EvalError::Analysis(diags));
        }
        self.eval(&analysis.root())?;
        Ok(())
    }

    /// Evaluate a node, returning its value.
    fn eval(&mut self, node: &Node) -> Result<SwiftValue, EvalError> {
        match node.kind() {
            // A program / block: evaluate each statement, yield the last value.
            NodeKind::SourceFile | NodeKind::Block => {
                let mut last = SwiftValue::Void;
                for child in node.children() {
                    last = self.eval(&child)?;
                }
                Ok(last)
            }
            // An expression statement wraps a single expression.
            NodeKind::ExprStmt => {
                let mut last = SwiftValue::Void;
                for child in node.children() {
                    last = self.eval(&child)?;
                }
                Ok(last)
            }
            NodeKind::CallExpr => self.eval_call(node),
            NodeKind::IntegerLiteral => Ok(SwiftValue::Int(node.int().unwrap_or(0))),
            NodeKind::BoolLiteral => Ok(SwiftValue::Bool(node.bool().unwrap_or(false))),
            NodeKind::FloatLiteral => Ok(SwiftValue::Double(node.float().unwrap_or(0.0))),
            other => Err(EvalError::Unsupported(format!("{other:?}"))),
        }
    }

    /// Evaluate a call expression: `callee(arg, ...)`.
    ///
    /// In the msf AST the first child is the callee (an identifier reference for
    /// a free function like `print`) and the remaining children are arguments.
    fn eval_call(&mut self, node: &Node) -> Result<SwiftValue, EvalError> {
        let mut children = node.children();
        let callee = children
            .next()
            .ok_or_else(|| EvalError::Unsupported("call with no callee".into()))?;

        let name = match callee.kind() {
            NodeKind::IdentExpr => callee
                .text()
                .ok_or_else(|| EvalError::Unsupported("unnamed callee".into()))?,
            other => {
                return Err(EvalError::Unsupported(format!("callee of kind {other:?}")));
            }
        };

        let mut args = Vec::new();
        for arg in children {
            args.push(self.eval(&arg)?);
        }

        let native = self
            .natives
            .get(&name)
            .copied()
            .ok_or(EvalError::UnknownFunction(name))?;
        Ok(native(self.out, &args))
    }
}
