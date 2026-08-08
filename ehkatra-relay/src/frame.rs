//! The wire format: length-prefixed frames carrying protocol messages.
//!
//! # Why not WebSocket in v0.1 (D-065)
//! docs/15 names WS as the replica↔relay transport. A WS client+server pulls
//! roughly twenty crates (HTTP parsing, SHA-1 for the handshake, URL, RNG),
//! against a workspace dependency ceiling of 40 that currently stands at 10
//! (DP-S2) and a "no custom crypto, one hashing stack" rule (DP-B9). The sync
//! *semantics* — docs/27 §1's machine, never-drop queueing, anti-entropy,
//! admission control — are transport-independent and fully proven in-process,
//! and docs/20 puts transport at L3 where it "changes transport never
//! semantics". So v0.1 frames on loopback TCP and the WS upgrade is a shell
//! swap behind this module. Recorded as D-065 with debt TD-27.
//!
//! Framing: `u32 big-endian length ‖ u8 kind ‖ payload`. Length caps the frame
//! at `MAX_FRAME` so a hostile or corrupt peer cannot make the reader allocate
//! (docs/37 boundary 2).

use std::io::{self, Read, Write};

use usk_oplog::Op;
use usk_sync::machine::{Hello, Negotiated};
use usk_sync::VectorClock;
use usk_types::ActorId;

/// 16 MiB — comfortably above a legitimate op batch, far below "allocate
/// whatever the peer claims".
pub const MAX_FRAME: usize = 16 << 20;

/// Protocol messages (docs/15 §Protocol).
#[derive(Clone, PartialEq, Debug)]
pub enum Message {
    Hello(Hello),
    HelloAck(Negotiated),
    HelloReject {
        peer_wire: u16,
    },
    Need(VectorClock),
    Give(Vec<Op>),
    Ops(Vec<Op>),
    Ack(VectorClock),
    /// Anti-entropy converged: both sides hold the same op set.
    InSync,
}

const K_HELLO: u8 = 1;
const K_HELLO_ACK: u8 = 2;
const K_HELLO_REJECT: u8 = 3;
const K_NEED: u8 = 4;
const K_GIVE: u8 = 5;
const K_OPS: u8 = 6;
const K_ACK: u8 = 7;
const K_IN_SYNC: u8 = 8;

fn put_clock(clock: &VectorClock, out: &mut Vec<u8>) {
    let entries: Vec<(ActorId, u64)> = clock.actors().collect();
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (actor, counter) in entries {
        out.extend_from_slice(&actor.0.to_be_bytes());
        out.extend_from_slice(&counter.to_be_bytes());
    }
}

fn put_ops(ops: &[Op], out: &mut Vec<u8>) {
    out.extend_from_slice(&(ops.len() as u32).to_be_bytes());
    for op in ops {
        let bytes = op.encode();
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(&bytes);
    }
}

impl Message {
    /// The message as a complete frame, length prefix included.
    pub fn to_frame(&self) -> Vec<u8> {
        let mut body = Vec::new();
        match self {
            Message::Hello(h) => {
                body.push(K_HELLO);
                body.extend_from_slice(&h.actor.0.to_be_bytes());
                body.extend_from_slice(&h.wire.to_be_bytes());
                body.extend_from_slice(&h.model.to_be_bytes());
                put_clock(&h.clock, &mut body);
            }
            Message::HelloAck(n) => {
                body.push(K_HELLO_ACK);
                body.extend_from_slice(&n.wire.to_be_bytes());
                body.extend_from_slice(&n.model.to_be_bytes());
            }
            Message::HelloReject { peer_wire } => {
                body.push(K_HELLO_REJECT);
                body.extend_from_slice(&peer_wire.to_be_bytes());
            }
            Message::Need(c) => {
                body.push(K_NEED);
                put_clock(c, &mut body);
            }
            Message::Give(ops) => {
                body.push(K_GIVE);
                put_ops(ops, &mut body);
            }
            Message::Ops(ops) => {
                body.push(K_OPS);
                put_ops(ops, &mut body);
            }
            Message::Ack(c) => {
                body.push(K_ACK);
                put_clock(c, &mut body);
            }
            Message::InSync => body.push(K_IN_SYNC),
        }
        let mut frame = Vec::with_capacity(body.len() + 4);
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&body);
        frame
    }

    /// Parses a frame body (length prefix already consumed).
    pub fn from_body(body: &[u8]) -> io::Result<Message> {
        let mut r = Cursor { b: body, at: 0 };
        let kind = r.u8()?;
        Ok(match kind {
            K_HELLO => Message::Hello(Hello {
                actor: ActorId(r.u128()?),
                wire: r.u16()?,
                model: r.u16()?,
                clock: r.clock()?,
            }),
            K_HELLO_ACK => Message::HelloAck(Negotiated {
                wire: r.u16()?,
                model: r.u16()?,
            }),
            K_HELLO_REJECT => Message::HelloReject {
                peer_wire: r.u16()?,
            },
            K_NEED => Message::Need(r.clock()?),
            K_GIVE => Message::Give(r.ops()?),
            K_OPS => Message::Ops(r.ops()?),
            K_ACK => Message::Ack(r.clock()?),
            K_IN_SYNC => Message::InSync,
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unknown message kind {other}"),
                ))
            }
        })
    }
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        let end = self
            .at
            .checked_add(n)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "length overflow"))?;
        let s = self
            .b
            .get(self.at..end)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "short frame"))?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> io::Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> io::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> io::Result<u64> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_be_bytes(a))
    }
    fn u128(&mut self) -> io::Result<u128> {
        let b = self.take(16)?;
        let mut a = [0u8; 16];
        a.copy_from_slice(b);
        Ok(u128::from_be_bytes(a))
    }
    fn clock(&mut self) -> io::Result<VectorClock> {
        let n = self.u32()? as usize;
        let mut clock = VectorClock::new();
        for _ in 0..n {
            let actor = ActorId(self.u128()?);
            let counter = self.u64()?;
            // A clock is a summary, so it is rebuilt by replaying its run
            // rather than trusted as a number — a peer cannot claim coverage
            // by asserting a counter.
            for c in 1..=counter {
                clock.observe(usk_types::OpId { actor, counter: c });
            }
        }
        Ok(clock)
    }
    fn ops(&mut self) -> io::Result<Vec<Op>> {
        let n = self.u32()? as usize;
        let mut out = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            let len = self.u32()? as usize;
            let bytes = self.take(len)?;
            let (op, used) = Op::decode(bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
            if used != len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "trailing bytes inside an op frame",
                ));
            }
            out.push(op);
        }
        Ok(out)
    }
}

/// Reads exactly one frame from a stream.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Message> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Message::from_body(&body)
}

/// Writes one frame and flushes it.
pub fn write_frame<W: Write>(w: &mut W, msg: &Message) -> io::Result<()> {
    w.write_all(&msg.to_frame())?;
    w.flush()
}
