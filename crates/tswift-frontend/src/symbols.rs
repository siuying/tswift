//! `list_symbols` — a cheap, outline-style symbol index over one or more
//! Swift source files.
//!
//! This is deliberately *not* a new semantic indexer: it walks the same parse
//! AST `Analysis` already exposes (`Node`/`NodeKind`), picking out
//! declaration nodes and reading the facts already decoded onto them
//! (`decl_name`, `type_name`, `param_info`, …). Each file is analyzed on its
//! own (`Analysis::analyze`), so every [`Symbol`]'s `line` is already
//! file-local — no combined-source remapping is needed the way
//! `analyze_program`'s diagnostics require, because listing symbols has no
//! cross-file semantic dependency to preserve.
//!
//! Only *declarations* are listed — a function's body (its `Block` of
//! statements) is not descended into, so local variables and nested helper
//! functions do not appear. This keeps the result an outline (types,
//! members, top-level functions/properties), matching what an IDE "Outline"
//! or `ctags`-style view shows.

use serde::Serialize;

use crate::{Node, NodeKind, SourceFile};

/// The declaration category of a [`Symbol`]. Serializes as its lowercase
/// Swift keyword (`func`, `struct`, `typealias`, …) — the JSON `kind` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Func,
    Struct,
    Class,
    Enum,
    Protocol,
    Actor,
    Extension,
    Var,
    Let,
    Case,
    Init,
    Deinit,
    Subscript,
    TypeAlias,
    AssociatedType,
}

impl SymbolKind {
    /// The lowercase Swift keyword this kind names, used as the JSON `kind`.
    pub fn name(self) -> &'static str {
        match self {
            SymbolKind::Func => "func",
            SymbolKind::Struct => "struct",
            SymbolKind::Class => "class",
            SymbolKind::Enum => "enum",
            SymbolKind::Protocol => "protocol",
            SymbolKind::Actor => "actor",
            SymbolKind::Extension => "extension",
            SymbolKind::Var => "var",
            SymbolKind::Let => "let",
            SymbolKind::Case => "case",
            SymbolKind::Init => "init",
            SymbolKind::Deinit => "deinit",
            SymbolKind::Subscript => "subscript",
            SymbolKind::TypeAlias => "typealias",
            SymbolKind::AssociatedType => "associatedtype",
        }
    }

    /// Whether this kind's declaration hosts nested member declarations that
    /// should also be listed, with this symbol's name as their `container`.
    fn is_container(self) -> bool {
        matches!(
            self,
            SymbolKind::Struct
                | SymbolKind::Class
                | SymbolKind::Enum
                | SymbolKind::Protocol
                | SymbolKind::Actor
                | SymbolKind::Extension
        )
    }

    fn of(kind: NodeKind) -> Option<SymbolKind> {
        match kind {
            NodeKind::FuncDecl => Some(SymbolKind::Func),
            NodeKind::StructDecl => Some(SymbolKind::Struct),
            NodeKind::ClassDecl => Some(SymbolKind::Class),
            NodeKind::EnumDecl => Some(SymbolKind::Enum),
            NodeKind::ProtocolDecl => Some(SymbolKind::Protocol),
            NodeKind::ActorDecl => Some(SymbolKind::Actor),
            NodeKind::ExtensionDecl => Some(SymbolKind::Extension),
            NodeKind::VarDecl => Some(SymbolKind::Var),
            NodeKind::LetDecl => Some(SymbolKind::Let),
            NodeKind::EnumCaseDecl => Some(SymbolKind::Case),
            NodeKind::InitDecl => Some(SymbolKind::Init),
            NodeKind::DeinitDecl => Some(SymbolKind::Deinit),
            NodeKind::SubscriptDecl => Some(SymbolKind::Subscript),
            NodeKind::TypeAliasDecl => Some(SymbolKind::TypeAlias),
            NodeKind::AssociatedTypeDecl => Some(SymbolKind::AssociatedType),
            _ => None,
        }
    }
}

/// One declaration found while listing symbols.
///
/// Serializes (via `serde`) to the shared `list_symbols` wire object
/// `{"name","kind","file","line","container"?,"signature"?}` — field order
/// is declaration order, and the two `Option` fields are omitted when `None`
/// (`skip_serializing_if`), matching the pre-serde hand-written shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// The `path` of the [`SourceFile`] this symbol was declared in.
    pub file: String,
    /// 1-based, file-local source line.
    pub line: u32,
    /// The name of the nearest enclosing container declaration (a
    /// struct/class/enum/protocol/actor/extension), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    /// A cheap, non-canonical text rendering of the declaration's shape
    /// (parameter list/return type for a func, element type for a var/let,
    /// …). Built from already-decoded node text, not re-derived from a type
    /// checker — good for a UI label, not for overload resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// Serialize `symbols` as a JSON array of objects
/// (`{"name","kind","file","line","container"?,"signature"?}`), the wire
/// format the wasm/FFI/CLI `list_symbols` entry points all share.
pub fn to_json(symbols: &[Symbol]) -> String {
    serde_json::to_string(symbols).expect("Symbol serialization is infallible")
}

/// List every declaration symbol across `files`. Each file is analyzed
/// independently (a syntax error in one file still yields symbols for the
/// others); files that fail to analyze contribute no symbols.
pub fn list_symbols(files: &[SourceFile]) -> Vec<Symbol> {
    let mut out = Vec::new();
    for file in files {
        let Ok(analysis) = crate::Analysis::analyze(&file.source, &file.path) else {
            continue;
        };
        walk(analysis.root(), &file.path, None, &mut out);
    }
    out
}

fn walk(node: Node<'_>, file: &str, container: Option<&str>, out: &mut Vec<Symbol>) {
    for child in node.children() {
        let Some(kind) = SymbolKind::of(child.kind()) else {
            // Not a declaration node itself, but declarations can be nested
            // inside a `#if` compiler-directive branch; descend through
            // those (and only those) non-declaration wrappers so their
            // members are not skipped.
            if child.kind() == NodeKind::CompilerDirective {
                walk(child, file, container, out);
            }
            continue;
        };
        // `init`/`deinit` carry no name token of their own (the dump shows
        // e.g. `InitDecl L4` with no text) — their name is the keyword itself.
        let name = match kind {
            SymbolKind::Init => "init".to_string(),
            SymbolKind::Deinit => "deinit".to_string(),
            SymbolKind::Subscript => "subscript".to_string(),
            _ => match child.decl_name() {
                Some(n) => n,
                None => continue,
            },
        };
        out.push(Symbol {
            name: name.clone(),
            kind,
            file: file.to_string(),
            line: child.line(),
            container: container.map(str::to_string),
            signature: signature_of(&child, kind, &name),
        });
        if kind.is_container() {
            walk(child, file, Some(&name), out);
        }
    }
}

/// Build a cheap signature string for one declaration.
fn signature_of(node: &Node<'_>, kind: SymbolKind, name: &str) -> Option<String> {
    match kind {
        SymbolKind::Func | SymbolKind::Init | SymbolKind::Subscript => {
            let params = param_list(node);
            let head = match kind {
                SymbolKind::Func => format!("func {name}({params})"),
                SymbolKind::Init => format!("init({params})"),
                SymbolKind::Subscript => format!("subscript({params})"),
                _ => unreachable!(),
            };
            Some(match ret_type(node) {
                Some(ret) => format!("{head} -> {ret}"),
                None => head,
            })
        }
        SymbolKind::Var | SymbolKind::Let => {
            // Prefer the written type annotation (a `TypeRef` child) over the
            // sema-resolved `type_name()`: the latter is only populated for
            // the handful of built-in scalar `Type`s sema models, so an
            // array/dictionary/optional annotation would otherwise be
            // silently dropped from this cheap signature.
            let ty = node
                .children()
                .find(|c| c.kind() == NodeKind::TypeRef)
                .and_then(|c| c.text())
                .or_else(|| node.type_name());
            Some(match ty {
                Some(ty) => format!("{name}: {ty}"),
                None => name.to_string(),
            })
        }
        SymbolKind::Case => {
            let assoc: Vec<String> = node
                .children()
                .filter(|c| c.kind() == NodeKind::TypeRef)
                .filter_map(|c| c.text())
                .collect();
            Some(if assoc.is_empty() {
                format!("case {name}")
            } else {
                format!("case {name}({})", assoc.join(", "))
            })
        }
        SymbolKind::Struct
        | SymbolKind::Class
        | SymbolKind::Enum
        | SymbolKind::Protocol
        | SymbolKind::Actor
        | SymbolKind::Extension => {
            let conforms: Vec<String> = node
                .children()
                .filter(|c| c.kind() == NodeKind::TypeRef)
                .filter_map(|c| c.text())
                .collect();
            Some(if conforms.is_empty() {
                name.to_string()
            } else {
                format!("{name}: {}", conforms.join(", "))
            })
        }
        SymbolKind::TypeAlias | SymbolKind::AssociatedType | SymbolKind::Deinit => None,
    }
}

/// Render a `FuncDecl`/`InitDecl`/`SubscriptDecl`'s `Param` children as
/// Swift-style parameter source text (`_ x: Int, label y: String...`).
fn param_list(node: &Node<'_>) -> String {
    node.children()
        .filter(|c| c.kind() == NodeKind::Param)
        .map(|p| {
            let info = p.param_info();
            // `arg_label()` is `None` both for the default (external label ==
            // internal name, the common case) and for an explicit `_`
            // (no-label) parameter — the parser doesn't retain a written `_`
            // once it's dropped the label (see `parse_param`). This cheap
            // signature can't tell those apart, so it renders the common
            // case: `name: Type`, omitting a label only when one was written
            // that differs from the internal name.
            let label = match &info.label {
                Some(l) if *l != info.name => format!("{l} "),
                _ => String::new(),
            };
            let ty = p.type_name().unwrap_or_default();
            let variadic = if info.variadic { "..." } else { "" };
            let inout_kw = if info.is_inout { "inout " } else { "" };
            format!("{label}{}: {inout_kw}{ty}{variadic}", info.name)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A `FuncDecl`/`SubscriptDecl`'s resolved return type, unless it's the
/// unwritten default `Void` (omitted from the cheap signature as noise).
fn ret_type(node: &Node<'_>) -> Option<String> {
    node.type_name().filter(|t| t != "Void")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_top_level_and_nested_symbols_with_containers() {
        let src = "struct Box {\n    var items: [Int]\n    func add(_ x: Int, label y: String) -> Bool { return true }\n    init(x: Int) {}\n}\nenum E { case a(Int) }\nprotocol P { func f() }\n";
        let files = [SourceFile::new("main.swift", src)];
        let syms = list_symbols(&files);

        let by_name = |n: &str| syms.iter().find(|s| s.name == n).unwrap();

        let boxs = by_name("Box");
        assert_eq!(boxs.kind, SymbolKind::Struct);
        assert_eq!(boxs.container, None);
        assert_eq!(boxs.line, 1);

        let items = by_name("items");
        assert_eq!(items.kind, SymbolKind::Var);
        assert_eq!(items.container.as_deref(), Some("Box"));
        assert_eq!(items.signature.as_deref(), Some("items: [Int]"));

        let add = by_name("add");
        assert_eq!(add.kind, SymbolKind::Func);
        assert_eq!(add.container.as_deref(), Some("Box"));
        assert_eq!(
            add.signature.as_deref(),
            Some("func add(x: Int, label y: String) -> Bool")
        );

        let init = syms
            .iter()
            .find(|s| s.kind == SymbolKind::Init)
            .expect("an init symbol");
        assert_eq!(init.container.as_deref(), Some("Box"));
        assert_eq!(init.signature.as_deref(), Some("init(x: Int)"));

        let a = by_name("a");
        assert_eq!(a.kind, SymbolKind::Case);
        assert_eq!(a.container.as_deref(), Some("E"));
        assert_eq!(a.signature.as_deref(), Some("case a(Int)"));

        let f = by_name("f");
        assert_eq!(f.container.as_deref(), Some("P"));
    }

    /// Local declarations inside a function body are not listed — only the
    /// outline of declarations, not statement-level locals.
    #[test]
    fn does_not_descend_into_function_bodies() {
        let src = "func outer() {\n    let local = 1\n    func inner() {}\n}\n";
        let files = [SourceFile::new("main.swift", src)];
        let syms = list_symbols(&files);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "outer");
    }

    /// Symbols across two files carry their own file path and file-local line.
    #[test]
    fn symbols_across_two_files_carry_their_own_file_and_line() {
        let files = [
            SourceFile::new("Models.swift", "struct Point {\n    let x: Int\n}\n"),
            SourceFile::new("main.swift", "func run() {}\n"),
        ];
        let syms = list_symbols(&files);
        let point = syms.iter().find(|s| s.name == "Point").unwrap();
        assert_eq!(point.file, "Models.swift");
        assert_eq!(point.line, 1);
        let x = syms.iter().find(|s| s.name == "x").unwrap();
        assert_eq!(x.file, "Models.swift");
        assert_eq!(x.line, 2);
        let run = syms.iter().find(|s| s.name == "run").unwrap();
        assert_eq!(run.file, "main.swift");
        assert_eq!(run.line, 1);
    }

    /// A struct's inherited-protocol list renders into the cheap signature.
    #[test]
    fn container_signature_includes_conformances() {
        let files = [SourceFile::new(
            "main.swift",
            "struct User: Codable, Equatable {\n    let name: String\n}\n",
        )];
        let syms = list_symbols(&files);
        let user = syms.iter().find(|s| s.name == "User").unwrap();
        assert_eq!(user.signature.as_deref(), Some("User: Codable, Equatable"));
    }

    #[test]
    fn to_json_renders_the_wire_shape() {
        let files = [SourceFile::new(
            "main.swift",
            "struct S {\n    let x: Int\n}\n",
        )];
        let syms = list_symbols(&files);
        let json = to_json(&syms);
        assert_eq!(
            json,
            "[{\"name\":\"S\",\"kind\":\"struct\",\"file\":\"main.swift\",\"line\":1,\"signature\":\"S\"},\
{\"name\":\"x\",\"kind\":\"let\",\"file\":\"main.swift\",\"line\":2,\"container\":\"S\",\"signature\":\"x: Int\"}]"
        );
    }

    /// Reconstruct the pre-serde hand-written `to_json` output and compare,
    /// as parsed JSON, against today's serde-derived output — pinning the
    /// wire schema (key set, order-insensitive) across the refactor.
    #[test]
    fn to_json_matches_pre_serde_writer() {
        let files = [SourceFile::new(
            "main.swift",
            "struct Box {\n    let x: Int\n    func f() {}\n}\nenum E { case a(Int) }\n",
        )];
        let syms = list_symbols(&files);
        let old = old_to_json(&syms);
        let old_v: serde_json::Value = serde_json::from_str(&old).unwrap();
        let new_v: serde_json::Value = serde_json::from_str(&to_json(&syms)).unwrap();
        assert_eq!(old_v, new_v);
    }

    /// Frozen copy of the pre-serde hand-rolled `to_json` writer, the oracle
    /// for [`to_json_matches_pre_serde_writer`].
    fn old_to_json(symbols: &[Symbol]) -> String {
        fn esc(s: &str) -> String {
            let mut out = String::from("\"");
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    _ => out.push(c),
                }
            }
            out.push('"');
            out
        }
        use std::fmt::Write as _;
        let mut out = String::from("[");
        for (i, s) in symbols.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"name\":{},\"kind\":{},\"file\":{},\"line\":{}",
                esc(&s.name),
                esc(s.kind.name()),
                esc(&s.file),
                s.line
            );
            if let Some(c) = &s.container {
                let _ = write!(out, ",\"container\":{}", esc(c));
            }
            if let Some(sig) = &s.signature {
                let _ = write!(out, ",\"signature\":{}", esc(sig));
            }
            out.push('}');
        }
        out.push(']');
        out
    }

    /// A file with a syntax error contributes no symbols, but sibling files
    /// still do.
    #[test]
    fn a_broken_file_does_not_block_symbols_from_other_files() {
        let files = [
            SourceFile::new("bad.swift", "let = =\n"),
            SourceFile::new("good.swift", "struct Ok {}\n"),
        ];
        let syms = list_symbols(&files);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Ok");
    }
}
