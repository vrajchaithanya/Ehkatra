//! replay-check — the DP-A2 determinism gate.
//! Builds a fixed, deterministic op corpus (LCG-driven, no ambient entropy),
//! replays it, and prints the canonical state hash. CI runs this binary on
//! native AND wasm32; the printed hashes MUST be identical.

use usk_oplog::{
    Alignment, Anchor, AxisSpan, FontFacet, Op, OpLog, OpaqueOp, Payload, RangeBinding, StyleFacet,
    StyleTarget, UnknownFacet,
};
use usk_state::State;
use usk_types::{ActorId, ColId, Decimal, OpId, RowId, Value};

fn main() {
    let mut log = OpLog::new();
    let mut rows: Vec<OpId> = Vec::new();
    let mut cols: Vec<OpId> = Vec::new();
    let mut deleted_rows: Vec<OpId> = Vec::new();
    let mut deleted_cols: Vec<OpId> = Vec::new();
    let mut seed: u64 = 0xDEADBEEFCAFEBABE;
    let mut rand = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed >> 33
    };

    // 3 actors, 5000 ops covering **every** payload variant. docs/29 is
    // explicit: a new op type must join this generator, or the determinism
    // gate silently stops covering it. Row 9's SetFormula/UndeleteRow/
    // UndeleteCol — and ClearCell, never covered before — are exercised here.
    for i in 0..5000u64 {
        // Per-op counter is 1-based and advances in lockstep with the loop, so
        // the corpus is a pure function of the seed (DP-A2).
        let counter = i + 1;
        let actor = ActorId(1 + (rand() % 3) as u128);
        let id = OpId { actor, counter };
        let lamport = i + 1;
        let payload = match rand() % 16 {
            0 | 1 => {
                let anchor = if rows.is_empty() {
                    Anchor::Start
                } else {
                    Anchor::After(rows[(rand() as usize) % rows.len()])
                };
                rows.push(id);
                Payload::InsertRow { anchor }
            }
            2 => {
                let anchor = if cols.is_empty() {
                    Anchor::Start
                } else {
                    Anchor::After(cols[(rand() as usize) % cols.len()])
                };
                cols.push(id);
                Payload::InsertCol { anchor }
            }
            3 if rows.len() > 4 => {
                let r = rows[(rand() as usize) % rows.len()];
                deleted_rows.push(r);
                Payload::DeleteRow { row: RowId(r) }
            }
            4 if cols.len() > 4 => {
                let c = cols[(rand() as usize) % cols.len()];
                deleted_cols.push(c);
                Payload::DeleteCol { col: ColId(c) }
            }
            // Undeletes: the Row 9 ops. Resurrecting a row that was never
            // deleted is still a valid op, so the corpus exercises both.
            5 if !deleted_rows.is_empty() => {
                let r = deleted_rows[(rand() as usize) % deleted_rows.len()];
                Payload::UndeleteRow { row: RowId(r) }
            }
            6 if !deleted_cols.is_empty() => {
                let c = deleted_cols[(rand() as usize) % deleted_cols.len()];
                Payload::UndeleteCol { col: ColId(c) }
            }
            // DP-A5 forward preservation, in the gate (TD-25). An op tag this
            // build does not know must hash and retransmit identically on every
            // target, and docs/29 is explicit that a payload variant absent
            // from this generator is a variant the determinism gate does not
            // cover — session 9 found four such variants the hard way.
            10 => match OpaqueOp::new(0x2B + (rand() % 16) as u8, opaque_body(rand())) {
                Some(o) => Payload::Opaque(o),
                // Unreachable: 0x2B..=0x3A are outside model version 1's
                // taxonomy by construction. A fallback rather than an unwrap,
                // because DP-C1 has no exception for "obviously fine".
                //
                // The base moved from 0x19 in session 30: `SetStyle` and
                // `ClearStyle` took 0x19/0x1A, so the old range would have
                // handed `OpaqueOp::new` a tag this build now knows and fallen
                // through to the fallback for two values in sixteen. The
                // generator must produce tags that are genuinely unknown, or it
                // stops testing preservation and starts testing the fallback.
                None => Payload::UndeleteRow { row: RowId(id) },
            },
            // Styles (ADR-041). docs/29: *"New op type → extend the canonical
            // encoding … and add to replay-check's generator so the corpus
            // exercises it."* Both new tags and an **unknown facet** are
            // generated, because the facet vocabulary is extensible in exactly
            // the way the op taxonomy is and needs the same evidence.
            11 if !rows.is_empty() && !cols.is_empty() => Payload::SetStyle {
                target: style_target(&rows, &cols, &mut rand),
                facet: style_facet(&mut rand),
            },
            12 if !rows.is_empty() && !cols.is_empty() => Payload::ClearStyle {
                target: style_target(&rows, &cols, &mut rand),
                // Slot 0x05 is outside the modelled set: clearing a facet this
                // build does not know is a legal op and must hash identically
                // on every target.
                facet_slot: 1 + (rand() % 5) as u8,
            },
            7 if !rows.is_empty() && !cols.is_empty() => {
                let r = rows[(rand() as usize) % rows.len()];
                let c = cols[(rand() as usize) % cols.len()];
                Payload::ClearCell {
                    row: RowId(r),
                    col: ColId(c),
                }
            }
            // Formulas carry identity bindings, so this arm also exercises the
            // variable-length binding vector in the canonical encoding.
            8 | 9 if rows.len() > 2 && cols.len() > 2 => {
                let r = rows[(rand() as usize) % rows.len()];
                let c = cols[(rand() as usize) % cols.len()];
                let n_bindings = 1 + (rand() % 3) as usize;
                let mut bindings = Vec::with_capacity(n_bindings);
                for _ in 0..n_bindings {
                    bindings.push(RangeBinding {
                        row_start: rows[(rand() as usize) % rows.len()],
                        row_end: rows[(rand() as usize) % rows.len()],
                        col_start: cols[(rand() as usize) % cols.len()],
                        col_end: cols[(rand() as usize) % cols.len()],
                        anchors: (rand() % 4) as u8,
                    });
                }
                Payload::SetFormula {
                    row: RowId(r),
                    col: ColId(c),
                    source: alloc_formula(rand()),
                    bindings,
                }
            }
            _ => {
                if rows.is_empty() || cols.is_empty() {
                    rows.push(id);
                    Payload::InsertRow {
                        anchor: Anchor::Start,
                    }
                } else {
                    let r = rows[(rand() as usize) % rows.len()];
                    let c = cols[(rand() as usize) % cols.len()];
                    let v = match rand() % 5 {
                        0 => Value::Number((rand() % 100000) as f64 / 100.0),
                        1 => Value::Bool(rand() % 2 == 0),
                        2 => Value::Text(alloc_text(rand())),
                        // Row 5's exact-decimal domain, in the gate at last.
                        3 => Value::Decimal(Decimal::new((rand() % 1_000_000) as i128, -2)),
                        _ => Value::Blank,
                    };
                    Payload::SetCell {
                        row: RowId(r),
                        col: ColId(c),
                        value: v,
                    }
                }
            }
        };
        log.append(Op {
            id,
            lamport,
            payload,
        });
    }

    let state = State::replay(&log);
    println!("oplog:{}", log.canonical_hash());
    println!("state:{}", state.state_hash());
}

/// A style rectangle: mostly bounded by identities, sometimes a whole
/// row/column. `AxisSpan::All` has to be in the corpus because it is the
/// variant with no identity in it at all, and therefore the one whose encoding
/// nothing else would cover.
fn style_target(rows: &[OpId], cols: &[OpId], rand: &mut impl FnMut() -> u64) -> StyleTarget {
    let span = |ids: &[OpId], rand: &mut dyn FnMut() -> u64| -> AxisSpan {
        if rand().is_multiple_of(4) {
            AxisSpan::All
        } else {
            AxisSpan::Between(
                ids[(rand() as usize) % ids.len()],
                ids[(rand() as usize) % ids.len()],
            )
        }
    };
    StyleTarget {
        rows: span(rows, rand),
        cols: span(cols, rand),
    }
}

/// Every facet variant, including an **unknown** one — the facet-level
/// equivalent of the opaque op, whose bytes must survive and hash identically
/// on a build that cannot interpret them (ADR-041 decision 3).
fn style_facet(rand: &mut impl FnMut() -> u64) -> StyleFacet {
    match rand() % 5 {
        0 => StyleFacet::NumberFormat(alloc_format(rand())),
        1 => StyleFacet::Font(FontFacet {
            flags: (rand() % 16) as u8,
            half_points: (rand() % 400) as u16,
            argb: rand() as u32,
            name: alloc_text(rand()),
        }),
        2 => StyleFacet::Fill(rand() as u32),
        3 => StyleFacet::Align(Alignment {
            horizontal: (rand() % 4) as u8,
            vertical: (rand() % 3) as u8,
            wrap: rand().is_multiple_of(2),
        }),
        _ => match UnknownFacet::new(0x40 + (rand() % 8) as u8, opaque_body(rand())) {
            Some(u) => StyleFacet::Unknown(u),
            // Unreachable: 0x40..=0x47 are outside the modelled facet tags.
            None => StyleFacet::Fill(0),
        },
    }
}

fn alloc_format(n: u64) -> String {
    format!("0.{}%", "0".repeat((n % 5) as usize + 1))
}

fn alloc_text(n: u64) -> String {
    format!("cell-{n}")
}

fn alloc_formula(n: u64) -> String {
    format!("=SUM(A1:B{})+{}", n % 97 + 1, n % 13)
}

/// A deterministic body for a preserved-opaque op — bytes this build has no
/// interpretation for, which is exactly the point.
fn opaque_body(n: u64) -> Vec<u8> {
    let len = 4 + (n % 12) as usize;
    (0..len)
        .map(|k| (n.wrapping_mul(k as u64 + 1) & 0xFF) as u8)
        .collect()
}
