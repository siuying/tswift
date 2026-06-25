//! Lexical environment: a stack of name → binding scopes.
//!
//! A [`Binding`] records both the value and whether the name was declared with
//! `var` (mutable) or `let` (immutable). Assignment walks outward from the
//! innermost scope, enforcing immutability.

use std::collections::HashMap;

use crate::value::SwiftValue;

/// One variable binding: its current value and whether it may be reassigned.
#[derive(Debug, Clone)]
pub struct Binding {
    pub value: SwiftValue,
    pub mutable: bool,
}

/// A stack of scopes. The last entry is the innermost (current) scope.
#[derive(Debug, Default)]
pub struct Env {
    scopes: Vec<HashMap<String, Binding>>,
}

/// Why a binding mutation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindError {
    /// No binding with this name is in scope.
    Unbound(String),
    /// The binding exists but was declared `let`.
    Immutable(String),
}

impl Env {
    /// A new environment with a single global scope.
    pub fn new() -> Env {
        Env {
            scopes: vec![HashMap::new()],
        }
    }

    /// Enter a fresh nested scope.
    pub fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Leave the innermost scope, discarding its bindings.
    pub fn pop(&mut self) {
        self.scopes.pop();
    }

    /// Declare a new binding in the innermost scope (shadowing any outer one).
    pub fn declare(&mut self, name: &str, value: SwiftValue, mutable: bool) {
        self.scopes
            .last_mut()
            .expect("at least one scope")
            .insert(name.to_string(), Binding { value, mutable });
    }

    /// Look up a binding's value, searching innermost-outward.
    pub fn get(&self, name: &str) -> Option<&SwiftValue> {
        for scope in self.scopes.iter().rev() {
            if let Some(b) = scope.get(name) {
                return Some(&b.value);
            }
        }
        None
    }

    /// Assign to an existing mutable binding, searching innermost-outward.
    pub fn assign(&mut self, name: &str, value: SwiftValue) -> Result<(), BindError> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(b) = scope.get_mut(name) {
                if !b.mutable {
                    return Err(BindError::Immutable(name.to_string()));
                }
                b.value = value;
                return Ok(());
            }
        }
        Err(BindError::Unbound(name.to_string()))
    }
}
