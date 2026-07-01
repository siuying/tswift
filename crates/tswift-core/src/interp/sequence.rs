//! Custom `Sequence`/`IteratorProtocol` iteration: detecting user conformers,
//! driving their `makeIterator()`/`next()` to run `for`-in loops, and eagerly
//! materializing them into arrays for the stdlib algorithm layer.

use tswift_frontend::Node;

use super::{
    trap, Eval, EvalError, Interpreter, LoopFlow, Place, Signal, MAX_SEQUENCE_MATERIALIZE,
};
use crate::value::SwiftValue;

impl<'w> Interpreter<'w> {
    /// Whether `value` declares `Sequence`/`IteratorProtocol` conformance and
    /// exposes the corresponding iteration method.
    pub(super) fn is_custom_sequence(&self, value: &SwiftValue) -> bool {
        self.value_type_name(value).is_some_and(|t| {
            (self.is_sequence_conformer(&t) && self.seq_type_has_method(&t, "makeIterator"))
                || (self.is_iterator_conformer(&t) && self.seq_type_has_method(&t, "next"))
        })
    }

    fn is_sequence_conformer(&self, type_name: &str) -> bool {
        self.all_protocols(type_name)
            .iter()
            .any(|p| p == "Sequence")
    }

    fn is_iterator_conformer(&self, type_name: &str) -> bool {
        self.all_protocols(type_name)
            .iter()
            .any(|p| p == "IteratorProtocol")
    }

    /// `type_has_method` extended to class declarations (walking the chain), for
    /// custom-sequence detection over struct/enum/class conformers.
    fn seq_type_has_method(&self, type_name: &str, method: &str) -> bool {
        self.type_has_method(type_name, method)
            || (self.types.is_class(type_name) && self.lookup_method(type_name, method).is_some())
    }

    /// Dispatch `next()`/`makeIterator()` on a sequence/iterator value, routing
    /// a class receiver through dynamic dispatch and a struct/enum receiver
    /// through the value-method path (writing the mutated iterator back to
    /// `place`).
    fn call_sequence_method(
        &mut self,
        receiver: SwiftValue,
        type_name: &str,
        method: &str,
        place: Option<Place>,
    ) -> Eval {
        if self.types.is_class(type_name) {
            // A class iterator mutates through its reference; no write-back.
            self.dispatch_class_method(receiver, type_name, method, Vec::new())
        } else {
            self.call_struct_method(receiver, type_name, method, Vec::new(), place)
        }
    }

    /// `for x in seq` over a custom `Sequence`/`IteratorProtocol`: obtain the
    /// iterator (the value itself if it has `next()`, else `makeIterator()`),
    /// then drive the mutating `next()` until it yields `nil`, running the loop
    /// body for each element. Supports a binding name or a `for case` pattern.
    pub(super) fn run_sync_sequence(
        &mut self,
        seq: &SwiftValue,
        var_name: Option<&str>,
        pattern: Option<Node<'static>>,
        where_clause: Option<Node<'static>>,
        body: &Node<'static>,
        label: &Option<String>,
    ) -> Eval {
        const ITER: &str = "$synciter";
        let seq_ty = self
            .value_type_name(seq)
            .ok_or_else(|| EvalError::Type("sequence has no type".into()))?;
        // A Sequence with a makeIterator() method is driven through that method
        // even if it also happens to expose a helper named next(). A type that
        // only conforms as an IteratorProtocol is its own iterator.
        let iter = if self.is_sequence_conformer(&seq_ty)
            && self.seq_type_has_method(&seq_ty, "makeIterator")
        {
            self.call_sequence_method(seq.clone(), &seq_ty, "makeIterator", None)?
        } else {
            seq.clone()
        };
        let iter_ty = self
            .value_type_name(&iter)
            .ok_or_else(|| EvalError::Type("iterator has no type".into()))?;

        self.env.push();
        self.env.declare(ITER, iter, true);
        let outcome = loop {
            let current = self.env.get(ITER).unwrap_or(SwiftValue::Nil);
            let place = Place {
                root: ITER.into(),
                path: Vec::new(),
            };
            let next = match self.call_sequence_method(current, &iter_ty, "next", Some(place)) {
                Ok(v) => v,
                Err(e) => break Err(e),
            };
            // `next()` returns `Element?`: `nil` ends the sequence.
            if matches!(next, SwiftValue::Nil) {
                break Ok(SwiftValue::Void);
            }
            self.env.push();
            // A `for case` pattern that fails to match skips the element.
            if let Some(pat) = pattern {
                match self.match_pattern(&pat, &next) {
                    Ok(Some(binds)) => {
                        for (name, value) in binds {
                            self.env.declare(&name, value, false);
                        }
                    }
                    Ok(None) => {
                        self.env.pop();
                        continue;
                    }
                    Err(s) => {
                        self.env.pop();
                        break Err(s);
                    }
                }
            } else if let Some(name) = var_name {
                self.env.declare(name, next, false);
            }
            if let Some(w) = where_clause {
                match self.eval_condition(&w) {
                    Ok(true) => {}
                    Ok(false) => {
                        self.env.pop();
                        continue;
                    }
                    Err(s) => {
                        self.env.pop();
                        break Err(s);
                    }
                }
            }
            let flow = self.run_loop_body(body, label);
            self.env.pop();
            match flow {
                Ok(LoopFlow::Continue) => {}
                Ok(LoopFlow::Break) => break Ok(SwiftValue::Void),
                Err(s) => break Err(s),
            }
        };
        self.env.pop();
        outcome
    }

    /// Eagerly drive a custom `Sequence`/`IteratorProtocol` into an array of
    /// elements for standard-library sequence algorithms.
    pub(super) fn materialize_custom_sequence(
        &mut self,
        seq: SwiftValue,
    ) -> Result<Vec<SwiftValue>, Signal> {
        const ITER: &str = "$algoiter";
        let seq_ty = self
            .value_type_name(&seq)
            .ok_or_else(|| EvalError::Type("sequence has no type".into()))?;
        let iter = if self.is_sequence_conformer(&seq_ty)
            && self.seq_type_has_method(&seq_ty, "makeIterator")
        {
            self.call_sequence_method(seq, &seq_ty, "makeIterator", None)?
        } else {
            seq
        };
        let iter_ty = self
            .value_type_name(&iter)
            .ok_or_else(|| EvalError::Type("iterator has no type".into()))?;
        self.env.push();
        self.env.declare(ITER, iter, true);
        let mut items = Vec::new();
        let result = loop {
            if items.len() >= MAX_SEQUENCE_MATERIALIZE {
                break Err(trap(format!(
                    "custom sequence algorithm exceeded {MAX_SEQUENCE_MATERIALIZE} elements"
                )));
            }
            let current = self.env.get(ITER).unwrap_or(SwiftValue::Nil);
            let place = Place {
                root: ITER.into(),
                path: Vec::new(),
            };
            let next = match self.call_sequence_method(current, &iter_ty, "next", Some(place)) {
                Ok(next) => next,
                Err(err) => break Err(err),
            };
            if matches!(next, SwiftValue::Nil) {
                break Ok(items);
            }
            items.push(next);
        };
        self.env.pop();
        result
    }
}
