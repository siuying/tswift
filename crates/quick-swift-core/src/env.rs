//! Lexical environment: a stack of name → binding scopes.
//!
//! Scopes are `Rc<RefCell<…>>` so a function value can *capture* its enclosing
//! scope chain by reference. Two consequences fall out for free: recursion and
//! mutual recursion work (a function's captured global scope sees sibling
//! functions declared later), and nested functions observe live updates to the
//! variables they close over.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::value::SwiftValue;

/// One variable binding: its current value and whether it may be reassigned.
#[derive(Debug, Clone)]
pub struct Binding {
    pub value: SwiftValue,
    pub mutable: bool,
}

/// A single lexical scope, shareable between an environment and the closures
/// that capture it.
pub type Scope = Rc<RefCell<HashMap<String, Binding>>>;

/// A stack of scopes. The last entry is the innermost (current) scope.
#[derive(Debug, Clone, Default)]
pub struct Env {
    scopes: Vec<Scope>,
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
            scopes: vec![Scope::default()],
        }
    }

    /// Build an environment over an existing captured scope chain, then open a
    /// fresh innermost scope for new bindings.
    pub fn with_captured(mut scopes: Vec<Scope>) -> Env {
        scopes.push(Scope::default());
        Env { scopes }
    }

    /// A clone of the current scope chain, suitable for a closure to capture.
    /// The `Rc` clones share the underlying scopes by reference.
    pub fn capture(&self) -> Vec<Scope> {
        self.scopes.clone()
    }

    /// Enter a fresh nested scope.
    pub fn push(&mut self) {
        self.scopes.push(Scope::default());
    }

    /// Leave the innermost scope, discarding its bindings.
    pub fn pop(&mut self) {
        self.scopes.pop();
    }

    /// Declare a new binding in the innermost scope (shadowing any outer one).
    pub fn declare(&mut self, name: &str, value: SwiftValue, mutable: bool) {
        self.scopes
            .last()
            .expect("at least one scope")
            .borrow_mut()
            .insert(name.to_string(), Binding { value, mutable });
    }

    /// Look up a binding's value, searching innermost-outward.
    pub fn get(&self, name: &str) -> Option<SwiftValue> {
        for scope in self.scopes.iter().rev() {
            if let Some(b) = scope.borrow().get(name) {
                return Some(b.value.clone());
            }
        }
        None
    }

    /// Assign to an existing mutable binding, searching innermost-outward.
    pub fn assign(&mut self, name: &str, value: SwiftValue) -> Result<(), BindError> {
        for scope in self.scopes.iter().rev() {
            let mut s = scope.borrow_mut();
            if let Some(b) = s.get_mut(name) {
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
