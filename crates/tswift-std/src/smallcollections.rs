//! `CollectionOfOne` and `EmptyCollection` method intrinsics.
//!
//! ## CollectionOfOne
//!
//! Holds exactly one element; represented as:
//! ```text
//! SwiftValue::Struct { type_name: "CollectionOfOne",
//!                      fields: [("_element", <value>)] }
//! ```
//! Constructor: `CollectionOfOne(x)` (free function in `free.rs`).
//!
//! ## EmptyCollection
//!
//! Typed empty sequence; represented as:
//! ```text
//! SwiftValue::Struct { type_name: "EmptyCollection", fields: [] }
//! ```
//! Constructor: `EmptyCollection<T>()` (free function in `free.rs`).

use std::rc::Rc;

use tswift_core::{
    BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError, StdResult,
    StructObj, SwiftValue,
};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register all `CollectionOfOne` and `EmptyCollection` intrinsics.
pub fn install(interp: &mut Interpreter<'_>) {
    install_collection_of_one(interp);
    install_empty_collection(interp);
}

fn install_collection_of_one(interp: &mut Interpreter<'_>) {
    let nm = |interp: &mut Interpreter<'_>, name: &str, func: tswift_core::IntrinsicFn| {
        interp.register_intrinsic(
            BuiltinReceiver::CollectionOfOne,
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    };

    interp.register_property(BuiltinReceiver::CollectionOfOne, "count", coo_count);
    interp.register_property(BuiltinReceiver::CollectionOfOne, "isEmpty", coo_is_empty);
    interp.register_property(BuiltinReceiver::CollectionOfOne, "first", coo_first);
    interp.register_property(BuiltinReceiver::CollectionOfOne, "last", coo_first); // same as first
    interp.register_property(
        BuiltinReceiver::CollectionOfOne,
        "startIndex",
        coo_start_index,
    );
    interp.register_property(BuiltinReceiver::CollectionOfOne, "endIndex", coo_end_index);
    interp.register_property(
        BuiltinReceiver::CollectionOfOne,
        "debugDescription",
        coo_description,
    );
    interp.register_property(BuiltinReceiver::CollectionOfOne, "hashValue", coo_hash);

    nm(interp, "makeIterator", coo_make_iterator);
    nm(interp, "next", coo_make_iterator); // single-element iterator
    nm(interp, "index", coo_index);
}

fn install_empty_collection(interp: &mut Interpreter<'_>) {
    let nm = |interp: &mut Interpreter<'_>, name: &str, func: tswift_core::IntrinsicFn| {
        interp.register_intrinsic(
            BuiltinReceiver::EmptyCollection,
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    };

    interp.register_property(BuiltinReceiver::EmptyCollection, "count", ec_count);
    interp.register_property(BuiltinReceiver::EmptyCollection, "isEmpty", ec_is_empty);
    interp.register_property(BuiltinReceiver::EmptyCollection, "first", ec_nil);
    interp.register_property(BuiltinReceiver::EmptyCollection, "last", ec_nil);
    interp.register_property(BuiltinReceiver::EmptyCollection, "startIndex", ec_zero);
    interp.register_property(BuiltinReceiver::EmptyCollection, "endIndex", ec_zero);
    interp.register_property(BuiltinReceiver::EmptyCollection, "hashValue", ec_hash);

    nm(interp, "makeIterator", ec_make_iterator);
    nm(interp, "next", ec_make_iterator);
    nm(interp, "distance", ec_distance);
    nm(interp, "index", ec_index);
}

// ---------------------------------------------------------------------------
// CollectionOfOne constructors / helpers
// ---------------------------------------------------------------------------

/// Build a `CollectionOfOne` wrapping `element`.
#[allow(dead_code)]
pub fn make_collection_of_one(element: SwiftValue) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "CollectionOfOne".into(),
        fields: vec![("_element".into(), element)],
    }))
}

/// Build an `EmptyCollection`.
#[allow(dead_code)]
pub fn make_empty_collection() -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "EmptyCollection".into(),
        fields: vec![],
    }))
}

// ---------------------------------------------------------------------------
// CollectionOfOne intrinsics
// ---------------------------------------------------------------------------

fn coo_element(v: &SwiftValue) -> Result<SwiftValue, StdError> {
    match v {
        SwiftValue::Struct(obj) if obj.type_name == "CollectionOfOne" => {
            obj.get("_element").cloned().ok_or_else(|| {
                StdError::Error(EvalError::Type("CollectionOfOne missing _element".into()))
            })
        }
        other => Err(StdError::Error(EvalError::Type(format!(
            "expected CollectionOfOne, got {}",
            other.type_name()
        )))),
    }
}

fn coo_count(v: SwiftValue) -> StdResult {
    coo_element(&v)?;
    Ok(SwiftValue::int(1))
}

fn coo_is_empty(v: SwiftValue) -> StdResult {
    coo_element(&v)?;
    Ok(SwiftValue::Bool(false))
}

fn coo_first(v: SwiftValue) -> StdResult {
    coo_element(&v)
}

fn coo_start_index(v: SwiftValue) -> StdResult {
    coo_element(&v)?;
    Ok(SwiftValue::int(0))
}

fn coo_end_index(v: SwiftValue) -> StdResult {
    coo_element(&v)?;
    Ok(SwiftValue::int(1))
}

fn coo_description(v: SwiftValue) -> StdResult {
    let e = coo_element(&v)?;
    Ok(SwiftValue::Str(format!("CollectionOfOne({e})")))
}

fn coo_hash(v: SwiftValue) -> StdResult {
    let e = coo_element(&v)?;
    Ok(SwiftValue::int(
        crate::array::slice_stable_hash(&e) as i64 as i128
    ))
}

fn coo_make_iterator(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    Ok(Outcome {
        result: recv.clone(),
        receiver: recv,
    })
}

fn coo_index(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    coo_element(&recv)?;
    let base = match args.first() {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => {
            return Err(StdError::Error(EvalError::Type(
                "index(_:offsetBy:) expects integer base index".into(),
            )))
        }
    };
    let offset = match args.get(1) {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => 0,
    };
    let result = base + offset;
    if !(0..=1).contains(&result) {
        return Err(StdError::Error(EvalError::Trap(
            "CollectionOfOne index out of bounds".into(),
        )));
    }
    Ok(Outcome {
        result: SwiftValue::int(result),
        receiver: recv,
    })
}

// ---------------------------------------------------------------------------
// EmptyCollection intrinsics
// ---------------------------------------------------------------------------

fn ensure_empty(v: &SwiftValue) -> Result<(), StdError> {
    match v {
        SwiftValue::Struct(obj) if obj.type_name == "EmptyCollection" => Ok(()),
        other => Err(StdError::Error(EvalError::Type(format!(
            "expected EmptyCollection, got {}",
            other.type_name()
        )))),
    }
}

fn ec_count(v: SwiftValue) -> StdResult {
    ensure_empty(&v)?;
    Ok(SwiftValue::int(0))
}

fn ec_is_empty(v: SwiftValue) -> StdResult {
    ensure_empty(&v)?;
    Ok(SwiftValue::Bool(true))
}

fn ec_nil(v: SwiftValue) -> StdResult {
    ensure_empty(&v)?;
    Ok(SwiftValue::Nil)
}

fn ec_zero(v: SwiftValue) -> StdResult {
    ensure_empty(&v)?;
    Ok(SwiftValue::int(0))
}

fn ec_hash(v: SwiftValue) -> StdResult {
    ensure_empty(&v)?;
    Ok(SwiftValue::int(0))
}

fn ec_make_iterator(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    ensure_empty(&recv)?;
    Ok(Outcome {
        result: recv.clone(),
        receiver: recv,
    })
}

fn ec_distance(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    ensure_empty(&recv)?;
    let from = match args.first() {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => {
            return Err(StdError::Error(EvalError::Type(
                "distance(from:to:) expects integer indices".into(),
            )))
        }
    };
    let to = match args.get(1) {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => {
            return Err(StdError::Error(EvalError::Type(
                "distance(from:to:) expects integer indices".into(),
            )))
        }
    };
    // EmptyCollection only has index 0; any other value is a logic error.
    if from != 0 || to != 0 {
        return Err(StdError::Error(EvalError::Trap(
            "EmptyCollection index out of bounds".into(),
        )));
    }
    Ok(Outcome {
        result: SwiftValue::int(0),
        receiver: recv,
    })
}

fn ec_index(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    ensure_empty(&recv)?;
    let base = match args.first() {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => {
            return Err(StdError::Error(EvalError::Type(
                "index(_:offsetBy:) expects integer base index".into(),
            )))
        }
    };
    let offset = match args.get(1) {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => 0,
    };
    let result = base + offset;
    if result != 0 {
        return Err(StdError::Error(EvalError::Trap(
            "EmptyCollection index out of bounds".into(),
        )));
    }
    Ok(Outcome {
        result: SwiftValue::int(0),
        receiver: recv,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct Ctx;
    impl tswift_core::StdContext for Ctx {
        fn call_closure(&mut self, _: usize, _: Vec<SwiftValue>) -> StdResult {
            Ok(SwiftValue::Nil)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!()
        }
    }

    #[test]
    fn collection_of_one_basics() {
        let v = make_collection_of_one(SwiftValue::int(42));
        assert_eq!(coo_count(v.clone()).unwrap(), SwiftValue::int(1));
        assert_eq!(coo_is_empty(v.clone()).unwrap(), SwiftValue::Bool(false));
        assert_eq!(coo_first(v.clone()).unwrap(), SwiftValue::int(42));
        assert_eq!(coo_start_index(v.clone()).unwrap(), SwiftValue::int(0));
        assert_eq!(coo_end_index(v.clone()).unwrap(), SwiftValue::int(1));
    }

    #[test]
    fn empty_collection_basics() {
        let v = make_empty_collection();
        assert_eq!(ec_count(v.clone()).unwrap(), SwiftValue::int(0));
        assert_eq!(ec_is_empty(v.clone()).unwrap(), SwiftValue::Bool(true));
        assert_eq!(ec_nil(v.clone()).unwrap(), SwiftValue::Nil);
        assert_eq!(ec_zero(v.clone()).unwrap(), SwiftValue::int(0));
    }

    #[test]
    fn empty_collection_distance_roundtrip() {
        let v = make_empty_collection();
        let r = ec_distance(&mut Ctx, v, vec![SwiftValue::int(0), SwiftValue::int(0)])
            .unwrap()
            .result;
        assert_eq!(r, SwiftValue::int(0));
    }
}
