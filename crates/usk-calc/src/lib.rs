//! usk-calc — the dependency graph and incremental recalculation engine
//! (docs/13, BOOTSTRAP row 7).
//!
//! # The idea that makes this scale
//! Nodes are **formula groups**, not cells. A column filled with `=A1*2` is one
//! node covering 100,000 cells, because every member shares an R1C1 pattern.
//! Edges are **range-granular**: a group records the rectangles it reads, and
//! "who reads what I just wrote" is a stab query against a spatial index rather
//! than a materialised cell-to-cell edge set. docs/13 puts the difference at
//! ~0.1 MB against ~96 MB at a million formulas — the reason the graph is a
//! graph over regions and not over cells.
//!
//! # Evaluation order
//! Edit → dirty rectangles → stab → transitive marking with **early cutoff**
//! (a group whose recomputed values are unchanged stops propagating) → topo
//! **levels** → evaluate level by level. Levels are the parallelism seam: every
//! group in a level is independent by construction. Actual threading waits for
//! the PAL `Compute` trait, since the kernel is `no_std` and rayon lives behind
//! the PAL (DP-A3, docs/10) — so the recorded benchmark is single-threaded and
//! MEASUREMENTS.md says so.
//!
//! Cycles are found by the same pass: whatever the level assignment cannot
//! place is, by definition, in a cycle, and every cell in it becomes `#CIRC!`.

#![no_std]
extern crate alloc;

pub mod graph;
pub mod sheet;

pub use graph::{Engine, RecalcStats};
pub use sheet::{Cell, CellRef, Rect, Sheet};
