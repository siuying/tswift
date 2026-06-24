//! The `eval(node, env)` tree-walker.
//!
//! Control flow (`return`, and later `break`/`continue`/`throw`) unwinds through
//! the `Err` channel as a [`Signal`], so a `?` naturally propagates it up to the
//! construct that handles it — without panicking. Real interpreter failures ride
//! the same channel as [`Signal::Error`].

use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

use msf::{Analysis, Node, NodeKind};

use crate::env::{BindError, Env, Scope};
use crate::ops;
use crate::value::{IntValue, IntWidth, SwiftValue};

/// Maximum nested Swift call depth before the interpreter traps, converting
/// unbounded recursion into a catchable error instead of a native stack
/// overflow.
const MAX_CALL_DEPTH: usize = 5000;

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
    /// A runtime trap: overflow, division by zero, deep recursion, etc.
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

/// A non-local control transfer produced while evaluating a node. Travels up the
/// `Err` channel so `?` propagates it to the handling construct.
///
/// `Break`/`Continue`/`Throw` are wired in the control-flow and error-handling
/// milestones; they exist here so the dispatch shape is stable.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
enum Signal {
    /// `return [value]` — unwinds to the enclosing function call.
    Return(SwiftValue),
    /// `break [label]` — unwinds to the targeted loop/switch.
    Break(Option<String>),
    /// `continue [label]` — unwinds to the targeted loop.
    Continue(Option<String>),
    /// A thrown Swift error value (error handling milestone).
    Throw(SwiftValue),
    /// A genuine interpreter error (not Swift control flow).
    Error(EvalError),
}

impl From<EvalError> for Signal {
    fn from(e: EvalError) -> Self {
        Signal::Error(e)
    }
}

/// Convenience: an operator/runtime trap message becomes a [`Signal::Error`].
fn trap(msg: String) -> Signal {
    Signal::Error(EvalError::Trap(msg))
}

type Eval = Result<SwiftValue, Signal>;

/// One declared Swift parameter, precomputed from its `AST_PARAM` node.
struct Param<'a> {
    label: Option<String>,
    name: String,
    variadic: bool,
    default: Option<Node<'a>>,
}

/// A user-defined function: its parameters, body, and captured scope chain.
struct FuncDef<'a> {
    params: Vec<Param<'a>>,
    body: Option<Node<'a>>,
    captured: Vec<Scope>,
}

/// The tree-walking interpreter.
pub struct Interpreter<'a, 'w> {
    out: &'w mut dyn Write,
    natives: HashMap<String, NativeFn>,
    env: Env,
    funcs: Vec<FuncDef<'a>>,
    depth: usize,
}

impl<'a, 'w> Interpreter<'a, 'w> {
    /// Create an interpreter that writes program output to `out`.
    pub fn new(out: &'w mut dyn Write) -> Self {
        Interpreter {
            out,
            natives: HashMap::new(),
            env: Env::new(),
            funcs: Vec::new(),
            depth: 0,
        }
    }

    /// Register a native function callable from Swift source by `name`.
    pub fn register_native(&mut self, name: &str, f: NativeFn) {
        self.natives.insert(name.to_string(), f);
    }

    /// Evaluate a fully-analyzed program.
    pub fn run(&mut self, analysis: &'a Analysis) -> Result<(), EvalError> {
        if !analysis.is_ok() {
            let diags = analysis
                .diagnostics()
                .iter()
                .map(|d| format!("  {}:{}: {}", d.line, d.col, d.message))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(EvalError::Analysis(diags));
        }
        match self.eval(&analysis.root()) {
            Ok(_) => Ok(()),
            Err(Signal::Error(e)) => Err(e),
            Err(Signal::Throw(v)) => Err(EvalError::Trap(format!("uncaught error: {v}"))),
            Err(Signal::Return(_)) => Ok(()),
            Err(other) => Err(EvalError::Unsupported(format!(
                "stray control flow: {other:?}"
            ))),
        }
    }

    /// Evaluate a node, returning its value (or a propagating [`Signal`]).
    fn eval(&mut self, node: &Node<'a>) -> Eval {
        match node.kind() {
            NodeKind::SourceFile | NodeKind::Block => self.eval_block(node),
            NodeKind::ExprStmt => self.eval_seq(node),
            NodeKind::FuncDecl => Ok(SwiftValue::Void), // hoisted by eval_block
            NodeKind::ReturnStmt => {
                let value = match node.children().next() {
                    Some(e) => self.eval(&e)?,
                    None => SwiftValue::Void,
                };
                Err(Signal::Return(value))
            }
            NodeKind::LetDecl => self.eval_decl(node, false),
            NodeKind::VarDecl => self.eval_decl(node, true),
            NodeKind::CallExpr => self.eval_call(node),
            NodeKind::BinaryExpr => self.eval_binary(node),
            NodeKind::UnaryExpr => self.eval_unary(node),
            NodeKind::AssignExpr => self.eval_assign(node),
            NodeKind::ParenExpr => self.eval_only_child(node),
            NodeKind::TernaryExpr => self.eval_ternary(node),
            NodeKind::MemberExpr => self.eval_member(node),
            NodeKind::IdentExpr => self.eval_ident(node),
            NodeKind::IntegerLiteral => Ok(self.eval_int_literal(node)),
            NodeKind::BoolLiteral => Ok(SwiftValue::Bool(node.bool().unwrap_or(false))),
            NodeKind::FloatLiteral => Ok(SwiftValue::Double(node.float().unwrap_or(0.0))),
            NodeKind::StringLiteral => Ok(SwiftValue::Str(decode_string_literal(
                &node.text().unwrap_or_default(),
            ))),
            other => Err(EvalError::Unsupported(format!("{other:?}")).into()),
        }
    }

    /// A `{ … }` block or source file: hoist function declarations first so
    /// forward references and mutual recursion resolve, then run statements.
    fn eval_block(&mut self, node: &Node<'a>) -> Eval {
        for child in node.children() {
            if child.kind() == NodeKind::FuncDecl {
                self.declare_func(&child);
            }
        }
        self.eval_seq(node)
    }

    /// Evaluate each child in order, yielding the last value.
    fn eval_seq(&mut self, node: &Node<'a>) -> Eval {
        let mut last = SwiftValue::Void;
        for child in node.children() {
            last = self.eval(&child)?;
        }
        Ok(last)
    }

    /// Evaluate the single meaningful child of a wrapper node (e.g. paren).
    fn eval_only_child(&mut self, node: &Node<'a>) -> Eval {
        let child = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("empty wrapper node".into()))?;
        self.eval(&child)
    }

    /// Register a function declaration as a first-class value in the current
    /// scope, capturing the enclosing scope chain.
    fn declare_func(&mut self, node: &Node<'a>) {
        let Some(name) = node.text() else {
            return;
        };
        // Avoid double-hoisting if eval_block runs twice on the same node.
        if matches!(self.env.get(&name), Some(SwiftValue::Function(_))) {
            return;
        }
        let mut params = Vec::new();
        let mut body = None;
        for child in node.children() {
            match child.kind() {
                NodeKind::Param => {
                    let info = child.param_info();
                    // The parameter's default value, if any, is a non-type child.
                    let default = child.children().find(|c| c.kind() != NodeKind::TypeIdent);
                    params.push(Param {
                        label: info.label,
                        name: info.name,
                        variadic: info.variadic,
                        default,
                    });
                }
                NodeKind::Block => body = Some(child),
                _ => {}
            }
        }
        let captured = self.env.capture();
        let id = self.funcs.len();
        self.funcs.push(FuncDef {
            params,
            body,
            captured,
        });
        self.env.declare(&name, SwiftValue::Function(id), false);
    }

    /// `let`/`var name [= init]`.
    fn eval_decl(&mut self, node: &Node<'a>, mutable: bool) -> Eval {
        let name = node
            .decl_name()
            .ok_or_else(|| EvalError::Unsupported("declaration without a name".into()))?;
        let value = match node.children().last() {
            Some(init) if is_expr(&init) => {
                let v = self.eval(&init)?;
                self.coerce_to_decl_type(node, v)
            }
            _ => SwiftValue::Void,
        };
        self.env.declare(&name, value, mutable);
        Ok(SwiftValue::Void)
    }

    /// If the declaration carries an explicit integer type annotation, retag the
    /// initializer's width to match it. (msf collapses fixed-width ints to
    /// `Int`, so the `TYPE_IDENT` node is the only reliable source.)
    fn coerce_to_decl_type(&self, node: &Node<'a>, value: SwiftValue) -> SwiftValue {
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
    fn eval_ident(&mut self, node: &Node<'a>) -> Eval {
        let name = node
            .text()
            .ok_or_else(|| EvalError::Unsupported("unnamed identifier".into()))?;
        self.env
            .get(&name)
            .ok_or(EvalError::UnknownVariable(name).into())
    }

    /// An integer literal, widened to its msf-resolved type when known.
    fn eval_int_literal(&self, node: &Node<'a>) -> SwiftValue {
        let raw = node.int().unwrap_or(0) as i128;
        let width = node
            .type_name()
            .and_then(|n| IntWidth::from_type_name(&n))
            .unwrap_or(IntWidth::I64);
        SwiftValue::Int(IntValue::new(raw, width))
    }

    /// A binary operation, with short-circuiting `&&`/`||`.
    fn eval_binary(&mut self, node: &Node<'a>) -> Eval {
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
        ops::binary(&op, &l, &r).map_err(trap)
    }

    /// A ternary `cond ? a : b`, evaluating only the taken branch.
    fn eval_ternary(&mut self, node: &Node<'a>) -> Eval {
        let mut kids = node.children();
        let cond = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("ternary without condition".into()))?;
        let then = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("ternary without then-branch".into()))?;
        let els = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("ternary without else-branch".into()))?;
        let c = self.eval(&cond)?;
        let taken = c
            .as_bool()
            .ok_or_else(|| EvalError::Type(format!("ternary needs Bool, got {}", c.type_name())))?;
        if taken {
            self.eval(&then)
        } else {
            self.eval(&els)
        }
    }

    /// A unary operation (`-x`, `!b`, `~n`).
    fn eval_unary(&mut self, node: &Node<'a>) -> Eval {
        let op = node
            .op_text()
            .or_else(|| node.text())
            .ok_or_else(|| EvalError::Unsupported("unary without operator".into()))?;
        let operand = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("unary without operand".into()))?;
        let v = self.eval(&operand)?;
        ops::unary(&op, &v).map_err(trap)
    }

    /// Assignment: plain `=` and compound `+=`, `-=`, … to a binding.
    fn eval_assign(&mut self, node: &Node<'a>) -> Eval {
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
            )
            .into());
        }
        let name = target
            .text()
            .ok_or_else(|| EvalError::Unsupported("unnamed assignment target".into()))?;

        let new_value = if op == "=" {
            self.eval(&rhs)?
        } else {
            let bin_op = op.trim_end_matches('=');
            let current = self
                .env
                .get(&name)
                .ok_or_else(|| EvalError::UnknownVariable(name.clone()))?;
            let r = self.eval(&rhs)?;
            ops::binary(bin_op, &current, &r).map_err(trap)?
        };

        match self.env.assign(&name, new_value) {
            Ok(()) => Ok(SwiftValue::Void),
            Err(BindError::Immutable(n)) => Err(EvalError::Immutable(n).into()),
            Err(BindError::Unbound(n)) => Err(EvalError::UnknownVariable(n).into()),
        }
    }

    /// Member access: static integer members (`Int.max`/`Int.min`) and
    /// `Array.count`.
    fn eval_member(&mut self, node: &Node<'a>) -> Eval {
        let member = node
            .text()
            .ok_or_else(|| EvalError::Unsupported("member without a name".into()))?;
        let base = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("member without a base".into()))?;

        if base.kind() == NodeKind::IdentExpr {
            if let Some(type_name) = base.text() {
                if self.env.get(&type_name).is_none() {
                    if let Some(w) = IntWidth::from_type_name(&type_name) {
                        return match member.as_str() {
                            "max" => Ok(SwiftValue::Int(IntValue::new(w.max(), w))),
                            "min" => Ok(SwiftValue::Int(IntValue::new(w.min(), w))),
                            _ => {
                                Err(EvalError::Unsupported(format!("{type_name}.{member}")).into())
                            }
                        };
                    }
                }
            }
        }

        let value = self.eval(&base)?;
        match (&value, member.as_str()) {
            (SwiftValue::Array(items), "count") => Ok(SwiftValue::int(items.len() as i128)),
            (SwiftValue::Array(items), "isEmpty") => Ok(SwiftValue::Bool(items.is_empty())),
            (SwiftValue::Str(s), "count") => Ok(SwiftValue::int(s.chars().count() as i128)),
            (SwiftValue::Str(s), "isEmpty") => Ok(SwiftValue::Bool(s.is_empty())),
            _ => Err(
                EvalError::Unsupported(format!("member .{member} on {}", value.type_name())).into(),
            ),
        }
    }

    /// Evaluate a call: a user function, a native, or a conversion initializer.
    fn eval_call(&mut self, node: &Node<'a>) -> Eval {
        let mut children = node.children();
        let callee = children
            .next()
            .ok_or_else(|| EvalError::Unsupported("call with no callee".into()))?;

        // Evaluate arguments, preserving any labels.
        let mut args: Vec<(Option<String>, SwiftValue)> = Vec::new();
        for arg in children {
            let label = arg.arg_label();
            let value = self.eval(&arg)?;
            args.push((label, value));
        }

        if callee.kind() == NodeKind::IdentExpr {
            let name = callee
                .text()
                .ok_or_else(|| EvalError::Unsupported("unnamed callee".into()))?;

            // A bound function value (incl. recursion) takes priority.
            if let Some(SwiftValue::Function(id)) = self.env.get(&name) {
                return self.call_function(id, args);
            }
            // Conversion initializers take exactly one argument.
            if args.len() == 1 {
                if let Some(v) = self.try_conversion(&name, &args[0].1)? {
                    return Ok(v);
                }
            }
            if let Some(native) = self.natives.get(&name).copied() {
                let plain: Vec<SwiftValue> = args.into_iter().map(|(_, v)| v).collect();
                return Ok(native(self.out, &plain));
            }
            return Err(EvalError::UnknownFunction(name).into());
        }

        // Callee is an arbitrary expression — must evaluate to a function value.
        let value = self.eval(&callee)?;
        match value {
            SwiftValue::Function(id) => self.call_function(id, args),
            other => {
                Err(EvalError::Type(format!("`{}` is not callable", other.type_name())).into())
            }
        }
    }

    /// Invoke a user function by its table id with (possibly labeled) arguments.
    fn call_function(&mut self, id: usize, args: Vec<(Option<String>, SwiftValue)>) -> Eval {
        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap(
                "stack overflow: recursion exceeded the maximum call depth".into(),
            ));
        }

        // Bind parameters in a fresh scope over the function's captured chain.
        let captured = self.funcs[id].captured.clone();
        let call_env = Env::with_captured(captured);
        let saved = std::mem::replace(&mut self.env, call_env);

        let outcome = self.bind_and_run(id, args);

        self.env = saved;
        self.depth -= 1;
        outcome
    }

    fn bind_and_run(&mut self, id: usize, args: Vec<(Option<String>, SwiftValue)>) -> Eval {
        // Bind parameters. `params` are looked up by index to avoid borrowing
        // `self.funcs` across the `self.eval` calls for default values.
        let param_count = self.funcs[id].params.len();
        let mut ai = 0;
        for pi in 0..param_count {
            let (label, name, variadic, default) = {
                let p = &self.funcs[id].params[pi];
                (p.label.clone(), p.name.clone(), p.variadic, p.default)
            };
            if variadic {
                let mut pack = Vec::new();
                while ai < args.len() && args[ai].0.is_none() {
                    pack.push(args[ai].1.clone());
                    ai += 1;
                }
                self.env
                    .declare(&name, SwiftValue::Array(Rc::new(pack)), false);
            } else if ai < args.len() {
                let _ = &label;
                self.env.declare(&name, args[ai].1.clone(), false);
                ai += 1;
            } else if let Some(def) = default {
                let v = self.eval(&def)?;
                self.env.declare(&name, v, false);
            } else {
                return Err(EvalError::Type(format!("missing argument for `{name}`")).into());
            }
        }

        let body = self.funcs[id].body;
        match body {
            Some(b) => match self.eval(&b) {
                Ok(_) => Ok(SwiftValue::Void),
                Err(Signal::Return(v)) => Ok(v),
                Err(other) => Err(other),
            },
            None => Ok(SwiftValue::Void),
        }
    }

    /// Attempt a numeric/string conversion `Type(value)`. Returns `Ok(None)` if
    /// `name` is not a known conversion type.
    fn try_conversion(&self, name: &str, value: &SwiftValue) -> Result<Option<SwiftValue>, Signal> {
        if let Some(w) = IntWidth::from_type_name(name) {
            let raw = match value {
                SwiftValue::Int(i) => i.raw,
                SwiftValue::Double(d) => d.trunc() as i128,
                SwiftValue::Bool(b) => *b as i128,
                _ => {
                    return Err(EvalError::Type(format!(
                        "cannot convert {} to {name}",
                        value.type_name()
                    ))
                    .into())
                }
            };
            let v = IntValue::new(raw, w);
            if !v.in_range() {
                return Err(trap(format!("{raw} is not representable as {name}")));
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
                        ))
                        .into())
                    }
                };
                Ok(Some(SwiftValue::Double(d)))
            }
            "String" => Ok(Some(SwiftValue::Str(value.to_string()))),
            _ => Ok(None),
        }
    }
}

/// Whether a node is an expression (vs. a type annotation or other non-value
/// child appearing under a declaration).
fn is_expr(node: &Node) -> bool {
    !matches!(node.kind(), NodeKind::TypeIdent)
}

/// Decode a Swift string literal's *source text* (including its delimiters) into
/// the runtime string it denotes: strips quotes and processes escapes.
fn decode_string_literal(raw: &str) -> String {
    if raw.starts_with('#') {
        let hashes = raw.chars().take_while(|&c| c == '#').count();
        let inner = &raw[hashes..raw.len().saturating_sub(hashes)];
        let inner = inner
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(inner);
        return inner.to_string();
    }
    if let Some(body) = raw
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
    {
        return decode_escapes(strip_multiline_indent(body));
    }
    let body = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw);
    decode_escapes(body)
}

fn strip_multiline_indent(body: &str) -> &str {
    body.trim_start_matches('\n').trim_end_matches([' ', '\t'])
}

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

    #[test]
    fn factorial_recurses() {
        let out = run(
            "func factorial(_ n: Int) -> Int { return n == 0 ? 1 : n * factorial(n - 1) }\nprint(factorial(5))\n",
        )
        .unwrap();
        assert_eq!(out, "120\n");
    }

    #[test]
    fn labels_defaults_and_calls() {
        let out = run(
            "func add(_ a: Int, to b: Int = 5) -> Int { return a + b }\nprint(add(1))\nprint(add(2, to: 3))\n",
        )
        .unwrap();
        assert_eq!(out, "6\n5\n");
    }

    #[test]
    fn first_class_functions() {
        let out = run(
            "func inc(_ n: Int) -> Int { return n + 1 }\nfunc apply(_ f: (Int) -> Int, _ x: Int) -> Int { return f(x) }\nprint(apply(inc, 5))\n",
        )
        .unwrap();
        assert_eq!(out, "6\n");
    }

    #[test]
    fn variadic_collects_into_array() {
        let out =
            run("func n(_ xs: Int...) -> Int { return xs.count }\nprint(n(1, 2, 3))\nprint(n())\n")
                .unwrap();
        assert_eq!(out, "3\n0\n");
    }

    #[test]
    fn mutual_recursion_and_forward_reference() {
        let out = run(
            "func isEven(_ n: Int) -> Bool { return n == 0 ? true : isOdd(n - 1) }\nfunc isOdd(_ n: Int) -> Bool { return n == 0 ? false : isEven(n - 1) }\nprint(isEven(10))\n",
        )
        .unwrap();
        assert_eq!(out, "true\n");
    }

    #[test]
    fn deep_recursion_traps_not_crashes() {
        // Run on a generous stack so the depth guard fires before any native
        // overflow, proving recursion yields a catchable error.
        let handle = std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| run("func loop(_ n: Int) -> Int { return loop(n + 1) }\nprint(loop(0))\n"))
            .unwrap();
        let result = handle.join().unwrap();
        assert!(matches!(result, Err(EvalError::Trap(_))), "got {result:?}");
    }
}
