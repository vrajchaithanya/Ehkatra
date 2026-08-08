//! The relay endpoint: message handling on top of `usk_sync::Relay`.
//!
//! Fanout, retention and admission control — and nothing else. It never merges,
//! never resolves a conflict and never rewrites an op, so a compromised relay
//! can delay or withhold but cannot corrupt (docs/15, docs/37 boundary 2).

use usk_sync::machine::{wire_supported, Negotiated, MODEL_VERSION, WIRE_VERSION};
use usk_sync::relay::AdmissionError;
use usk_sync::Relay;
use usk_types::ActorId;

use crate::frame::Message;

/// What one inbound message produced.
#[derive(Default, Debug)]
pub struct RelayOut {
    pub to_sender: Vec<Message>,
    /// Fanned out to every subscriber except the sender.
    pub to_others: Vec<Message>,
}

/// Counters for the operator report (docs/36).
#[derive(Default, Debug, Clone, Copy)]
pub struct RelayStats {
    pub admitted: usize,
    pub spoofed: usize,
    pub invalid: usize,
    pub rate_limited: usize,
    pub byte_limited: usize,
}

#[derive(Default)]
pub struct RelayEndpoint {
    core: Relay,
    pub stats: RelayStats,
}

impl RelayEndpoint {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn retained(&self) -> usize {
        self.core.retained().len()
    }

    /// Handles one message from the connection authenticated as `principal`.
    ///
    /// `now_ms` is passed in rather than read: the token buckets must be
    /// reproducible in a test, and the relay is a pure function of its inputs.
    pub fn handle(&mut self, principal: ActorId, msg: Message, now_ms: u64) -> RelayOut {
        let mut out = RelayOut::default();
        match msg {
            Message::Hello(hello) => {
                // N−2 support (docs/15). A peer outside the window is told so
                // rather than left to time out.
                if wire_supported(hello.wire) {
                    out.to_sender.push(Message::HelloAck(Negotiated {
                        wire: hello.wire.min(WIRE_VERSION),
                        model: hello.model.min(MODEL_VERSION),
                    }));
                } else {
                    // The reject carries *our* version, so the peer can say
                    // what it must upgrade to rather than just that it failed.
                    out.to_sender.push(Message::HelloReject {
                        peer_wire: WIRE_VERSION,
                    });
                }
            }
            // Anti-entropy: give the peer exactly what its clock says it lacks,
            // then declare the exchange converged.
            Message::Need(clock) => {
                let missing = self.core.ops_missing_from(&clock);
                if !missing.is_empty() {
                    out.to_sender.push(Message::Give(missing));
                }
                out.to_sender.push(Message::InSync);
            }
            Message::Ops(ops) | Message::Give(ops) => {
                let admission = self.core.submit(principal, ops, now_ms);
                self.stats.admitted += admission.fanout.len();
                for (_, err) in &admission.refused {
                    match err {
                        AdmissionError::Spoofed => self.stats.spoofed += 1,
                        AdmissionError::Invalid => self.stats.invalid += 1,
                        AdmissionError::RateLimited => self.stats.rate_limited += 1,
                        AdmissionError::ByteLimited => self.stats.byte_limited += 1,
                    }
                }
                if !admission.fanout.is_empty() {
                    out.to_others.push(Message::Ops(admission.fanout));
                }
                // The ack is what releases the sender's never-drop queue, so it
                // is sent even when the batch was entirely redelivery.
                out.to_sender.push(Message::Ack(admission.watermark));
            }
            // A relay is not a replica: it has no queue to release and no
            // convergence of its own to declare.
            Message::Ack(_)
            | Message::InSync
            | Message::HelloAck(_)
            | Message::HelloReject { .. } => {}
        }
        out
    }
}
