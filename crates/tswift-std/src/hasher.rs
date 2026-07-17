//! `Hasher` — the standard-library hashing accumulator, plus the shared
//! `Hashable.hash(into:)` intrinsic for builtin value types.
//!
//! A `Hasher` is modelled as a `Struct { type_name: "Hasher" }` carrying an
//! `_state` digest field (an `Int`). `combine(_:)` folds a value's digest into
//! the state; `finalize()` returns the accumulated state as an `Int`.
//!
//! **Fidelity tier (honest):** Swift seeds each program run with a random hash
//! seed so hash values differ across runs; this runtime uses a fixed FNV-1a
//! seed so digests are deterministic (matching how the per-type `hashValue`
//! intrinsics already behave). Equal values still hash equally, which is the
//! observable contract user code relies on.
//!
//! `x.hash(into: &hasher)` is intercepted by the interpreter dispatcher (the
//! `into:` Hasher is `inout`); the `hash` intrinsic registered here folds the
//! receiver's digest into the passed Hasher and returns the updated Hasher,
//! which the dispatcher writes back through the `inout` place.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, EvalError, Interpreter, LabeledMethodEntry, MethodEntry, Outcome,
    StdContext, StdError, SwiftValue,
};

/// FNV-1a offset basis / prime, matching the per-type `hashValue` intrinsics.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Every builtin receiver that carries a `hashValue` and therefore a
/// `Hashable.hash(into:)`. Registering `hash` on each makes the member appear
/// in the coverage registry and lets the dispatcher fold it into a `Hasher`.
const HASHABLE_RECEIVERS: &[BuiltinReceiver] = &[
    BuiltinReceiver::Int,
    BuiltinReceiver::Double,
    BuiltinReceiver::Bool,
    BuiltinReceiver::String,
    BuiltinReceiver::Substring,
    BuiltinReceiver::Array,
    BuiltinReceiver::ArraySlice,
    BuiltinReceiver::ContiguousArray,
    BuiltinReceiver::Dictionary,
    BuiltinReceiver::Set,
    BuiltinReceiver::Range,
    BuiltinReceiver::ClosedRange,
    BuiltinReceiver::Optional,
    BuiltinReceiver::ReversedCollection,
];

/// Register the `Hasher` type and the shared `hash(into:)` intrinsic.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_free_fn("Hasher", hasher_init);

    let h = BuiltinReceiver::Hasher;
    interp.register_intrinsic(
        h,
        "combine",
        MethodEntry {
            mutating: true,
            func: combine,
        },
    );
    interp.register_intrinsic(
        h,
        "finalize",
        MethodEntry {
            mutating: false,
            func: finalize,
        },
    );

    // `hash(into:)` on every builtin Hashable receiver. The dispatcher
    // intercepts the call to write the returned Hasher back through the
    // `inout` place; this entry supplies the folding logic.
    for &recv in HASHABLE_RECEIVERS {
        interp.register_labeled_intrinsic(
            recv,
            "hash",
            LabeledMethodEntry {
                mutating: false,
                func: hash_into,
            },
        );
    }
}

/// Construct a fresh `Hasher` with the FNV-1a offset basis as its seed.
fn make_hasher(state: u64) -> SwiftValue {
    SwiftValue::Struct(Rc::new(tswift_core::StructObj {
        type_name: "Hasher".into(),
        fields: vec![("_state".into(), SwiftValue::int(i128::from(state as i64)))],
    }))
}

/// Read a `Hasher`'s accumulated state as a `u64`.
fn hasher_state(v: &SwiftValue) -> Option<u64> {
    match v {
        SwiftValue::Struct(obj) if obj.type_name == "Hasher" => {
            obj.get("_state").map(|f| match f {
                SwiftValue::Int(i) => i.raw as u64,
                _ => 0,
            })
        }
        _ => None,
    }
}

/// `Hasher()` — a new accumulator seeded with the FNV-1a offset basis.
fn hasher_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> Result<SwiftValue, StdError> {
    Ok(make_hasher(FNV_OFFSET))
}

/// `Hasher.combine(_:)` — fold a value's digest into the accumulator.
fn combine(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let state = hasher_state(&recv).ok_or_else(|| {
        StdError::Error(EvalError::Type("combine: receiver is not a Hasher".into()))
    })?;
    let value = args.first().ok_or_else(|| {
        StdError::Error(EvalError::Type("combine(_:) expects one argument".into()))
    })?;
    let next = fold(state, digest_of(value));
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: make_hasher(next),
    })
}

/// `Hasher.finalize()` — the accumulated state as an `Int`.
fn finalize(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let state = hasher_state(&recv).ok_or_else(|| {
        StdError::Error(EvalError::Type("finalize: receiver is not a Hasher".into()))
    })?;
    Ok(Outcome {
        result: SwiftValue::int(i128::from(state as i64)),
        receiver: recv,
    })
}

/// `x.hash(into: &hasher)` — fold the receiver's digest into `hasher` and
/// return the updated Hasher for the dispatcher to write back.
fn hash_into(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let hasher = args
        .iter()
        .find(|a| a.label.as_deref() == Some("into"))
        .map(|a| &a.value)
        .ok_or_else(|| {
            StdError::Error(EvalError::Type(
                "hash(into:) expects an `into` argument".into(),
            ))
        })?;
    let state = hasher_state(hasher).ok_or_else(|| {
        StdError::Error(EvalError::Type("hash(into:) `into` is not a Hasher".into()))
    })?;
    let next = fold(state, digest_of(&recv));
    Ok(Some(Outcome {
        result: make_hasher(next),
        receiver: recv,
    }))
}

/// Fold one digest into the running state (FNV-1a over the digest's 8 bytes).
fn fold(mut state: u64, digest: u64) -> u64 {
    for b in digest.to_le_bytes() {
        state ^= u64::from(b);
        state = state.wrapping_mul(FNV_PRIME);
    }
    state
}

/// A deterministic per-value digest, structurally consistent with the per-type
/// `hashValue` intrinsics (equal values digest equally).
fn digest_of(value: &SwiftValue) -> u64 {
    match value {
        SwiftValue::Void => 0,
        SwiftValue::Nil => 0x9e37_79b9_7f4a_7c15,
        SwiftValue::Bool(b) => u64::from(*b),
        SwiftValue::Int(i) => i.raw as u64,
        SwiftValue::Double(d) => {
            // Canonicalize so +0.0/-0.0 agree and every NaN agrees.
            let canon = if *d == 0.0 {
                0.0
            } else if d.is_nan() {
                f64::NAN
            } else {
                *d
            };
            canon.to_bits()
        }
        SwiftValue::Str(s) => hash_bytes(s.as_bytes()),
        SwiftValue::Substring { base, start, end } => {
            hash_bytes(base.get(*start..*end).unwrap_or("").as_bytes())
        }
        SwiftValue::Range { lo, hi, inclusive } => {
            let mut s = FNV_OFFSET;
            s = fold(s, *lo as u64);
            s = fold(s, *hi as u64);
            fold(s, u64::from(*inclusive))
        }
        // Ordered sequences fold each element's digest in order.
        SwiftValue::Array(items) => ordered_digest(items),
        SwiftValue::ArraySlice { base, start, end } => {
            ordered_digest(base.get(*start..*end).unwrap_or(&[]))
        }
        // Sets and dictionaries are unordered: combine element digests with a
        // commutative (`wrapping_add`) reducer so order does not matter.
        SwiftValue::Set(items) => items
            .iter()
            .fold(0u64, |acc, e| acc.wrapping_add(digest_of(e))),
        SwiftValue::Dict(pairs) => pairs.iter().fold(0u64, |acc, (k, v)| {
            acc.wrapping_add(fold(digest_of(k), digest_of(v)))
        }),
        // Everything else digests by its type name — a stable fallback.
        other => hash_bytes(other.type_name().as_bytes()),
    }
}

/// FNV-1a over a byte slice.
fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Fold an ordered element slice into a single digest.
fn ordered_digest(items: &[SwiftValue]) -> u64 {
    items
        .iter()
        .fold(FNV_OFFSET, |acc, e| fold(acc, digest_of(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_is_order_sensitive() {
        let a = fold(fold(FNV_OFFSET, 1), 2);
        let b = fold(fold(FNV_OFFSET, 2), 1);
        assert_ne!(a, b);
    }

    #[test]
    fn equal_values_digest_equally() {
        assert_eq!(
            digest_of(&SwiftValue::Str("hi".into())),
            digest_of(&SwiftValue::Str("hi".into()))
        );
        assert_eq!(
            digest_of(&SwiftValue::int(42)),
            digest_of(&SwiftValue::int(42))
        );
        assert_ne!(
            digest_of(&SwiftValue::int(42)),
            digest_of(&SwiftValue::int(43))
        );
    }

    #[test]
    fn set_digest_is_order_independent() {
        let s1 = SwiftValue::Set(Rc::new(vec![SwiftValue::int(1), SwiftValue::int(2)]));
        let s2 = SwiftValue::Set(Rc::new(vec![SwiftValue::int(2), SwiftValue::int(1)]));
        assert_eq!(digest_of(&s1), digest_of(&s2));
    }
}
