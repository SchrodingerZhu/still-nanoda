//! Placeholder:
//! ```ignore
//! Doc comment example
//! ```
#![allow(clippy::too_many_arguments)]
#![deny(clippy::cast_possible_truncation)]

pub mod conv;
pub mod debug_printer;
pub mod decode;
pub mod env;
pub mod eval;
pub mod extck;
pub mod expr;
pub mod importer;
pub mod inductive;
pub mod kernel_err;
pub mod lean_sys;
pub mod level;
pub mod name;
pub mod pretty_printer;
pub mod term;
pub mod quot;
pub mod tc;
#[cfg(test)]
mod tests;
pub mod union_find;
pub mod unique_hasher;
pub mod util;
pub mod value;

pub(crate) const STACK_SIZE: usize = 2 * 1024 * 1024 * 1024;
