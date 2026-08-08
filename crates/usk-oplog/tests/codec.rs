//! Codec conformance — BOOTSTRAP row 2's "encode/decode round-trip tests".
//!
//! The encoder shipped in Row 1; the decoder arrived with Row 10, because a
//! socket is the first thing that ever had to read an op back. These tests hold
//! the two halves to the canonical-encoding rule (DP-A4): exactly one byte
//! string per op, and decoding it returns the op unchanged.

use usk_oplog::{Anchor, DecodeError, Op, Payload, RangeBinding};
use usk_types::{
    ActorId, ArithOp, CellError, ColId, Decimal, ErrorKind, OpId, Origin, RowId, TypeTag, Value,
};

fn id(actor: u128, counter: u64) -> OpId {
    OpId {
        actor: ActorId(actor),
        counter,
    }
}

fn op(counter: u64, payload: Payload) -> Op {
    Op {
        id: id(1, counter),
        lamport: counter * 3,
        payload,
    }
}

/// One op of every payload variant, carrying one value of every value variant.
/// The list is the thing the determinism guide (docs/29) asks to be kept
/// complete, so it is written to be obviously exhaustive.
fn corpus() -> Vec<Op> {
    let cell = || (RowId(id(2, 7)), ColId(id(3, 9)));
    let values = [
        Value::Blank,
        Value::Bool(false),
        Value::Bool(true),
        Value::Number(-0.5),
        Value::Number(f64::INFINITY),
        Value::Text(String::from("hello — üñî")),
        Value::Text(String::new()),
        Value::Decimal(Decimal::new(-1234567, -4)),
        Value::Decimal(Decimal::ZERO),
        Value::Error(CellError::new(ErrorKind::Div0, Origin::Authored)),
        Value::Error(CellError::new(
            ErrorKind::Value,
            Origin::Coercion {
                from: TypeTag::Text,
                to: TypeTag::Number,
            },
        )),
        Value::Error(CellError::new(
            ErrorKind::Num,
            Origin::Arithmetic { op: ArithOp::Div },
        )),
        Value::Error(CellError::new(ErrorKind::Ref, Origin::Propagated)),
    ];

    let mut ops = vec![
        op(
            1,
            Payload::InsertRow {
                anchor: Anchor::Start,
            },
        ),
        op(
            2,
            Payload::InsertRow {
                anchor: Anchor::After(id(4, 2)),
            },
        ),
        op(
            3,
            Payload::InsertCol {
                anchor: Anchor::Start,
            },
        ),
        op(
            4,
            Payload::InsertCol {
                anchor: Anchor::After(id(4, 3)),
            },
        ),
        op(5, Payload::DeleteRow { row: cell().0 }),
        op(6, Payload::DeleteCol { col: cell().1 }),
        op(
            7,
            Payload::ClearCell {
                row: cell().0,
                col: cell().1,
            },
        ),
        op(
            8,
            Payload::SetFormula {
                row: cell().0,
                col: cell().1,
                source: String::from("=SUM(A1:B2) & \"x\""),
                bindings: vec![
                    RangeBinding {
                        row_start: id(2, 1),
                        row_end: id(2, 2),
                        col_start: id(3, 1),
                        col_end: id(3, 2),
                        anchors: 0b11,
                    },
                    RangeBinding {
                        row_start: id(2, 3),
                        row_end: id(2, 3),
                        col_start: id(3, 3),
                        col_end: id(3, 3),
                        anchors: 0,
                    },
                ],
            },
        ),
        op(
            9,
            Payload::SetFormula {
                row: cell().0,
                col: cell().1,
                source: String::new(),
                bindings: Vec::new(),
            },
        ),
        op(10, Payload::UndeleteRow { row: cell().0 }),
        op(11, Payload::UndeleteCol { col: cell().1 }),
    ];
    for (n, value) in values.into_iter().enumerate() {
        ops.push(op(
            100 + n as u64,
            Payload::SetCell {
                row: cell().0,
                col: cell().1,
                value,
            },
        ));
    }
    ops
}

#[test]
fn every_op_variant_round_trips_through_the_codec() {
    for original in corpus() {
        let bytes = original.encode();
        let (decoded, used) = Op::decode(&bytes).expect("decode");
        assert_eq!(used, bytes.len(), "decoder consumed the whole op");
        assert_eq!(decoded, original);
        assert_eq!(
            decoded.encode(),
            bytes,
            "re-encoding must reproduce the same bytes (DP-A4)"
        );
    }
}

#[test]
fn ops_decode_back_to_back_from_one_buffer() {
    let ops = corpus();
    let mut buffer = Vec::new();
    for op in &ops {
        buffer.extend_from_slice(&op.encode());
    }
    let mut at = 0usize;
    let mut out = Vec::new();
    while at < buffer.len() {
        let (op, used) = Op::decode(&buffer[at..]).expect("decode");
        at += used;
        out.push(op);
    }
    assert_eq!(out, ops);
}

/// Every truncation of every op is an error, never a panic and never a
/// half-built op. This is the property that matters on a socket, where a short
/// read is normal (DP-A10, DP-E2).
#[test]
fn truncated_input_is_an_error_not_a_panic() {
    for op in corpus() {
        let bytes = op.encode();
        for cut in 0..bytes.len() {
            assert_eq!(
                Op::decode(&bytes[..cut]),
                Err(DecodeError::Truncated),
                "prefix of length {cut} should be Truncated"
            );
        }
    }
}

#[test]
fn unknown_tags_and_bad_discriminants_are_named_errors() {
    let mut bytes = op(
        1,
        Payload::InsertRow {
            anchor: Anchor::Start,
        },
    )
    .encode();
    let tag_at = 16 + 8 + 8; // actor ‖ counter ‖ lamport
    bytes[tag_at] = 0x7F;
    assert_eq!(Op::decode(&bytes), Err(DecodeError::UnknownTag(0x7F)));

    bytes[tag_at] = 0x10;
    bytes[tag_at + 1] = 0x09; // anchor discriminant is 0x00 or 0x01
    assert_eq!(Op::decode(&bytes), Err(DecodeError::BadDiscriminant));

    let cell = (RowId(id(2, 7)), ColId(id(3, 9)));
    let mut text = op(
        1,
        Payload::SetCell {
            row: cell.0,
            col: cell.1,
            value: Value::Text(String::from("ok")),
        },
    )
    .encode();
    let last = text.len() - 1;
    text[last] = 0xFF; // invalid UTF-8 continuation
    assert_eq!(Op::decode(&text), Err(DecodeError::BadUtf8));
}

/// A hostile peer must not be able to send a second byte string for one value.
/// The decoder canonicalises on the way in, so it cannot.
#[test]
fn a_non_canonical_decimal_is_repaired_on_decode() {
    let canonical = Value::Decimal(Decimal::new(15, -1)); // 1.5
    let mut bytes = Vec::new();
    canonical.encode_into(&mut bytes);

    // Hand-built alternative spelling of the same number: 150 × 10^-2.
    let mut hostile = vec![0x06];
    hostile.extend_from_slice(&150i128.to_be_bytes());
    hostile.extend_from_slice(&(-2i16).to_be_bytes());
    assert_ne!(hostile, bytes, "the two spellings really do differ");

    let wrap = |payload: Vec<u8>| {
        let mut b = Vec::new();
        b.extend_from_slice(&1u128.to_be_bytes());
        b.extend_from_slice(&1u64.to_be_bytes());
        b.extend_from_slice(&1u64.to_be_bytes());
        b.push(0x14);
        b.extend_from_slice(&id(2, 7).actor.0.to_be_bytes());
        b.extend_from_slice(&7u64.to_be_bytes());
        b.extend_from_slice(&id(3, 9).actor.0.to_be_bytes());
        b.extend_from_slice(&9u64.to_be_bytes());
        b.extend_from_slice(&payload);
        b
    };

    let (decoded, _) = Op::decode(&wrap(hostile)).expect("decode");
    let Payload::SetCell { value, .. } = &decoded.payload else {
        panic!("expected SetCell");
    };
    assert_eq!(*value, canonical);
    assert_eq!(
        decoded.encode(),
        wrap(bytes),
        "re-encoding emits the one canonical spelling"
    );
}
