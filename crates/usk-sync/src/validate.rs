//! Schema and bounds validation on receive (DP-E4, docs/37 boundary 2).
//!
//! > *a permitted collaborator is still an untrusted input source*
//!
//! An op that fails validation is **quarantined**: never applied, never merged
//! into the log, never retransmitted, and reported to the user. docs/27 §1 lists
//! applying such an op as forbidden, and is explicit that the session stays LIVE
//! — one hostile op must not cost a legitimate collaborator their connection.
//!
//! # Where the limits come from
//! The size caps are Excel's own documented cell limits, which makes them
//! simultaneously a security bound and a compatibility bound (docs/32): a
//! workbook that exceeds them could not round-trip anyway. Choosing numbers that
//! already had to exist beats inventing security-flavoured constants.

use alloc::vec::Vec;
use usk_oplog::{Op, Payload};
use usk_types::{ActorId, Value};

/// Excel's formula-length limit, in bytes (8,192 characters).
pub const MAX_FORMULA_SOURCE: usize = 8192;
/// Excel's cell text limit, in bytes (32,767 characters).
pub const MAX_TEXT_BYTES: usize = 32_767;
/// References per formula. Excel's limit is 255 arguments; a formula may nest,
/// so this is deliberately looser while still bounded.
pub const MAX_BINDINGS: usize = 1024;
/// How far ahead of the receiver's own frontier a lamport may legitimately be.
///
/// Unbounded lamports are an attack, not a hypothetical: an op stamped
/// `u64::MAX` would win every LWW comparison at its cell for the rest of the
/// workbook's life, and no later honest edit could ever displace it. A replica
/// that has been offline mints one lamport per local op, so 2^32 is beyond any
/// honest history while still rejecting the saturating value.
pub const MAX_LAMPORT_JUMP: u64 = 1 << 32;
/// Bytes a preserved-opaque op (DP-A5) may carry. Sized as the largest thing a
/// *known* op can be — a cell's text — because a future op type that needs more
/// than one Excel cell's worth of payload is not the case forward preservation
/// was designed for, and an unbounded one is a memory-amplification attack that
/// costs the sender nothing.
pub const MAX_OPAQUE_BYTES: usize = MAX_TEXT_BYTES;

/// Why an op was refused. Reported to the user (docs/28 domain 2), never
/// swallowed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RejectReason {
    /// Counters are 1-based; 0 is not a value the reducer can mint.
    ZeroCounter,
    /// Lamport 0 would sort below every honest op at the same cell.
    ZeroLamport,
    /// The op claims an actor other than the authenticated principal
    /// (spoofing, docs/37 boundary 2).
    ActorMismatch,
    /// Lamport implausibly far ahead of the receiver's frontier.
    LamportOutOfBounds,
    FormulaTooLong,
    TooManyBindings,
    TextTooLong,
    /// A `SetFormula` binding whose endpoints have zero counters.
    MalformedBinding,
    /// A preserved-opaque op (DP-A5) larger than [`MAX_OPAQUE_BYTES`].
    OpaqueTooLong,
}

/// A refused op, kept with its reason so the report can name both.
#[derive(Clone, PartialEq, Debug)]
pub struct Rejection {
    pub op: Op,
    pub reason: RejectReason,
}

/// Validates one op against the receiver's context.
///
/// `principal` is the actor the transport authenticated; `None` skips the
/// spoofing check, which is correct for a replica reading its own durable queue
/// and wrong for anything arriving over a socket.
pub fn validate(
    op: &Op,
    principal: Option<ActorId>,
    frontier_lamport: u64,
) -> Result<(), RejectReason> {
    if op.id.counter == 0 {
        return Err(RejectReason::ZeroCounter);
    }
    if op.lamport == 0 {
        return Err(RejectReason::ZeroLamport);
    }
    if let Some(p) = principal {
        if op.id.actor != p {
            return Err(RejectReason::ActorMismatch);
        }
    }
    if op.lamport > frontier_lamport.saturating_add(MAX_LAMPORT_JUMP) {
        return Err(RejectReason::LamportOutOfBounds);
    }
    match &op.payload {
        Payload::SetCell { row, col, value } => {
            check_identity(row.0.counter, col.0.counter)?;
            if let Value::Text(s) = value {
                if s.len() > MAX_TEXT_BYTES {
                    return Err(RejectReason::TextTooLong);
                }
            }
        }
        Payload::ClearCell { row, col } => check_identity(row.0.counter, col.0.counter)?,
        Payload::SetFormula {
            row,
            col,
            source,
            bindings,
        } => {
            check_identity(row.0.counter, col.0.counter)?;
            if source.len() > MAX_FORMULA_SOURCE {
                return Err(RejectReason::FormulaTooLong);
            }
            if bindings.len() > MAX_BINDINGS {
                return Err(RejectReason::TooManyBindings);
            }
            for b in bindings {
                if b.row_start.counter == 0
                    || b.row_end.counter == 0
                    || b.col_start.counter == 0
                    || b.col_end.counter == 0
                {
                    return Err(RejectReason::MalformedBinding);
                }
            }
        }
        Payload::DeleteRow { row } | Payload::UndeleteRow { row } => {
            if row.0.counter == 0 {
                return Err(RejectReason::MalformedBinding);
            }
        }
        Payload::DeleteCol { col } | Payload::UndeleteCol { col } => {
            if col.0.counter == 0 {
                return Err(RejectReason::MalformedBinding);
            }
        }
        // An insert names its anchor, and an anchor referring to an op we have
        // not seen is a *causal gap*, not an invalid op — the causal buffer
        // holds it. Confusing the two would quarantine honest work.
        Payload::InsertRow { .. } | Payload::InsertCol { .. } => {}
        // DP-A5 requires us to retransmit an op we cannot read, which means we
        // must store it — so it gets a bound like every other untrusted input.
        // The bound is the *only* check available: the payload's meaning is by
        // definition unknown here, and inventing a stricter rule would refuse
        // exactly the forward-compatible ops preservation exists to carry.
        Payload::Opaque(o) => {
            if o.body().len() > MAX_OPAQUE_BYTES {
                return Err(RejectReason::OpaqueTooLong);
            }
        }
    }
    Ok(())
}

fn check_identity(row: u64, col: u64) -> Result<(), RejectReason> {
    if row == 0 || col == 0 {
        return Err(RejectReason::MalformedBinding);
    }
    Ok(())
}

/// Splits a batch into the ops that may be applied and those quarantined.
pub fn partition(
    ops: Vec<Op>,
    principal: Option<ActorId>,
    frontier_lamport: u64,
) -> (Vec<Op>, Vec<Rejection>) {
    let mut ok = Vec::with_capacity(ops.len());
    let mut bad = Vec::new();
    for op in ops {
        match validate(&op, principal, frontier_lamport) {
            Ok(()) => ok.push(op),
            Err(reason) => bad.push(Rejection { op, reason }),
        }
    }
    (ok, bad)
}
