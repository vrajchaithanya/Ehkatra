//! Codec conformance — BOOTSTRAP row 2's "encode/decode round-trip tests".
//!
//! The encoder shipped in Row 1; the decoder arrived with Row 10, because a
//! socket is the first thing that ever had to read an op back. These tests hold
//! the two halves to the canonical-encoding rule (DP-A4): exactly one byte
//! string per op, and decoding it returns the op unchanged.

use usk_oplog::{
    is_known_facet, is_known_tag, Alignment, Anchor, AxisSpan, DecodeError, FontFacet, Op, OpLog,
    OpaqueOp, Payload, RangeBinding, StyleFacet, StyleTarget, UnknownFacet, FONT_BOLD, FONT_ITALIC,
    FONT_STRIKE, FONT_UNDERLINE, MAX_OP_BYTES,
};
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
    // Styles (ADR-041). Every span shape and every facet shape, because the
    // list above is the one docs/29 asks to be kept complete and these are the
    // first new variants since the taxonomy was sealed.
    let targets = [
        StyleTarget::cell(cell().0, cell().1),
        StyleTarget {
            rows: AxisSpan::All,
            cols: AxisSpan::Between(id(3, 1), id(3, 4)),
        },
        StyleTarget {
            rows: AxisSpan::Between(id(2, 1), id(2, 9)),
            cols: AxisSpan::All,
        },
        StyleTarget {
            rows: AxisSpan::All,
            cols: AxisSpan::All,
        },
    ];
    let facets = [
        StyleFacet::NumberFormat(String::from("#,##0.00;[Red](#,##0.00)")),
        // An empty format code: the zero-length facet body, which is the case a
        // length prefix is easiest to get wrong on.
        StyleFacet::NumberFormat(String::new()),
        StyleFacet::Font(FontFacet {
            flags: FONT_BOLD | FONT_ITALIC | FONT_UNDERLINE | FONT_STRIKE,
            half_points: 21,
            argb: 0xFFC0_00FF,
            name: String::from("Segoe UI — üñî"),
        }),
        // An empty font name: the font body at its minimum length.
        StyleFacet::Font(FontFacet {
            flags: 0,
            half_points: 0,
            argb: 0,
            name: String::new(),
        }),
        StyleFacet::Fill(0xFFFF_FF00),
        StyleFacet::Align(Alignment {
            horizontal: 3,
            vertical: 2,
            wrap: true,
        }),
        StyleFacet::Align(Alignment {
            horizontal: 0,
            vertical: 0,
            wrap: false,
        }),
        StyleFacet::Unknown(
            UnknownFacet::new(0x40, vec![0xDE, 0xAD, 0xBE, 0xEF]).expect("0x40 is not ours"),
        ),
        // An unknown facet with an empty body: nothing to preserve, and it must
        // still round-trip rather than becoming a differently-shaped nothing.
        StyleFacet::Unknown(UnknownFacet::new(0x7F, Vec::new()).expect("0x7F is not ours")),
    ];
    let mut counter = 200u64;
    for target in targets {
        for facet in facets.clone() {
            counter += 1;
            ops.push(op(counter, Payload::SetStyle { target, facet }));
        }
        for facet_slot in [0x01, 0x02, 0x03, 0x04, 0x05, 0xFF] {
            counter += 1;
            ops.push(op(counter, Payload::ClearStyle { target, facet_slot }));
        }
    }
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

// ------------------------------------- TD-25: framing and forward preservation

/// A framed op from a build one model version ahead of this one: a tag we do
/// not know, carrying bytes we cannot interpret.
fn future_op(counter: u64, tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut body_bytes = Vec::new();
    body_bytes.extend_from_slice(&1u128.to_be_bytes());
    body_bytes.extend_from_slice(&counter.to_be_bytes());
    body_bytes.extend_from_slice(&(counter * 3).to_be_bytes());
    body_bytes.push(tag);
    body_bytes.extend_from_slice(body);
    out.extend_from_slice(&(body_bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(&body_bytes);
    out
}

/// The framed encoding is the unframed one plus a `u32` length — nothing about
/// an op's canonical bytes moved, which is why no hash did either (docs/26: the
/// container column holds "the identical bytes that were hashed").
#[test]
fn framing_wraps_the_canonical_encoding_without_altering_it() {
    for original in corpus() {
        let framed = original.encode_framed();
        let plain = original.encode();
        assert_eq!(&framed[..4], &(plain.len() as u32).to_be_bytes());
        assert_eq!(&framed[4..], &plain[..]);

        let (decoded, used) = Op::decode_framed(&framed).expect("decode framed");
        assert_eq!(used, framed.len());
        assert_eq!(decoded, original);
    }
}

/// **DP-A5, stated directly.** An op type this build does not know is
/// preserved, re-encodes to the author's exact bytes, and therefore hashes to
/// the hash the author computed.
#[test]
fn an_unknown_op_type_is_preserved_byte_for_byte() {
    let bytes = future_op(1, 0x42, b"a payload from the future");
    let (op, used) = Op::decode_framed(&bytes).expect("an unknown tag is not an error");
    assert_eq!(used, bytes.len());

    let Payload::Opaque(o) = &op.payload else {
        panic!("expected an opaque payload, got {:?}", op.payload);
    };
    assert_eq!(o.tag(), 0x42);
    assert_eq!(o.body(), b"a payload from the future");
    assert_eq!(op.id.counter, 1, "the header is still readable");
    assert_eq!(op.lamport, 3, "so the op can still be causally ordered");

    assert_eq!(
        op.encode_framed(),
        bytes,
        "re-encoding reproduces the author's bytes exactly"
    );
    assert_eq!(
        op.hash().as_bytes(),
        blake3::hash(&bytes[4..]).as_bytes(),
        "and therefore hashes opaque to the same value the author computed"
    );
}

/// The reason framing had to come first: without a length, an unknown tag ends
/// the stream, so **one op from a newer peer truncates everything behind it**.
#[test]
fn an_unknown_op_does_not_stop_the_ops_behind_it() {
    let known = corpus();
    let mut stream = Vec::new();
    stream.extend_from_slice(&known[0].encode_framed());
    stream.extend_from_slice(&future_op(77, 0xA0, b"..."));
    stream.extend_from_slice(&known[1].encode_framed());

    let mut at = 0usize;
    let mut out = Vec::new();
    while at < stream.len() {
        let (op, used) = Op::decode_framed(&stream[at..]).expect("stream keeps going");
        at += used;
        out.push(op);
    }
    assert_eq!(out.len(), 3);
    assert_eq!(out[0], known[0]);
    assert_eq!(out[2], known[1], "the op behind the unknown one is intact");
    assert!(matches!(out[1].payload, Payload::Opaque(_)));

    let mut round_tripped = Vec::new();
    for op in &out {
        round_tripped.extend_from_slice(&op.encode_framed());
    }
    assert_eq!(
        round_tripped, stream,
        "the whole stream retransmits verbatim"
    );
}

/// `is_known_tag` and the decoder's match are two lists that must agree. This
/// pins them to each other over all 256 tags rather than trusting whoever edits
/// one of them next.
#[test]
fn every_known_tag_decodes_and_every_other_is_opaque() {
    for tag in 0u8..=0xFF {
        let bytes = future_op(1, tag, b"\x00\x00\x00\x00\x00\x00\x00\x00");
        let opaque = matches!(
            Op::decode_framed(&bytes).map(|(op, _)| op.payload),
            Ok(Payload::Opaque(_))
        );
        assert_eq!(
            opaque,
            !is_known_tag(tag),
            "tag {tag:#04x}: opaque={opaque}, is_known_tag={}",
            is_known_tag(tag)
        );
    }
}

/// `is_known_facet` and the facet decoder's match are two lists that must
/// agree, exactly as `is_known_tag` and `Op::decode` are. Pinned over all 256
/// facet tags rather than trusted (ADR-041 decision 3).
#[test]
fn every_known_facet_decodes_and_every_other_is_unknown() {
    for tag in 0u8..=0xFF {
        // A four-byte body: valid for `Fill`, wrong length for the others, and
        // meaningless for an unknown tag — which is the point. What is asserted
        // is only *which arm* the tag reaches.
        let mut bytes = vec![tag];
        bytes.extend_from_slice(&4u32.to_be_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4]);
        let decoded = StyleFacet::decode(&bytes);
        match decoded {
            Ok(StyleFacet::Unknown(u)) => {
                assert!(
                    !is_known_facet(tag),
                    "tag {tag:#04x} is ours and was not decoded"
                );
                assert_eq!(u.tag(), tag);
                assert_eq!(u.body(), &[1, 2, 3, 4]);
            }
            other => assert!(
                is_known_facet(tag),
                "tag {tag:#04x} is not ours but did not become Unknown: {other:?}"
            ),
        }
    }
}

/// DP-A4 one level down: an unknown facet carrying a tag we *do* understand
/// would be a second spelling of a facet with exactly one, so it cannot be
/// built.
#[test]
fn an_unknown_facet_cannot_carry_a_tag_this_build_knows() {
    for tag in 0x01u8..=0x04 {
        assert!(
            UnknownFacet::new(tag, Vec::new()).is_none(),
            "facet tag {tag:#04x} is ours and must be decoded, not preserved"
        );
    }
    assert!(UnknownFacet::new(0x00, Vec::new()).is_some());
    assert!(UnknownFacet::new(0x05, Vec::new()).is_some());
}

/// The facet length prefix is checked, not trusted: a known facet whose body
/// does not fill the length it claims is a framing lie, and the decoder must
/// say so rather than read past it or silently accept the gap.
#[test]
fn a_facet_that_underfills_its_length_is_refused() {
    // Tag 0x03 is `Fill`, whose body is exactly four bytes. Claim six.
    let mut bytes = vec![0x03u8];
    bytes.extend_from_slice(&6u32.to_be_bytes());
    bytes.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
    assert_eq!(
        StyleFacet::decode(&bytes),
        Err(DecodeError::TrailingBytes { used: 4, len: 6 })
    );
}

/// An alignment discriminant outside its defined range is a named error, not a
/// silently different alignment — the same rule `Anchor` and `Origin` follow.
#[test]
fn an_out_of_range_alignment_is_a_named_error() {
    for body in [[9u8, 0, 0], [0, 9, 0], [0, 0, 2]] {
        let mut bytes = vec![0x04u8];
        bytes.extend_from_slice(&3u32.to_be_bytes());
        bytes.extend_from_slice(&body);
        assert_eq!(
            StyleFacet::decode(&bytes),
            Err(DecodeError::BadDiscriminant),
            "body {body:?} must not decode"
        );
    }
}

/// DP-A4 says one canonical byte string per op. An opaque op carrying a tag we
/// *do* understand would be a second spelling, so it cannot be built.
#[test]
fn an_opaque_op_cannot_carry_a_tag_this_build_knows() {
    for tag in 0x10u8..=0x1A {
        assert!(
            OpaqueOp::new(tag, Vec::new()).is_none(),
            "tag {tag:#04x} is ours and must be decoded, not preserved"
        );
    }
    assert!(OpaqueOp::new(0x1B, Vec::new()).is_some());
    assert!(OpaqueOp::new(0x00, Vec::new()).is_some());
}

/// "Hashed opaque" (DP-A5) means the op counts toward the op-set hash even
/// though it contributes nothing this build can read. Both halves are asserted,
/// because either alone would be a different — and wrong — design.
#[test]
fn an_unknown_op_is_hashed_into_the_log_it_cannot_change() {
    let (op, _) = Op::decode_framed(&future_op(50, 0xB1, b"future")).expect("decode");

    let mut without = OpLog::new();
    for known in corpus() {
        without.append(known);
    }
    let mut with = without.clone();
    with.append(op);

    assert_ne!(
        without.canonical_hash().as_bytes(),
        with.canonical_hash().as_bytes(),
        "a preserved op is part of the op set and must move its hash"
    );
}

/// A length prefix is an allocation request from an untrusted source, so it is
/// bounded (docs/37 boundary 2).
#[test]
fn a_frame_claiming_more_than_the_admission_bound_is_refused() {
    let mut bytes = (MAX_OP_BYTES as u32 + 1).to_be_bytes().to_vec();
    bytes.extend_from_slice(&[0u8; 8]);
    assert_eq!(
        Op::decode_framed(&bytes),
        Err(DecodeError::FrameTooLarge {
            len: MAX_OP_BYTES + 1
        })
    );
}

/// Every truncation of every framed op — prefix included — is a named error.
#[test]
fn a_truncated_frame_is_an_error_not_a_panic() {
    let mut framed: Vec<Vec<u8>> = corpus().iter().map(Op::encode_framed).collect();
    framed.push(future_op(9, 0xC3, b"future bytes"));
    for bytes in framed {
        for cut in 0..bytes.len() {
            assert_eq!(
                Op::decode_framed(&bytes[..cut]),
                Err(DecodeError::Truncated),
                "framed prefix of length {cut} should be Truncated"
            );
        }
    }
}

/// An op that decodes but leaves bytes unread inside its own frame is a framing
/// lie, not a valid op — the check that stops a peer smuggling data in the gap.
#[test]
fn an_op_that_underfills_its_frame_is_refused() {
    let op = corpus()[0].clone();
    let plain = op.encode();
    let mut bytes = ((plain.len() + 3) as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(&plain);
    bytes.extend_from_slice(b"pad");
    assert_eq!(
        Op::decode_framed(&bytes),
        Err(DecodeError::TrailingBytes {
            used: plain.len(),
            len: plain.len() + 3
        })
    );
}
