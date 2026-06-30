//! A structured view over a written type spelling.
//!
//! The parser records a `TypeRef` node's reconstructed source as a flat
//! `String` (see `tswift-parser`'s `parse_type_text`). Historically every
//! consumer that needed to *understand* that spelling — "is it optional?",
//! "what is the array element?", "what are a dictionary's key/value?" — did its
//! own ad-hoc string surgery (`trim_end_matches('?')`, `strip_prefix('[')`,
//! …), so the same type-shape logic was re-derived, and drifted, across many
//! sites.
//!
//! [`TypeRepr`] is that shape, parsed once into a real type. It borrows the
//! original spelling (so [`TypeRepr::text`] and [`TypeRepr::name`] hand back
//! slices of the input with no allocation) and exposes the shape as queries —
//! [`is_optional`](TypeRepr::is_optional), [`array_element`](TypeRepr::array_element),
//! [`dictionary`](TypeRepr::dictionary) — so consumers ask instead of re-parse.
//!
//! Scope: the surface the parser actually emits — names (dotted, generic),
//! optionals (`T?`), arrays (`[T]`), dictionaries (`[K: V]`), tuples, function
//! types (`(A) -> B`), and protocol composition (`P & Q`). Anything it does not
//! recognise falls back to [`TypeReprKind::Named`] carrying the raw slice, so a
//! query is always answerable.

/// A parsed type spelling. `text` is the trimmed slice of the original spelling
/// this node spans; `kind` is its shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRepr<'a> {
    text: &'a str,
    kind: TypeReprKind<'a>,
}

/// The shape of a [`TypeRepr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeReprKind<'a> {
    /// A nominal type, optionally dotted/generic — `Int`, `Foo.Bar`,
    /// `Array<Int>`. `name` is the base spelling up to any top-level generic
    /// argument list; `args` are the parsed generic arguments.
    Named {
        /// The nominal name with generic arguments removed (`Array` for
        /// `Array<Int>`), preserving any dotted qualification.
        name: &'a str,
        /// Parsed generic arguments, if a `<…>` list was present.
        args: Vec<TypeRepr<'a>>,
    },
    /// An optional `T?`.
    Optional(Box<TypeRepr<'a>>),
    /// An array `[T]`.
    Array(Box<TypeRepr<'a>>),
    /// A dictionary `[K: V]`.
    Dictionary {
        /// The key type.
        key: Box<TypeRepr<'a>>,
        /// The value type.
        value: Box<TypeRepr<'a>>,
    },
    /// A tuple `(A, B, …)`. A parenthesised single type `(T)` parses as `T`
    /// rather than a one-element tuple, matching Swift.
    Tuple(Vec<TypeRepr<'a>>),
    /// A function type `(Params) -> Ret`.
    Function {
        /// The parameter types.
        params: Vec<TypeRepr<'a>>,
        /// The return type.
        ret: Box<TypeRepr<'a>>,
    },
    /// A protocol composition `P & Q & …`.
    Composition(Vec<TypeRepr<'a>>),
}

impl<'a> TypeRepr<'a> {
    /// Parse a written type spelling into its shape. Never fails: an
    /// unrecognised spelling becomes a [`TypeReprKind::Named`] holding the raw
    /// slice.
    pub fn parse(spelling: &'a str) -> TypeRepr<'a> {
        let t = spelling.trim();

        // Function `(A, B) -> R` — lowest precedence, so split first.
        if let Some(idx) = arrow_pos(t) {
            let params = parse_param_list(t[..idx].trim());
            let ret = Box::new(TypeRepr::parse(t[idx + 2..].trim()));
            return TypeRepr {
                text: t,
                kind: TypeReprKind::Function { params, ret },
            };
        }

        // Protocol composition `P & Q`.
        let amp = top_level_indices(t, b'&');
        if !amp.is_empty() {
            let members = split_at(t, &amp).into_iter().map(TypeRepr::parse).collect();
            return TypeRepr {
                text: t,
                kind: TypeReprKind::Composition(members),
            };
        }

        // Optional postfix `T?`. (A trailing `!` IUO marker is left as part of
        // the name, matching the historical `trim_end_matches('?')` consumers.)
        if let Some(base) = t.strip_suffix('?') {
            return TypeRepr {
                text: t,
                kind: TypeReprKind::Optional(Box::new(TypeRepr::parse(base.trim_end()))),
            };
        }

        // Bracketed `[T]` array or `[K: V]` dictionary.
        if let Some(inner) = bracket_inner(t) {
            let colons = top_level_indices(inner, b':');
            if let Some(&ci) = colons.first() {
                return TypeRepr {
                    text: t,
                    kind: TypeReprKind::Dictionary {
                        key: Box::new(TypeRepr::parse(inner[..ci].trim())),
                        value: Box::new(TypeRepr::parse(inner[ci + 1..].trim())),
                    },
                };
            }
            return TypeRepr {
                text: t,
                kind: TypeReprKind::Array(Box::new(TypeRepr::parse(inner.trim()))),
            };
        }

        // Parenthesised group: a tuple, or `(T)` which is just `T`.
        if let Some(inner) = paren_inner(t) {
            if inner.trim().is_empty() {
                return TypeRepr {
                    text: t,
                    kind: TypeReprKind::Tuple(Vec::new()),
                };
            }
            let parts = split_commas(inner);
            if parts.len() == 1 {
                return TypeRepr::parse(strip_tuple_label(parts[0]));
            }
            let elems = parts
                .into_iter()
                .map(|p| TypeRepr::parse(strip_tuple_label(p)))
                .collect();
            return TypeRepr {
                text: t,
                kind: TypeReprKind::Tuple(elems),
            };
        }

        // Otherwise a nominal name, possibly generic.
        let (name, args) = parse_named(t);
        TypeRepr {
            text: t,
            kind: TypeReprKind::Named { name, args },
        }
    }

    /// The trimmed source slice this node spans.
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// The shape of this type.
    pub fn kind(&self) -> &TypeReprKind<'a> {
        &self.kind
    }

    /// Whether this is an optional `T?`.
    pub fn is_optional(&self) -> bool {
        matches!(self.kind, TypeReprKind::Optional(_))
    }

    /// The wrapped type if this is an optional, removing a single `?` layer;
    /// otherwise `self`.
    pub fn unwrap_optional(&self) -> &TypeRepr<'a> {
        match &self.kind {
            TypeReprKind::Optional(inner) => inner,
            _ => self,
        }
    }

    /// This type with every leading optional `?` layer removed (`T??` → `T`).
    pub fn strip_optionals(&self) -> &TypeRepr<'a> {
        let mut cur = self;
        while let TypeReprKind::Optional(inner) = &cur.kind {
            cur = inner;
        }
        cur
    }

    /// The element type if this is an array `[T]`; otherwise `None`.
    pub fn array_element(&self) -> Option<&TypeRepr<'a>> {
        match &self.kind {
            TypeReprKind::Array(inner) => Some(inner),
            _ => None,
        }
    }

    /// The `(key, value)` types if this is a dictionary `[K: V]`; else `None`.
    pub fn dictionary(&self) -> Option<(&TypeRepr<'a>, &TypeRepr<'a>)> {
        match &self.kind {
            TypeReprKind::Dictionary { key, value } => Some((key, value)),
            _ => None,
        }
    }

    /// The nominal name (generic arguments removed) if this is a named type.
    pub fn name(&self) -> Option<&'a str> {
        match &self.kind {
            TypeReprKind::Named { name, .. } => Some(name),
            _ => None,
        }
    }
}

/// Position of the top-level `->` arrow, if any.
fn arrow_pos(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'-' && i + 1 < b.len() && b[i + 1] == b'>' {
            if depth == 0 {
                return Some(i);
            }
            i += 2;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'<' => depth += 1,
            b')' | b']' | b'>' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Byte positions of `sep` that sit at bracket depth 0. Arrows (`->`) are
/// consumed as a unit so the `>` never disturbs the depth count.
fn top_level_indices(s: &str, sep: u8) -> Vec<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'-' && i + 1 < b.len() && b[i + 1] == b'>' {
            i += 2;
            continue;
        }
        let c = b[i];
        match c {
            b'(' | b'[' | b'<' => depth += 1,
            b')' | b']' | b'>' => depth -= 1,
            _ if c == sep && depth == 0 => out.push(i),
            _ => {}
        }
        i += 1;
    }
    out
}

/// Split `s` into the slices between the given separator byte positions.
fn split_at<'a>(s: &'a str, seps: &[usize]) -> Vec<&'a str> {
    let mut parts = Vec::with_capacity(seps.len() + 1);
    let mut start = 0;
    for &idx in seps {
        parts.push(s[start..idx].trim());
        start = idx + 1;
    }
    parts.push(s[start..].trim());
    parts
}

/// Split `s` on top-level commas.
fn split_commas(s: &str) -> Vec<&str> {
    let commas = top_level_indices(s, b',');
    split_at(s, &commas)
}

/// If `s` is `[ … ]` with the brackets balanced across the whole span, the
/// inner slice; else `None`.
fn bracket_inner(s: &str) -> Option<&str> {
    delimited_inner(s, b'[', b']')
}

/// If `s` is `( … )` balanced across the whole span, the inner slice.
fn paren_inner(s: &str) -> Option<&str> {
    delimited_inner(s, b'(', b')')
}

fn delimited_inner(s: &str, open: u8, close: u8) -> Option<&str> {
    let b = s.as_bytes();
    if b.first() != Some(&open) || b.last() != Some(&close) {
        return None;
    }
    // The opening delimiter must close exactly at the end, not earlier (so
    // `[A] & [B]` is not treated as one bracketed group).
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        if c == b'(' || c == b'[' || c == b'<' {
            depth += 1;
        } else if c == b')' || c == b']' || c == b'>' {
            depth -= 1;
            if depth == 0 && i != b.len() - 1 {
                return None;
            }
        }
    }
    Some(&s[1..s.len() - 1])
}

/// Drop a tuple element label (`name: Type` → `Type`); leave `Type` unchanged.
fn strip_tuple_label(part: &str) -> &str {
    let colons = top_level_indices(part, b':');
    match colons.first() {
        Some(&ci) => part[ci + 1..].trim(),
        None => part.trim(),
    }
}

/// Split a nominal spelling into its base name (generics removed) and parsed
/// generic arguments.
fn parse_named(s: &str) -> (&str, Vec<TypeRepr<'_>>) {
    let trimmed = s.trim_end();
    if trimmed.ends_with('>') {
        // The first `<` at depth 0 opens the generic argument list.
        if let Some(open) = trimmed.as_bytes().iter().position(|&c| c == b'<') {
            let name = s[..open].trim();
            let inner = &s[open + 1..trimmed.len() - 1];
            let args = split_commas(inner)
                .into_iter()
                .map(TypeRepr::parse)
                .collect();
            return (name, args);
        }
    }
    (s.trim(), Vec::new())
}

/// Parse a function parameter list: `(A, B)` / `()` / a bare `A`.
fn parse_param_list(s: &str) -> Vec<TypeRepr<'_>> {
    if let Some(inner) = paren_inner(s) {
        if inner.trim().is_empty() {
            return Vec::new();
        }
        return split_commas(inner)
            .into_iter()
            .map(|p| TypeRepr::parse(strip_tuple_label(p)))
            .collect();
    }
    vec![TypeRepr::parse(s)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_scalar() {
        let r = TypeRepr::parse("Int");
        assert_eq!(r.name(), Some("Int"));
        assert!(!r.is_optional());
        assert_eq!(r.text(), "Int");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(TypeRepr::parse("  String  ").name(), Some("String"));
    }

    #[test]
    fn optional_unwraps_one_layer() {
        let r = TypeRepr::parse("Int?");
        assert!(r.is_optional());
        assert_eq!(r.unwrap_optional().name(), Some("Int"));
        assert_eq!(r.strip_optionals().text(), "Int");
    }

    #[test]
    fn nested_optionals_strip_all() {
        let r = TypeRepr::parse("Int??");
        assert!(r.is_optional());
        assert_eq!(r.strip_optionals().name(), Some("Int"));
    }

    #[test]
    fn array_element_type() {
        let r = TypeRepr::parse("[String]");
        assert_eq!(r.array_element().and_then(TypeRepr::name), Some("String"));
        assert!(r.dictionary().is_none());
    }

    #[test]
    fn optional_array_is_not_an_array_at_top_level() {
        // `[Int]?` is an Optional whose payload is the array.
        let r = TypeRepr::parse("[Int]?");
        assert!(r.is_optional());
        assert!(r.array_element().is_none());
        assert_eq!(
            r.strip_optionals().array_element().and_then(TypeRepr::name),
            Some("Int")
        );
    }

    #[test]
    fn dictionary_key_value() {
        let r = TypeRepr::parse("[String: Int]");
        let (k, v) = r.dictionary().expect("dictionary");
        assert_eq!(k.name(), Some("String"));
        assert_eq!(v.name(), Some("Int"));
        assert!(r.array_element().is_none());
    }

    #[test]
    fn dictionary_with_nested_value() {
        let r = TypeRepr::parse("[String: [Int]]");
        let (k, v) = r.dictionary().expect("dictionary");
        assert_eq!(k.name(), Some("String"));
        assert_eq!(v.array_element().and_then(TypeRepr::name), Some("Int"));
    }

    #[test]
    fn generic_named() {
        let r = TypeRepr::parse("Array<Int>");
        assert_eq!(r.name(), Some("Array"));
        match r.kind() {
            TypeReprKind::Named { args, .. } => {
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].name(), Some("Int"));
            }
            other => panic!("expected Named, got {other:?}"),
        }
    }

    #[test]
    fn dotted_named_preserved() {
        assert_eq!(TypeRepr::parse("Swift.String").name(), Some("Swift.String"));
    }

    #[test]
    fn tuple_elements() {
        let r = TypeRepr::parse("(Int, String)");
        match r.kind() {
            TypeReprKind::Tuple(elems) => {
                assert_eq!(elems.len(), 2);
                assert_eq!(elems[0].name(), Some("Int"));
                assert_eq!(elems[1].name(), Some("String"));
            }
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    #[test]
    fn parenthesised_single_is_inner() {
        assert_eq!(TypeRepr::parse("(Int)").name(), Some("Int"));
    }

    #[test]
    fn labelled_tuple_drops_labels() {
        let r = TypeRepr::parse("(x: Int, y: Int)");
        match r.kind() {
            TypeReprKind::Tuple(elems) => {
                assert_eq!(elems[0].name(), Some("Int"));
                assert_eq!(elems[1].name(), Some("Int"));
            }
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    #[test]
    fn function_type() {
        let r = TypeRepr::parse("(Int, String) -> Bool");
        match r.kind() {
            TypeReprKind::Function { params, ret } => {
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].name(), Some("Int"));
                assert_eq!(ret.name(), Some("Bool"));
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn function_returning_optional() {
        let r = TypeRepr::parse("() -> Int?");
        match r.kind() {
            TypeReprKind::Function { params, ret } => {
                assert!(params.is_empty());
                assert!(ret.is_optional());
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn generic_with_function_argument() {
        // The `->` inside the generic list must not be read as a top-level
        // arrow, and its `>` must not unbalance the depth counter.
        let r = TypeRepr::parse("Array<(Int) -> Int>");
        assert_eq!(r.name(), Some("Array"));
        match r.kind() {
            TypeReprKind::Named { args, .. } => {
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0].kind(), TypeReprKind::Function { .. }));
            }
            other => panic!("expected Named, got {other:?}"),
        }
    }

    #[test]
    fn composition() {
        let r = TypeRepr::parse("P & Q");
        match r.kind() {
            TypeReprKind::Composition(parts) => {
                assert_eq!(parts.len(), 2);
                assert_eq!(parts[0].name(), Some("P"));
                assert_eq!(parts[1].name(), Some("Q"));
            }
            other => panic!("expected Composition, got {other:?}"),
        }
    }

    #[test]
    fn array_of_composition_is_not_a_bracket_group_split() {
        // `[A] & [B]` must split on `&`, not be read as one bracketed group.
        let r = TypeRepr::parse("[A] & [B]");
        assert!(matches!(r.kind(), TypeReprKind::Composition(_)));
    }
}
