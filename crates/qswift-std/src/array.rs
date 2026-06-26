//! `Array` method intrinsics.

use std::rc::Rc;

use qswift_core::{
    BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError, SwiftValue,
};

/// Register the `Array` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_intrinsic(
        BuiltinReceiver::Array,
        "append",
        MethodEntry {
            mutating: true,
            func: append,
        },
    );
}

/// `Array.append(_:)` — push one element onto the end, in place.
///
/// Mutating: the receiver arrives by value and the updated array is returned for
/// the dispatcher to write back. Copy-on-write is preserved by `Rc::make_mut`,
/// so a shared copy is cloned only when actually mutated.
fn append(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let SwiftValue::Array(mut items) = recv else {
        return Err(StdError::Error(EvalError::Type(
            "append called on a non-array receiver".into(),
        )));
    };
    let element = args.into_iter().next().ok_or_else(|| {
        StdError::Error(EvalError::Type("append expects one argument".into()))
    })?;
    Rc::make_mut(&mut items).push(element);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Array(items),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `StdContext` that satisfies the trait without an interpreter, so
    /// intrinsics can be unit-tested in isolation against the seam.
    struct MockCtx {
        sink: Vec<u8>,
    }

    impl StdContext for MockCtx {
        fn call_closure(
            &mut self,
            _id: usize,
            _args: Vec<SwiftValue>,
        ) -> qswift_core::StdResult {
            Ok(SwiftValue::Void)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            &mut self.sink
        }
    }

    #[test]
    fn append_pushes_and_returns_updated_receiver() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let recv = SwiftValue::Array(Rc::new(vec![SwiftValue::int(1), SwiftValue::int(2)]));
        let out = append(&mut ctx, recv, vec![SwiftValue::int(3)]).unwrap();

        assert_eq!(out.result, SwiftValue::Void);
        match out.receiver {
            SwiftValue::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[2], SwiftValue::int(3));
            }
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn append_preserves_copy_on_write() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let shared = Rc::new(vec![SwiftValue::int(1)]);
        let recv = SwiftValue::Array(Rc::clone(&shared));

        let out = append(&mut ctx, recv, vec![SwiftValue::int(2)]).unwrap();

        // The original storage is untouched; the mutation cloned it (CoW).
        assert_eq!(shared.as_slice(), &[SwiftValue::int(1)]);
        match out.receiver {
            SwiftValue::Array(items) => assert_eq!(items.len(), 2),
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn append_rejects_non_array_receiver() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let err = append(&mut ctx, SwiftValue::int(1), vec![SwiftValue::int(2)]).unwrap_err();
        assert!(matches!(err, StdError::Error(_)));
    }
}
