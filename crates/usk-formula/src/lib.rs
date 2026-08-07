//! usk-formula — the formula engine (docs/12, BOOTSTRAP row 6).
//!
//! Pipeline: `text → lexer → Pratt parser → lossless CST → AST → evaluator`.
//! The binder stage docs/12 names sits between AST and evaluation and resolves
//! A1 text to permanent identities; in v0.1 that resolution happens at read
//! time through the [`eval::Grid`] port, because the identity-interval
//! machinery it needs lands with the dependency graph in Row 7.
//!
//! `no_std + alloc` like every kernel crate (DP-A3): no clock, no filesystem,
//! no libm. Volatiles are injected through [`eval::Context`], and the few
//! transcendental functions needed for `^` are implemented here rather than
//! taken from a platform math library whose last-bit results vary — DP-A2
//! requires the same answer on every target, forever.

#![no_std]
extern crate alloc;

pub mod eval;
pub mod functions;
pub mod lexer;
pub mod parse;

use usk_types::coerce::Profile;
use usk_types::Value;

/// Parses and evaluates a formula in one step.
///
/// Convenience over the staged API, which callers reach for when they need the
/// CST (refactoring, error carets) or want to evaluate one parse many times.
pub fn evaluate<G: eval::Grid>(source: &str, grid: &G, profile: Profile) -> Value {
    let parsed = parse::parse(source);
    let ctx = eval::Context::new(grid, profile);
    eval::eval(&parsed.ast, &ctx)
}
