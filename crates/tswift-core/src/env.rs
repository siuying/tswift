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

    /// Replace the scope chain with one rooted only at the global (bottom)
    /// scope plus a fresh innermost scope, returning the previous chain. Used
    /// to run a method or computed-property body isolated from the caller's
    /// locals: it sees globals (sibling functions, types) and its own
    /// parameters/`self`, but never an enclosing function's variables.
    pub fn enter_isolated(&mut self) -> Vec<Scope> {
        let global = self.scopes.first().cloned().unwrap_or_default();
        std::mem::replace(&mut self.scopes, vec![global, Scope::default()])
    }

    /// Restore a scope chain previously taken by [`Env::enter_isolated`].
    pub fn restore(&mut self, saved: Vec<Scope>) {
        self.scopes = saved;
    }

    /// Leave the innermost scope, discarding its bindings.
    pub fn pop(&mut self) {
        self.scopes.pop();
    }

    /// Leave the innermost scope and, if it was not captured elsewhere, return
    /// the values it held (so the caller can run `deinit` for released objects).
    pub fn pop_owned(&mut self) -> Vec<SwiftValue> {
        match self.scopes.pop() {
            Some(scope) => match Rc::try_unwrap(scope) {
                Ok(cell) => cell.into_inner().into_values().map(|b| b.value).collect(),
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        }
    }

    /// Take and replace the global scope's owned values, for end-of-program
    /// `deinit`. Leaves a fresh empty global scope behind.
    pub fn drain_global(&mut self) -> Vec<SwiftValue> {
        if let Some(first) = self.scopes.first_mut() {
            let taken = std::mem::take(&mut *first.borrow_mut());
            taken.into_values().map(|b| b.value).collect()
        } else {
            Vec::new()
        }
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

    /// Look up a binding in the local scopes only — every scope except the
    /// global (bottom) one. Used so that, inside a method, parameters and
    /// block-locals are found first while the enclosing type's members get a
    /// chance to shadow module-level globals.
    pub fn get_local(&self, name: &str) -> Option<SwiftValue> {
        for scope in self.scopes.iter().skip(1).rev() {
            if let Some(b) = scope.borrow().get(name) {
                return Some(b.value.clone());
            }
        }
        None
    }

    /// Look up a binding in the global (bottom) scope only.
    pub fn get_global(&self, name: &str) -> Option<SwiftValue> {
        self.scopes
            .first()
            .and_then(|s| s.borrow().get(name).map(|b| b.value.clone()))
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
