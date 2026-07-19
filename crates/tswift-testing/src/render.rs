//! A small AST pretty-printer for `#expect`/`#require` failure messages.
//!
//! Nodes carry only a start line/col (no end offset), and multi-file
//! concatenation makes raw source slicing unreliable, so we reconstruct a
//! source-like spelling from the typed AST (plan §3.5). Unhandled shapes fall
//! back to `"<expression>"`.

use tswift_frontend::{Node, NodeKind};

/// Render `node` as a source-like Swift expression string.
pub fn expr(node: &Node<'_>) -> String {
    match node.kind() {
        NodeKind::BinaryExpr => {
            let op = node.text().unwrap_or_default();
            let mut it = node.children();
            let lhs = it.next();
            let rhs = it.next();
            match (lhs, rhs) {
                (Some(l), Some(r)) => format!("{} {} {}", expr(&l), op, expr(&r)),
                _ => op,
            }
        }
        NodeKind::PrefixExpr => {
            let op = node.text().unwrap_or_default();
            match node.first_child() {
                Some(c) => format!("{op}{}", expr(&c)),
                None => op,
            }
        }
        NodeKind::IdentExpr => node.text().unwrap_or_else(|| "_".into()),
        NodeKind::MemberExpr => match node.first_child() {
            Some(base) => format!("{}.{}", expr(&base), node.text().unwrap_or_default()),
            None => format!(".{}", node.text().unwrap_or_default()),
        },
        NodeKind::CallExpr => {
            let children: Vec<Node<'_>> = node.children().collect();
            let Some((callee, args)) = children.split_first() else {
                return "<expression>".into();
            };
            let rendered: Vec<String> = args.iter().map(render_arg).collect();
            format!("{}({})", expr(callee), rendered.join(", "))
        }
        NodeKind::SubscriptExpr => {
            let children: Vec<Node<'_>> = node.children().collect();
            let Some((base, idx)) = children.split_first() else {
                return "<expression>".into();
            };
            let rendered: Vec<String> = idx.iter().map(expr).collect();
            format!("{}[{}]", expr(base), rendered.join(", "))
        }
        NodeKind::TupleExpr => {
            let rendered: Vec<String> = node.children().map(|c| render_arg(&c)).collect();
            format!("({})", rendered.join(", "))
        }
        NodeKind::ArrayLiteral => {
            let rendered: Vec<String> = node.children().map(|c| expr(&c)).collect();
            format!("[{}]", rendered.join(", "))
        }
        NodeKind::TryExpr | NodeKind::AwaitExpr => match node.first_child() {
            Some(c) => expr(&c),
            None => "<expression>".into(),
        },
        NodeKind::IntegerLiteral | NodeKind::FloatLiteral | NodeKind::BoolLiteral => {
            node.text().unwrap_or_else(|| "<literal>".into())
        }
        NodeKind::NilLiteral => "nil".into(),
        NodeKind::StringLiteral => node.text().unwrap_or_else(|| "\"\"".into()),
        _ => "<expression>".into(),
    }
}

/// Render a call/tuple argument, honouring an optional label (`count: 3`).
fn render_arg(node: &Node<'_>) -> String {
    match node.arg_label() {
        Some(label) => format!("{label}: {}", expr(node)),
        None => expr(node),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_frontend::Analysis;

    fn expr_of(src: &str) -> String {
        // Wrap the expression in a `#expect` so it lands as a directive child
        // we can pull out and render.
        let program = format!("func _f() {{ #expect({src}) }}\n");
        let analysis = Analysis::analyze(&program, "r.swift").unwrap();
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let directive = find(analysis.root(), NodeKind::CompilerDirective).unwrap();
        expr(&directive.first_child().unwrap())
    }

    fn find(node: Node<'static>, kind: NodeKind) -> Option<Node<'static>> {
        if node.kind() == kind {
            return Some(node);
        }
        node.children().find_map(|c| find(c, kind))
    }

    #[test]
    fn renders_binary_comparison() {
        assert_eq!(expr_of("add(1, 1) == 3"), "add(1, 1) == 3");
    }

    #[test]
    fn renders_member_and_unary() {
        assert_eq!(expr_of("!user.active"), "!user.active");
    }

    #[test]
    fn renders_subscript_and_bare_ident() {
        assert_eq!(expr_of("items[0] == flag"), "items[0] == flag");
    }
}
