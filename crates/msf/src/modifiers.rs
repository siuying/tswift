//! Decoded view of an [`ASTNode.modifiers`](crate::Node::modifiers) bitmask.
//!
//! msf packs every declaration/closure/parameter modifier into one `u32`, and
//! **reuses some bits across unrelated node kinds** (see the `MOD_*` table in
//! `vendor/msf/include/msf.h` §9): bit 22 is `weak`-capture on an
//! `AST_CLOSURE_CAPTURE`, `borrowing` on an `AST_PARAM`, and `testable` on an
//! `AST_IMPORT_DECL`. A flat bit→name table therefore cannot be correct — it
//! must be read against the node's [`NodeKind`].
//!
//! [`ModifierSet`] decodes context-aware names **without ever discarding truth**:
//! whatever bits it could not name for a given kind are preserved in
//! [`ModifierSet::unknown_bits`], and the original mask is always available via
//! [`ModifierSet::raw`]. A dump can therefore surface every set bit, named or
//! not, instead of silently dropping the ones an older table forgot.

use crate::NodeKind;

// ── Bit positions, mirrored from vendor/msf/include/msf.h §9 ───────────────
// Global bits: same meaning on every node kind.
const MOD_PUBLIC: u32 = 1 << 0;
const MOD_PRIVATE: u32 = 1 << 1;
const MOD_INTERNAL: u32 = 1 << 2;
const MOD_FILEPRIVATE: u32 = 1 << 3;
const MOD_OPEN: u32 = 1 << 4;
const MOD_STATIC: u32 = 1 << 5;
const MOD_FINAL: u32 = 1 << 6;
const MOD_OVERRIDE: u32 = 1 << 7;
const MOD_MUTATING: u32 = 1 << 8;
const MOD_NONMUTATING: u32 = 1 << 9;
const MOD_LAZY: u32 = 1 << 10;
const MOD_WEAK: u32 = 1 << 11;
const MOD_UNOWNED: u32 = 1 << 12;
const MOD_ASYNC: u32 = 1 << 13;
const MOD_THROWS: u32 = 1 << 14;
const MOD_RETHROWS: u32 = 1 << 15;
const MOD_INDIRECT: u32 = 1 << 16;
const MOD_REQUIRED: u32 = 1 << 17;
const MOD_CONVENIENCE: u32 = 1 << 18;
const MOD_DYNAMIC: u32 = 1 << 19;
const MOD_NONISOLATED: u32 = 1 << 20;
const MOD_ISOLATED: u32 = 1 << 21;
const MOD_MAIN_ACTOR: u32 = 1 << 25;
const MOD_ESCAPING: u32 = 1 << 26;
const MOD_AUTOCLOSURE: u32 = 1 << 27;
const MOD_VARIADIC: u32 = 1 << 28;
const MOD_FAILABLE: u32 = 1 << 29;
const MOD_SUPPRESSED_CONFORMANCE: u32 = 1 << 31;

// Context-dependent bits: meaning depends on the owning node kind.
const BIT_22: u32 = 1 << 22; // testable / capture-weak / borrowing / assoc-type
const BIT_23: u32 = 1 << 23; // capture-unowned / consuming / sendable
const BIT_24: u32 = 1 << 24; // capture-safe / protocol-prop-set
const BIT_30: u32 = 1 << 30; // package (access) / implicitly-unwrapped-failable (init)

/// A node's modifier bitmask decoded into names, with unrecognised bits kept.
///
/// Construct it from a node via [`Node::modifier_set`](crate::Node::modifier_set).
/// Decoding is context-aware: the same raw bit yields different names depending
/// on the node's [`NodeKind`]. Bits with no known meaning for that kind are not
/// dropped — they live in [`unknown_bits`](Self::unknown_bits), and the full
/// original mask is always [`raw`](Self::raw).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifierSet {
    raw: u32,
    names: Vec<&'static str>,
    unknown_bits: u32,
}

impl ModifierSet {
    /// Decode `raw` against `kind`, naming the bits whose meaning that kind
    /// fixes and recording the rest in [`unknown_bits`](Self::unknown_bits).
    pub fn decode(raw: u32, kind: NodeKind) -> ModifierSet {
        let mut d = Decoder {
            names: Vec::new(),
            remaining: raw,
        };

        // ── Global bits (identical meaning regardless of node kind) ────────
        d.take(MOD_PUBLIC, "public");
        d.take(MOD_PRIVATE, "private");
        d.take(MOD_INTERNAL, "internal");
        d.take(MOD_FILEPRIVATE, "fileprivate");
        d.take(MOD_OPEN, "open");
        d.take(MOD_STATIC, "static");
        d.take(MOD_FINAL, "final");
        d.take(MOD_OVERRIDE, "override");
        d.take(MOD_MUTATING, "mutating");
        d.take(MOD_NONMUTATING, "nonmutating");
        d.take(MOD_LAZY, "lazy");
        d.take(MOD_WEAK, "weak");
        d.take(MOD_UNOWNED, "unowned");
        d.take(MOD_ASYNC, "async");
        d.take(MOD_THROWS, "throws");
        d.take(MOD_RETHROWS, "rethrows");
        d.take(MOD_INDIRECT, "indirect");
        d.take(MOD_REQUIRED, "required");
        d.take(MOD_CONVENIENCE, "convenience");
        d.take(MOD_DYNAMIC, "dynamic");
        d.take(MOD_NONISOLATED, "nonisolated");
        d.take(MOD_ISOLATED, "isolated");
        d.take(MOD_MAIN_ACTOR, "mainActor");
        d.take(MOD_ESCAPING, "escaping");
        d.take(MOD_AUTOCLOSURE, "autoclosure");
        d.take(MOD_VARIADIC, "variadic");
        d.take(MOD_FAILABLE, "failable");
        d.take(MOD_SUPPRESSED_CONFORMANCE, "suppressed_conformance");

        // ── Context-dependent bits (reused across unrelated kinds) ─────────
        match kind {
            NodeKind::ImportDecl => {
                d.take(BIT_22, "testable");
            }
            NodeKind::ClosureCapture => {
                d.take(BIT_22, "weak");
                d.take(BIT_23, "unowned");
                d.take(BIT_24, "safe");
            }
            NodeKind::Param => {
                d.take(BIT_22, "borrowing");
                d.take(BIT_23, "consuming");
            }
            NodeKind::ClosureExpr | NodeKind::TypeFunc => {
                d.take(BIT_23, "sendable");
            }
            NodeKind::ProtocolReq => {
                d.take(BIT_22, "associatedtype");
                d.take(BIT_24, "settable");
            }
            NodeKind::InitDecl => {
                d.take(BIT_30, "implicitly_unwrapped_failable");
            }
            _ => {}
        }
        // Bit 30 is `package` access control on every kind except `init`, where
        // it was already consumed above as the IUO-failable marker.
        d.take(BIT_30, "package");

        ModifierSet {
            raw,
            names: d.names,
            unknown_bits: d.remaining,
        }
    }

    /// The original, undecoded `modifiers` bitmask — the ground truth.
    pub fn raw(&self) -> u32 {
        self.raw
    }

    /// The decoded modifier names, in canonical (bit) order.
    pub fn names(&self) -> &[&'static str] {
        &self.names
    }

    /// Bits that were set but had no known meaning for this node kind. Always
    /// preserved so a dump can surface them rather than hide them. Zero when
    /// every set bit was named.
    pub fn unknown_bits(&self) -> u32 {
        self.unknown_bits
    }

    /// `true` when no modifier bits were set at all.
    pub fn is_empty(&self) -> bool {
        self.raw == 0
    }
}

/// Bit-consuming helper: each named bit is removed from `remaining`, so whatever
/// is left over is exactly the set of bits we did not recognise.
struct Decoder {
    names: Vec<&'static str>,
    remaining: u32,
}

impl Decoder {
    fn take(&mut self, bit: u32, name: &'static str) {
        if self.remaining & bit != 0 {
            self.names.push(name);
            self.remaining &= !bit;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_global_bits_and_keeps_nothing_unknown() {
        let set = ModifierSet::decode(MOD_STATIC | MOD_THROWS, NodeKind::FuncDecl);
        assert_eq!(set.names(), &["static", "throws"]);
        assert_eq!(set.unknown_bits(), 0);
        assert_eq!(set.raw(), MOD_STATIC | MOD_THROWS);
    }

    #[test]
    fn decodes_package_access_control() {
        // `package struct S {}` — bit 30 on a type decl is `package`, and must
        // not be silently dropped (the bug this model fixes).
        let set = ModifierSet::decode(BIT_30, NodeKind::StructDecl);
        assert_eq!(set.names(), &["package"]);
        assert_eq!(set.unknown_bits(), 0);
    }

    #[test]
    fn bit_30_is_iuo_failable_on_init_not_package() {
        let set = ModifierSet::decode(BIT_30, NodeKind::InitDecl);
        assert_eq!(set.names(), &["implicitly_unwrapped_failable"]);
        assert_eq!(set.unknown_bits(), 0);
    }

    #[test]
    fn bit_22_is_context_dependent() {
        assert_eq!(
            ModifierSet::decode(BIT_22, NodeKind::ImportDecl).names(),
            &["testable"]
        );
        assert_eq!(
            ModifierSet::decode(BIT_22, NodeKind::Param).names(),
            &["borrowing"]
        );
        assert_eq!(
            ModifierSet::decode(BIT_22, NodeKind::ClosureCapture).names(),
            &["weak"]
        );
    }

    #[test]
    fn preserves_unknown_bits_instead_of_dropping_them() {
        // Bit 22 has no decoded meaning on a plain func decl — keep it as truth.
        let set = ModifierSet::decode(MOD_PUBLIC | BIT_22, NodeKind::FuncDecl);
        assert_eq!(set.names(), &["public"]);
        assert_eq!(set.unknown_bits(), BIT_22);
        assert_eq!(set.raw(), MOD_PUBLIC | BIT_22);
    }
}
