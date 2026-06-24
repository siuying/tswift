//! The `eval(node, env)` tree-walker.
//!
//! Evaluates literals, arithmetic, and `let`/`var` bindings on top of msf's
//! typed AST. Integer widths follow msf's resolved types so overflow-trapping
//! and wrapping operators match Swift exactly.

use std::collections::HashMap;
use std::io::Write;

use msf::{Analysis, Node, NodeKind};

use crate::env::{BindError, Env};
use crate::ops;
use crate::value::{IntValue, IntWidth, SwiftValue};

/// A native (Rust-implemented) Swift function. It receives the output sink and
/// the already-evaluated arguments, and returns its result value.
pub type NativeFn = fn(&mut dyn Write, &[SwiftValue]) -> SwiftValue;

/// A failure while evaluating the AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// A construct the evaluator does not implement yet.
    Unsupported(String),
    /// A call to a function name with no registered native.
    UnknownFunction(String),
    /// Use of an unbound variable.
    UnknownVariable(String),
    /// Assignment to a `let` binding.
    Immutable(String),
    /// A runtime trap: overflow, division by zero, etc. (Swift `fatalError`).
    Trap(String),
    /// A type error the evaluator detected at runtime.
    Type(String),
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
            EvalError::UnknownVariable(name) => write!(f, "unknown variable: {name}"),
            EvalError::Immutable(name) => {
                write!(f, "cannot assign to `{name}`: it is a `let` constant")
            }
            EvalError::Trap(msg) => write!(f, "fatal error: {msg}"),
            EvalError::Type(msg) => write!(f, "type error: {msg}"),
            EvalError::Analysis(diags) => write!(f, "analysis failed:\n{diags}"),
            EvalError::Io(e) => write!(f, "output error: {e}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// The tree-walking interpreter. Owns the native function table and the
/// environment, and borrows an output sink for the duration of a run.
pub struct Interpreter<'w> {
    out: &'w mut dyn Write,
    natives: HashMap<String, NativeFn>,
    env: Env,
}

impl<'w> Interpreter<'w> {
    /// Create an interpreter that writes program output to `out`.
    pub fn new(out: &'w mut dyn Write) -> Self {
        Interpreter {
            out,
            natives: HashMap::new(),
            env: Env::new(),
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
            NodeKind::SourceFile | NodeKind::Block | NodeKind::ExprStmt => {
                let mut last = SwiftValue::Void;
                for child in node.children() {
                    last = self.eval(&child)?;
                }
                Ok(last)
            }
            NodeKind::LetDecl => self.eval_decl(node, false),
            NodeKind::VarDecl => self.eval_decl(node, true),
            NodeKind::CallExpr => self.eval_call(node),
            NodeKind::BinaryExpr => self.eval_binary(node),
            NodeKind::UnaryExpr => self.eval_unary(node),
            NodeKind::AssignExpr => self.eval_assign(node),
            NodeKind::ParenExpr => self.eval_only_child(node),
            NodeKind::MemberExpr => self.eval_member(node),
            NodeKind::IdentExpr => self.eval_ident(node),
            NodeKind::IntegerLiteral => Ok(self.eval_int_literal(node)),
            NodeKind::BoolLiteral => Ok(SwiftValue::Bool(node.bool().unwrap_or(false))),
            NodeKind::FloatLiteral => Ok(SwiftValue::Double(node.float().unwrap_or(0.0))),
            NodeKind::StringLiteral => Ok(SwiftValue::Str(decode_string_literal(
                &node.text().unwrap_or_default(),
            ))),
            other => Err(EvalError::Unsupported(format!("{other:?}"))),
        }
    }

    /// Evaluate the single meaningful child of a wrapper node (e.g. paren).
    fn eval_only_child(&mut self, node: &Node) -> Result<SwiftValue, EvalError> {
        let child = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("empty wrapper node".into()))?;
        self.eval(&child)
    }

    /// `let`/`var name [= init]`.
    fn eval_decl(&mut self, node: &Node, mutable: bool) -> Result<SwiftValue, EvalError> {
        let name = node
            .decl_name()
            .ok_or_else(|| EvalError::Unsupported("declaration without a name".into()))?;
        // The initializer, if any, is the node's last expression child.
        let value = match node.children().last() {
            Some(init) => {
                let v = self.eval(&init)?;
                self.coerce_to_decl_type(node, v)
            }
            None => SwiftValue::Void,
        };
        self.env.declare(&name, value, mutable);
        Ok(SwiftValue::Void)
    }

    /// If the declaration has an explicit integer type annotation, retag the
    /// initializer's width to match it.
    ///
    /// msf collapses every fixed-width integer to `Int` in its resolved types,
    /// so the only reliable source for the declared width is the explicit
    /// `TYPE_IDENT` annotation node, when present.
    fn coerce_to_decl_type(&self, node: &Node, value: SwiftValue) -> SwiftValue {
        let SwiftValue::Int(i) = &value else {
            return value;
        };
        for child in node.children() {
            if child.kind() == NodeKind::TypeIdent {
                if let Some(w) = child.text().as_deref().and_then(IntWidth::from_type_name) {
                    return SwiftValue::Int(IntValue::new(i.raw, w));
                }
            }
        }
        value
    }

    /// An identifier reference: look up a binding.
    fn eval_ident(&mut self, node: &Node) -> Result<SwiftValue, EvalError> {
        let name = node
            .text()
            .ok_or_else(|| EvalError::Unsupported("unnamed identifier".into()))?;
        self.env
            .get(&name)
            .cloned()
            .ok_or(EvalError::UnknownVariable(name))
    }

    /// An integer literal, widened to its msf-resolved type when known.
    fn eval_int_literal(&self, node: &Node) -> SwiftValue {
        let raw = node.int().unwrap_or(0) as i128;
        let width = node
            .type_name()
            .and_then(|n| IntWidth::from_type_name(&n))
            .unwrap_or(IntWidth::I64);
        SwiftValue::Int(IntValue::new(raw, width))
    }

    /// A binary operation, with short-circuiting `&&`/`||`.
    fn eval_binary(&mut self, node: &Node) -> Result<SwiftValue, EvalError> {
        let op = node
            .op_text()
            .ok_or_else(|| EvalError::Unsupported("binary without operator".into()))?;
        let mut kids = node.children();
        let lhs = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("binary without lhs".into()))?;
        let rhs = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("binary without rhs".into()))?;

        // Short-circuit logical operators.
        if op == "&&" || op == "||" {
            let l = self.eval(&lhs)?;
            let lb = l.as_bool().ok_or_else(|| {
                EvalError::Type(format!("`{op}` needs Bool, got {}", l.type_name()))
            })?;
            if op == "&&" && !lb {
                return Ok(SwiftValue::Bool(false));
            }
            if op == "||" && lb {
                return Ok(SwiftValue::Bool(true));
            }
            let r = self.eval(&rhs)?;
            let rb = r.as_bool().ok_or_else(|| {
                EvalError::Type(format!("`{op}` needs Bool, got {}", r.type_name()))
            })?;
            return Ok(SwiftValue::Bool(rb));
        }

        let l = self.eval(&lhs)?;
        let r = self.eval(&rhs)?;
        ops::binary(&op, &l, &r).map_err(EvalError::Trap)
    }

    /// A unary operation (`-x`, `!b`, `~n`).
    fn eval_unary(&mut self, node: &Node) -> Result<SwiftValue, EvalError> {
        let op = node
            .op_text()
            .or_else(|| node.text())
            .ok_or_else(|| EvalError::Unsupported("unary without operator".into()))?;
        let operand = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("unary without operand".into()))?;
        let v = self.eval(&operand)?;
        ops::unary(&op, &v).map_err(EvalError::Trap)
    }

    /// Assignment: plain `=` and compound `+=`, `-=`, … to a binding.
    fn eval_assign(&mut self, node: &Node) -> Result<SwiftValue, EvalError> {
        let op = node
            .op_text()
            .ok_or_else(|| EvalError::Unsupported("assignment without operator".into()))?;
        let mut kids = node.children();
        let target = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("assignment without target".into()))?;
        let rhs = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("assignment without value".into()))?;

        if target.kind() != NodeKind::IdentExpr {
            return Err(EvalError::Unsupported(
                "assignment target must be a simple variable for now".into(),
            ));
        }
        let name = target
            .text()
            .ok_or_else(|| EvalError::Unsupported("unnamed assignment target".into()))?;

        let new_value = if op == "=" {
            self.eval(&rhs)?
        } else {
            // Compound: strip the trailing `=` to get the binary operator.
            let bin_op = op.trim_end_matches('=');
            let current = self
                .env
                .get(&name)
                .cloned()
                .ok_or_else(|| EvalError::UnknownVariable(name.clone()))?;
            let r = self.eval(&rhs)?;
            ops::binary(bin_op, &current, &r).map_err(EvalError::Trap)?
        };

        match self.env.assign(&name, new_value) {
            Ok(()) => Ok(SwiftValue::Void),
            Err(BindError::Immutable(n)) => Err(EvalError::Immutable(n)),
            Err(BindError::Unbound(n)) => Err(EvalError::UnknownVariable(n)),
        }
    }

    /// Member access. Supports static members of integer types (`Int.max`,
    /// `Int.min`, etc.).
    fn eval_member(&mut self, node: &Node) -> Result<SwiftValue, EvalError> {
        let member = node
            .text()
            .ok_or_else(|| EvalError::Unsupported("member without a name".into()))?;
        let base = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("member without a base".into()))?;

        if base.kind() == NodeKind::IdentExpr {
            if let Some(type_name) = base.text() {
                if let Some(w) = IntWidth::from_type_name(&type_name) {
                    return match member.as_str() {
                        "max" => Ok(SwiftValue::Int(IntValue::new(w.max(), w))),
                        "min" => Ok(SwiftValue::Int(IntValue::new(w.min(), w))),
                        _ => Err(EvalError::Unsupported(format!("{type_name}.{member}"))),
                    };
                }
            }
        }
        Err(EvalError::Unsupported(format!("member access .{member}")))
    }

    /// Evaluate a call: a native function, or a numeric/string conversion
    /// initializer like `Int(x)`, `Double(n)`, `UInt8(v)`.
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

        // Conversion initializers take exactly one argument.
        if args.len() == 1 {
            if let Some(v) = self.try_conversion(&name, &args[0])? {
                return Ok(v);
            }
        }

        let native = self
            .natives
            .get(&name)
            .copied()
            .ok_or(EvalError::UnknownFunction(name))?;
        Ok(native(self.out, &args))
    }

    /// Attempt a numeric/string conversion `Type(value)`. Returns `Ok(None)` if
    /// `name` is not a known conversion type.
    fn try_conversion(
        &self,
        name: &str,
        value: &SwiftValue,
    ) -> Result<Option<SwiftValue>, EvalError> {
        if let Some(w) = IntWidth::from_type_name(name) {
            let raw = match value {
                SwiftValue::Int(i) => i.raw,
                SwiftValue::Double(d) => d.trunc() as i128,
                SwiftValue::Bool(b) => *b as i128,
                _ => {
                    return Err(EvalError::Type(format!(
                        "cannot convert {} to {name}",
                        value.type_name()
                    )))
                }
            };
            let v = IntValue::new(raw, w);
            if !v.in_range() {
                return Err(EvalError::Trap(format!(
                    "{raw} is not representable as {name}"
                )));
            }
            return Ok(Some(SwiftValue::Int(v)));
        }
        match name {
            "Double" | "Float" => {
                let d = match value {
                    SwiftValue::Int(i) => i.raw as f64,
                    SwiftValue::Double(d) => *d,
                    _ => {
                        return Err(EvalError::Type(format!(
                            "cannot convert {} to {name}",
                            value.type_name()
                        )))
                    }
                };
                Ok(Some(SwiftValue::Double(d)))
            }
            "String" => Ok(Some(SwiftValue::Str(value.to_string()))),
            _ => Ok(None),
        }
    }
}

/// Decode a Swift string literal's *source text* (including its delimiters) into
/// the runtime string it denotes: strips quotes and processes escapes.
fn decode_string_literal(raw: &str) -> String {
    // Raw string: #"..."# (and ##"..."##). Strip matching #s and quotes; no
    // escape processing inside.
    if raw.starts_with('#') {
        let hashes = raw.chars().take_while(|&c| c == '#').count();
        let inner = &raw[hashes..raw.len().saturating_sub(hashes)];
        let inner = inner
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(inner);
        return inner.to_string();
    }
    // Multiline: """ ... """
    if let Some(body) = raw
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
    {
        return decode_escapes(strip_multiline_indent(body));
    }
    // Ordinary: "..."
    let body = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw);
    decode_escapes(body)
}

/// Remove the leading/trailing newlines and common indentation of a multiline
/// string body (a simplified take on Swift's whitespace rules).
fn strip_multiline_indent(body: &str) -> &str {
    body.trim_start_matches('\n').trim_end_matches([' ', '\t'])
}

/// Process backslash escapes in a single-line/multiline string body.
fn decode_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('u') => {
                // \u{XXXX}
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut hex = String::new();
                    for h in chars.by_ref() {
                        if h == '}' {
                            break;
                        }
                        hex.push(h);
                    }
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                        }
                    }
                } else {
                    out.push('u');
                }
            }
            Some(other) => {
                // Unknown escape (e.g. interpolation `\(` handled elsewhere):
                // keep both characters verbatim.
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a program and capture its stdout.
    fn run(src: &str) -> Result<String, EvalError> {
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut interp = Interpreter::new(&mut buf);
            crate::install_test_print(&mut interp);
            interp.run(&analysis)?;
        }
        Ok(String::from_utf8(buf).unwrap())
    }

    #[test]
    fn arithmetic_and_bindings() {
        let out = run("let a = 7\nvar b = a * 6\nb += 1\nprint(b)\n").unwrap();
        assert_eq!(out, "43\n");
    }

    #[test]
    fn wrapping_add_on_int_max() {
        let out = run("print(Int.max &+ 1)\n").unwrap();
        assert_eq!(out, format!("{}\n", i64::MIN));
    }

    #[test]
    fn overflow_traps() {
        let err = run("print(Int.max + 1)\n").unwrap_err();
        assert!(matches!(err, EvalError::Trap(_)), "got {err:?}");
    }

    #[test]
    fn let_is_immutable() {
        // msf rejects this at analysis time; the runtime guard is the backstop.
        let err = run("let a = 1\na = 2\n").unwrap_err();
        assert!(
            matches!(err, EvalError::Immutable(_) | EvalError::Analysis(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn typed_width_conversions() {
        let out = run("let x: UInt8 = 255\nprint(x &+ 1)\n").unwrap();
        assert_eq!(out, "0\n");
    }

    #[test]
    fn double_formatting() {
        let out = run("print(3.0)\nprint(3.5)\n").unwrap();
        assert_eq!(out, "3.0\n3.5\n");
    }

    #[test]
    fn int_from_double_truncates() {
        let out = run("print(Int(3.9))\n").unwrap();
        assert_eq!(out, "3\n");
    }
}
