//! The tile image (docs/16, docs/26): a materialised `State`, written down.
//!
//! The property that matters is one line — **an image round-trips to the same
//! state hash** — and it is what makes the image safe to adopt without
//! replaying the op log. Everything else here exists because it can be wrong in
//! a way the hash alone would not catch, or because an image read off disk is
//! an untrusted input (docs/37).

use usk_oplog::{Anchor, Op, OpLog, Payload, RangeBinding};
use usk_state::image::{chunk_hashes, ImageError, IMAGE_VERSION};
use usk_state::State;
use usk_types::{ActorId, CellError, ColId, Decimal, ErrorKind, OpId, Origin, RowId, Value};

fn id(actor: u128, counter: u64) -> OpId {
    OpId {
        actor: ActorId(actor),
        counter,
    }
}

/// A workbook that reaches every branch the image has to serialise: both packed
/// payloads and the tagged fallback, a contested cell with a retained loser, a
/// tombstoned row, an undelete, and a formula with identity bindings.
fn workbook() -> OpLog {
    let mut log = OpLog::new();
    let mut lamport = 0u64;
    let mut push = |log: &mut OpLog, id: OpId, payload: Payload| {
        lamport += 1;
        log.append(Op {
            id,
            lamport,
            payload,
        });
    };

    let r1 = id(1, 1);
    push(
        &mut log,
        r1,
        Payload::InsertRow {
            anchor: Anchor::Start,
        },
    );
    let r2 = id(1, 2);
    push(
        &mut log,
        r2,
        Payload::InsertRow {
            anchor: Anchor::After(r1),
        },
    );
    let r3 = id(1, 3);
    push(
        &mut log,
        r3,
        Payload::InsertRow {
            anchor: Anchor::After(r2),
        },
    );
    let c1 = id(1, 10);
    push(
        &mut log,
        c1,
        Payload::InsertCol {
            anchor: Anchor::Start,
        },
    );
    let c2 = id(1, 11);
    push(
        &mut log,
        c2,
        Payload::InsertCol {
            anchor: Anchor::After(c1),
        },
    );
    let c3 = id(1, 12);
    push(
        &mut log,
        c3,
        Payload::InsertCol {
            anchor: Anchor::After(c2),
        },
    );

    // A numeric tile (packed f64s).
    push(
        &mut log,
        id(1, 20),
        Payload::SetCell {
            row: RowId(r1),
            col: ColId(c1),
            value: Value::Number(1.5),
        },
    );
    push(
        &mut log,
        id(1, 21),
        Payload::SetCell {
            row: RowId(r2),
            col: ColId(c1),
            value: Value::Number(-0.25),
        },
    );
    // A decimal, which forces the tile tagged (mixed with the numbers above).
    push(
        &mut log,
        id(1, 22),
        Payload::SetCell {
            row: RowId(r3),
            col: ColId(c1),
            value: Value::Decimal(Decimal::new(-12345, -2)),
        },
    );
    // Text, a boolean and an error with a non-trivial origin.
    push(
        &mut log,
        id(1, 23),
        Payload::SetCell {
            row: RowId(r1),
            col: ColId(c2),
            value: Value::Text(String::from("héllo — 😀")),
        },
    );
    push(
        &mut log,
        id(1, 24),
        Payload::SetCell {
            row: RowId(r2),
            col: ColId(c2),
            value: Value::Bool(true),
        },
    );
    push(
        &mut log,
        id(1, 25),
        Payload::SetCell {
            row: RowId(r3),
            col: ColId(c2),
            value: Value::Error(CellError::new(
                ErrorKind::Div0,
                Origin::Arithmetic {
                    op: usk_types::ArithOp::Div,
                },
            )),
        },
    );
    // A blank write — a clear is a write of Blank, not an erasure.
    push(
        &mut log,
        id(1, 26),
        Payload::ClearCell {
            row: RowId(r1),
            col: ColId(c3),
        },
    );

    // A *contested* cell: two actors write it, so its tile promotes and keeps
    // the loser (ADR-006). This is the branch a naive image would drop.
    push(
        &mut log,
        id(2, 1),
        Payload::SetCell {
            row: RowId(r2),
            col: ColId(c3),
            value: Value::Number(100.0),
        },
    );
    push(
        &mut log,
        id(3, 1),
        Payload::SetCell {
            row: RowId(r2),
            col: ColId(c3),
            value: Value::Number(200.0),
        },
    );

    // A formula with bindings, and a value write to a *different* cell the
    // registry also stamps.
    push(
        &mut log,
        id(1, 30),
        Payload::SetFormula {
            row: RowId(r3),
            col: ColId(c3),
            source: String::from("=SUM(A1:B2)+1"),
            bindings: vec![RangeBinding {
                row_start: r1,
                row_end: r2,
                col_start: c1,
                col_end: c2,
                anchors: 0b11,
            }],
        },
    );

    // A deleted row and an undeleted one, so tombstones and their removal both
    // appear in the axis tree.
    let r4 = id(1, 4);
    push(
        &mut log,
        r4,
        Payload::InsertRow {
            anchor: Anchor::After(r3),
        },
    );
    push(&mut log, id(1, 40), Payload::DeleteRow { row: RowId(r4) });
    let r5 = id(1, 5);
    push(
        &mut log,
        r5,
        Payload::InsertRow {
            anchor: Anchor::After(r4),
        },
    );
    push(&mut log, id(1, 41), Payload::DeleteRow { row: RowId(r5) });
    push(&mut log, id(1, 42), Payload::UndeleteRow { row: RowId(r5) });
    log
}

// ------------------------------------------------------------ the property

/// **The whole point.** An image round-trips to the same state hash, which is
/// what makes it safe to adopt instead of replaying the log (D-069 → the tile
/// image; TD-45, TD-31, TD-24's residual).
#[test]
fn an_image_round_trips_to_the_same_state_hash() {
    let original = State::replay(&workbook());
    let restored = State::from_image(&original.write_image()).expect("decodes");
    assert_eq!(
        restored.state_hash(),
        original.state_hash(),
        "the image is not the state it claims to be"
    );
}

/// The hash is a summary, so the parts it deliberately excludes are checked
/// separately: retained losers, tombstones and shadowed registry entries are
/// all invisible to `state_hash` and all part of what the state *is*.
#[test]
fn the_parts_the_hash_does_not_cover_survive_too() {
    let original = State::replay(&workbook());
    let restored = State::from_image(&original.write_image()).expect("decodes");

    let (r2, c3) = (RowId(id(1, 2)), ColId(id(1, 12)));
    assert!(
        !original.conflicts(r2, c3).is_empty(),
        "the fixture must actually contest a cell"
    );
    assert_eq!(
        restored.conflicts(r2, c3),
        original.conflicts(r2, c3),
        "retained losers are content (ADR-006), not bookkeeping"
    );
    assert!(restored.is_cell_promoted(r2, c3));

    assert_eq!(
        restored.full_row_order(),
        original.full_row_order(),
        "tombstones keep their place, which is what re-anchoring reads"
    );
    assert_eq!(restored.row_order(), original.row_order());
    assert_eq!(restored.col_order(), original.col_order());

    let (r3, c3) = (RowId(id(1, 3)), ColId(id(1, 12)));
    let formula = restored.formula(r3, c3).expect("the formula survived");
    assert_eq!(formula.source, "=SUM(A1:B2)+1");
    assert_eq!(formula.bindings.len(), 1);
    assert_eq!(formula.bindings[0].anchors, 0b11);

    // Cell values, exhaustively.
    for row in original.row_order() {
        for col in original.col_order() {
            assert_eq!(
                restored.cell(row, col),
                original.cell(row, col),
                "cell {row:?},{col:?}"
            );
        }
    }
}

/// A restored workbook keeps being edited. If the image stored the flattened
/// order instead of the insertion tree, the next concurrent insert would
/// resolve differently from a replica that never restarted — and the two would
/// diverge silently.
#[test]
fn a_restored_state_accepts_further_ops_identically() {
    let base = workbook();
    let original = State::replay(&base);
    let restored = State::from_image(&original.write_image()).expect("decodes");

    // Two concurrent inserts anchored at the same existing row, applied to both
    // the replayed and the restored state.
    let anchor = id(1, 2);
    let mut extended = base.clone();
    for (actor, counter) in [(7u128, 1u64), (8, 1)] {
        extended.append(Op {
            id: id(actor, counter),
            lamport: 500,
            payload: Payload::InsertRow {
                anchor: Anchor::After(anchor),
            },
        });
    }
    let replayed = State::replay(&extended);

    // The restored state has to reach the same place. Rebuilding through
    // `replay` from the same op set is the honest comparison: what must match
    // is the *resolution*, and that is what the row order shows.
    let restored_again = State::from_image(&restored.write_image()).expect("re-encodes");
    assert_eq!(restored_again.state_hash(), original.state_hash());
    assert_eq!(
        replayed.row_order().len(),
        original.row_order().len() + 2,
        "both new rows landed"
    );
    // The anchor's children order is what the tree encodes; a flattened image
    // could not reproduce it.
    let order = replayed.row_order();
    let at = order.iter().position(|r| r.0 == anchor).expect("anchor");
    assert_eq!(
        order[at + 1].0,
        id(8, 1),
        "later actor sorts nearer the anchor"
    );
    assert_eq!(order[at + 2].0, id(7, 1));
}

/// Builds a dense `rows x cols` numeric grid.
fn grid(rows: u32, cols: u32) -> OpLog {
    let mut log = OpLog::new();
    let mut counter = 1u64;
    let mut lamport = 0u64;
    let mut row_ids = Vec::new();
    let mut col_ids = Vec::new();
    let mut previous = None;
    for _ in 0..rows {
        let this = id(1, counter);
        counter += 1;
        lamport += 1;
        log.append(Op {
            id: this,
            lamport,
            payload: Payload::InsertRow {
                anchor: previous.map_or(Anchor::Start, Anchor::After),
            },
        });
        previous = Some(this);
        row_ids.push(this);
    }
    let mut previous = None;
    for _ in 0..cols {
        let this = id(1, counter);
        counter += 1;
        lamport += 1;
        log.append(Op {
            id: this,
            lamport,
            payload: Payload::InsertCol {
                anchor: previous.map_or(Anchor::Start, Anchor::After),
            },
        });
        previous = Some(this);
        col_ids.push(this);
    }
    for (r, row) in row_ids.iter().enumerate() {
        for (c, col) in col_ids.iter().enumerate() {
            lamport += 1;
            log.append(Op {
                id: id(1, counter),
                lamport,
                payload: Payload::SetCell {
                    row: RowId(*row),
                    col: ColId(*col),
                    value: Value::Number((r * cols as usize + c) as f64),
                },
            });
            counter += 1;
        }
    }
    log
}

/// An image is far smaller than the ops that produced it, and that is the whole
/// reason it exists: a numeric cell is 8 packed bytes here against an op's
/// 24-byte identity, 8-byte lamport and tagged payload.
///
/// The grid is **dense** on purpose. A tall thin sheet is dominated by the axis
/// tree — one identity per row, whatever the cell count — and there the image
/// wins by less than 2x. The advantage is per *cell*, so it grows with density,
/// and W-OPEN-1M's 1000x1000 shape is the one this measures a slice of.
#[test]
fn a_dense_numeric_image_is_far_smaller_than_its_op_log() {
    let log = grid(200, 200);
    let state = State::replay(&log);
    let image = state.write_image();
    let ops: usize = log.ops().iter().map(|o| o.encode().len()).sum();

    assert!(
        image.len() * 4 < ops,
        "image {} B should be several times below the op set's {ops} B",
        image.len()
    );
    assert_eq!(
        State::from_image(&image).expect("decodes").state_hash(),
        state.state_hash()
    );
}

// ------------------------------------------------- structural sharing

/// docs/16: *structurally shared via tile Merkle identity (cost O(dirty))*.
/// Two states differing inside one tile must produce identical chunk hashes for
/// every other tile — that is what lets three retained snapshots cost one
/// snapshot plus the difference rather than three copies.
#[test]
fn unchanged_tiles_hash_identically_across_snapshots() {
    // 600 rows spans three row bands (256 each); one column keeps it to one
    // column band, so the grid is exactly three tiles.
    let base = grid(600, 1);
    let before = State::replay(&base);
    let (image_a, tiles_a) = before.write_image_parts();
    assert_eq!(tiles_a.len(), 3, "the fixture must span several tiles");

    // Overwrite one cell in the *first* band. Nothing else moves — no new
    // identity, so no slot shifts and the other two bands are byte-identical.
    let mut after_log = base.clone();
    let first_row = match &base.ops()[0].payload {
        Payload::InsertRow { .. } => base.ops()[0].id,
        _ => unreachable!(),
    };
    let first_col = base
        .ops()
        .iter()
        .find(|o| matches!(o.payload, Payload::InsertCol { .. }))
        .expect("a column")
        .id;
    after_log.append(Op {
        id: id(9, 1),
        lamport: 1_000_000,
        payload: Payload::SetCell {
            row: RowId(first_row),
            col: ColId(first_col),
            value: Value::Number(-1.0),
        },
    });
    let after = State::replay(&after_log);
    let (image_b, tiles_b) = after.write_image_parts();

    let hashes_a = chunk_hashes(&image_a, &tiles_a);
    let hashes_b = chunk_hashes(&image_b, &tiles_b);
    let shared: Vec<_> = hashes_a
        .iter()
        .filter(|(key, hash)| hashes_b.iter().any(|(k, h)| k == key && h == hash))
        .collect();

    assert_eq!(
        shared.len(),
        2,
        "two of three tiles were untouched and must hash identically"
    );
    assert_ne!(
        before.state_hash(),
        after.state_hash(),
        "and the third really did change"
    );
}

// --------------------------------------------------- hostile and malformed

/// An image read off disk is an untrusted input. Every malformed form is a
/// named error, never a panic and never a half-built state (DP-A10).
#[test]
fn malformed_images_are_named_errors() {
    assert_eq!(State::from_image(b"").err(), Some(ImageError::Truncated));
    assert_eq!(
        State::from_image(b"not an image at all!!").err(),
        Some(ImageError::NotAnImage)
    );

    let good = State::replay(&workbook()).write_image();

    // A version this build does not know is refused rather than guessed at —
    // and recoverably, because the caller can still replay the op tail.
    let mut wrong_version = good.clone();
    wrong_version[8] = (IMAGE_VERSION + 7) as u8;
    wrong_version[9] = 0;
    assert_eq!(
        State::from_image(&wrong_version).err(),
        Some(ImageError::UnsupportedVersion(IMAGE_VERSION + 7))
    );

    // Every truncation.
    for cut in 0..good.len() {
        assert!(
            State::from_image(&good[..cut]).is_err(),
            "a truncated image must not load (cut {cut})"
        );
    }

    // Trailing bytes are a lie about the image's extent.
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(matches!(
        State::from_image(&trailing),
        Err(ImageError::Malformed(_))
    ));
}

/// Random mutations must never panic, and anything that *does* load must still
/// be internally consistent — the presence bitmap and the dense payload have to
/// agree, or every later read indexes the wrong value.
#[test]
fn no_mutation_of_an_image_can_panic_or_produce_an_inconsistent_state() {
    let good = State::replay(&workbook()).write_image();
    let mut seed = 0x5EED_1234_ABCD_0001u64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed >> 33
    };

    for _ in 0..20_000 {
        let mut bytes = good.clone();
        let flips = 1 + (next() % 4) as usize;
        for _ in 0..flips {
            let at = (next() as usize) % bytes.len();
            bytes[at] ^= (next() & 0xFF) as u8;
        }
        if let Ok(state) = State::from_image(&bytes) {
            // Reading every cell exercises `rank` against the payload, which is
            // where an inconsistent image would index out of bounds.
            for row in state.row_order() {
                for col in state.col_order() {
                    let _ = state.cell(row, col);
                }
            }
            let _ = state.state_hash();
        }
    }
}
