//! W-SYNC-RELAY (docs/38) — the Row 10 acceptance workload.
//!
//! > *2 and 50 replicas through one relay, each replica 10 ops/s for 60 s, 1%
//! > simulated packet loss. Measures: propagation p95; convergence time after
//! > last op; queued-op durability across a mid-run kill.*
//!
//! # What the numbers mean
//! Propagation and convergence are reported in **bus milliseconds** — the
//! simulated clock of the deterministic transport, where one hop costs
//! `LATENCY_MS`. That is deliberate: these are properties of the *protocol*
//! (how many round trips a fact needs to reach every replica), and measuring
//! them against a wall clock would report the speed of this laptop's memcpy
//! instead. Wall time is reported separately, and is a real number about the
//! implementation rather than the protocol.
//!
//! Everything is seeded (D-052), so a surprising run is reproducible from its
//! seed alone.

use std::time::Instant;

use ehkatra_relay::bus::Bus;
use ehkatra_relay::replica::Replica;
use usk_reduce::Command;
use usk_types::Value;

/// One-way link latency, bus milliseconds.
const LATENCY_MS: u64 = 5;
/// docs/38: 1% packet loss.
const LOSS_PERMILLE: u64 = 10;
/// docs/38: 10 ops/s for 60 s.
const OPS_PER_SEC: u64 = 10;
const DURATION_SEC: u64 = 60;
/// One tick per authored op, per replica.
const TICK_MS: u64 = 1000 / OPS_PER_SEC;
const TICKS: u64 = DURATION_SEC * OPS_PER_SEC;

/// How long the victim edits offline before the kill, in ticks.
const OFFLINE_TICKS: u64 = 30;
/// The shared workbook every replica edits into.
const ROWS: u32 = 20;
const COLS: u32 = 5;

struct Report {
    replicas: usize,
    ops: usize,
    dropped: usize,
    reconnects: usize,
    propagation_p50: u64,
    propagation_p95: u64,
    convergence_ms: u64,
    converged: bool,
    killed_queue: usize,
    killed_ops_delivered: usize,
    quarantined: usize,
    wall_ms: u128,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sizes: Vec<usize> = if args.iter().any(|a| a == "--quick") {
        vec![2]
    } else {
        vec![2, 50]
    };

    println!(
        "W-SYNC-RELAY — {DURATION_SEC}s @ {OPS_PER_SEC} ops/s/replica, \
              {}‰ loss, {LATENCY_MS} ms one-way link",
        LOSS_PERMILLE
    );
    println!();
    for n in sizes {
        let r = run(n, 0x5142_0000 + n as u64);
        print_report(&r);
    }
}

fn print_report(r: &Report) {
    println!("--- {} replicas ---", r.replicas);
    println!("  ops authored              {}", r.ops);
    println!(
        "  frames dropped            {} ({} reconnects)",
        r.dropped, r.reconnects
    );
    println!("  propagation p50           {} bus-ms", r.propagation_p50);
    println!("  propagation p95           {} bus-ms", r.propagation_p95);
    println!("  convergence after last op {} bus-ms", r.convergence_ms);
    println!(
        "  all replicas equal        {}",
        if r.converged {
            "YES"
        } else {
            "NO — DIVERGED"
        }
    );
    println!(
        "  mid-run kill              {} ops queued at death, {} delivered after recovery",
        r.killed_queue, r.killed_ops_delivered
    );
    println!("  quarantined remote ops    {}", r.quarantined);
    println!("  wall time                 {} ms", r.wall_ms);
    println!();
}

fn run(replicas: usize, seed: u64) -> Report {
    let started = Instant::now();
    let mut bus = Bus::new(replicas, seed, LATENCY_MS, LOSS_PERMILLE);
    bus.connect_all();

    // A shared grid, built by one replica and propagated before the run starts,
    // so the measured phase is cell traffic rather than structure.
    for _ in 0..ROWS {
        bus.edit(0, Command::InsertRow { before: 0 });
    }
    for _ in 0..COLS {
        bus.edit(0, Command::InsertCol { before: 0 });
    }
    bus.settle(2000);

    let kill_at = TICKS / 2;
    let victim = if replicas > 1 { 1 } else { 0 };
    let mut killed_queue = 0usize;
    let mut killed_ops: Vec<usk_oplog::Op> = Vec::new();
    let mut ops = 0usize;

    for tick in 0..TICKS {
        for i in 0..replicas {
            let row = ((tick as u32).wrapping_add(i as u32 * 7)) % ROWS;
            let col = (i as u32) % COLS;
            bus.edit(
                i,
                Command::SetValue {
                    row,
                    col,
                    value: Value::Number((tick * 100 + i as u64) as f64),
                },
            );
            ops += 1;
        }

        // Take the victim offline first, so it is holding a real backlog of
        // unacknowledged work when the process dies. A kill that catches an
        // empty queue proves nothing about never-drop.
        //
        // This must be a *partition*, not a dropped frame plus a long timer.
        // The first version of this harness used the latter, and at 50 replicas
        // any subsequent loss ran the teardown-and-reconnect path, armed a fresh
        // 500 ms timer, and let the victim rejoin and drain before the kill — so
        // the run reported 2 queued ops where it should have reported ~30, and
        // the durability measurement was quietly worthless.
        if tick == kill_at.saturating_sub(OFFLINE_TICKS) {
            bus.partition(victim);
        }

        if tick == kill_at {
            // KILL −9 mid-run: everything but the durable op log is lost.
            let log = bus.replicas[victim].log();
            let unacked = bus.replicas[victim].unacked();
            let actor = bus.replicas[victim].actor();
            killed_queue = unacked.len();
            killed_ops = unacked.clone();
            bus.replicas[victim] = Replica::recover(actor, 0x5EED, &log, &unacked);
            // Heal, not reconnect: the link comes back at the same moment the
            // process does, which is what a restart after a network outage
            // actually looks like.
            bus.heal(victim);
        }

        bus.advance(TICK_MS);
    }

    // Convergence time after the last op.
    let quiet_from = bus.now_ms;
    // Generous: a replica deep in docs/27's backoff schedule can legitimately
    // wait 60 s of bus time before its next attempt, and a 50-replica run has
    // many of them. A settle window shorter than the protocol's own worst-case
    // recovery would report "diverged" for a session that was merely waiting.
    bus.settle(120_000);
    let convergence_ms = bus.now_ms - quiet_from;

    // Did every op the victim was holding when it died actually arrive?
    let peer = if victim == 0 { replicas - 1 } else { 0 };
    let peer_log = bus.replicas[peer].log();
    let killed_ops_delivered = killed_ops
        .iter()
        .filter(|op| peer_log.iter().any(|o| o.id == op.id))
        .count();

    Report {
        replicas,
        ops,
        dropped: bus.dropped,
        reconnects: bus.reconnects,
        propagation_p50: bus.propagation_p50_ms(),
        propagation_p95: bus.propagation_p95_ms(),
        convergence_ms,
        converged: bus.converged(),
        killed_queue,
        killed_ops_delivered,
        quarantined: bus.replicas.iter().map(|r| r.rejected.len()).sum(),
        wall_ms: started.elapsed().as_millis(),
    }
}
