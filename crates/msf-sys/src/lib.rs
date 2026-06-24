//! Raw, generated FFI bindings to the msf (Mini Swift Frontend) C library.
//!
//! This crate is the **only** place msf's C ABI is described. Everything here is
//! `unsafe` by nature: raw pointers into msf's arena-allocated AST, anonymous
//! unions, and enum-as-constants. Nothing in this crate enforces the lifetime
//! or aliasing rules msf requires — that is the job of the safe `msf` crate,
//! which is the single consumer of these bindings.
//!
//! Do not depend on `msf-sys` directly from runtime crates; depend on `msf`.
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
