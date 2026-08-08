//! A replica that connects to the relay, runs a scripted edit sequence, and
//! prints its state hash — the visible half of the two-terminal demo.
//!
//! ```text
//! ehkatra-peer <actor-number> <script>
//! ```
//!
//! `script` names one of the built-in edit sequences. Both peers build the
//! *same* grid from opposite directions and make concurrent structural edits,
//! so "the hashes match" is a claim about merge, not about doing nothing.

use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use ehkatra_relay::frame::{read_frame, write_frame, Message};
use ehkatra_relay::replica::Replica;
use ehkatra_relay::RELAY_PORT;
use usk_reduce::Command;
use usk_types::{ActorId, Value};

/// How long to keep exchanging after the last local edit before declaring the
/// session settled. Generous: this is a demo, not a benchmark.
const QUIESCE: Duration = Duration::from_millis(1500);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let actor_n: u128 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let script = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| String::from("alice"));

    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, RELAY_PORT);
    let stream = match TcpStream::connect(addr) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
            eprintln!("no relay on {addr} — start `ehkatra-relay` first");
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("connect failed: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = stream.set_nodelay(true) {
        eprintln!("set_nodelay: {e}");
    }

    let mut writer = stream.try_clone().expect("clone stream");
    let mut reader = stream;
    let (tx, rx): (Sender<Message>, Receiver<Message>) = channel();
    thread::spawn(move || loop {
        match read_frame(&mut reader) {
            Ok(m) => {
                if tx.send(m).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    });

    let mut replica = Replica::new(
        ActorId(actor_n),
        (actor_n as u32).wrapping_mul(2_654_435_761),
    );
    let send = |w: &mut TcpStream, msgs: Vec<Message>| {
        for m in msgs {
            if let Err(e) = write_frame(w, &m) {
                eprintln!("write failed: {e}");
                std::process::exit(1);
            }
        }
    };

    println!("peer {actor_n} ({script}) connecting to {addr}");
    send(&mut writer, replica.connect());

    // Wait for the handshake to complete before editing, so the demo shows the
    // protocol working rather than a queue draining.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !replica.is_live() && Instant::now() < deadline {
        if let Ok(msg) = rx.recv_timeout(Duration::from_millis(50)) {
            let out = replica.receive(msg);
            send(&mut writer, out);
        }
    }
    if !replica.is_live() {
        eprintln!("handshake did not complete");
        std::process::exit(1);
    }
    println!("peer {actor_n}: LIVE");

    // A peer that joins an empty workbook cannot address a cell that has no
    // row yet — the reducer refuses out-of-range coordinates rather than
    // inventing structure (DP-A1: only ops create rows). So a peer whose script
    // writes *into* a grid waits for that grid to arrive first. This is a real
    // property of the model, not a demo workaround: structure is data too.
    let (need_rows, need_cols) = required_grid(&script);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let rows = replica.doc.state().row_order().len();
        let cols = replica.doc.state().col_order().len();
        if rows >= need_rows && cols >= need_cols {
            break;
        }
        if let Ok(msg) = rx.recv_timeout(Duration::from_millis(50)) {
            let out = replica.receive(msg);
            send(&mut writer, out);
        }
    }

    let mut applied = 0usize;
    let mut refused = 0usize;
    for (n, cmd) in edits(&script).into_iter().enumerate() {
        match replica.edit(cmd) {
            Ok(msgs) => {
                applied += 1;
                send(&mut writer, msgs);
            }
            Err(e) => {
                refused += 1;
                eprintln!("edit {n} refused: {e:?}");
            }
        }
        // Drain anything that arrived while we were editing.
        while let Ok(msg) = rx.try_recv() {
            let out = replica.receive(msg);
            send(&mut writer, out);
        }
        thread::sleep(Duration::from_millis(60));
    }
    println!("peer {actor_n}: {applied} local edits applied, {refused} refused");

    // Quiesce: keep merging until the line goes quiet.
    let mut last = Instant::now();
    while last.elapsed() < QUIESCE {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(msg) => {
                let out = replica.receive(msg);
                send(&mut writer, out);
                last = Instant::now();
            }
            Err(_) => continue,
        }
    }

    let rows = replica.doc.state().row_order().len();
    let cols = replica.doc.state().col_order().len();
    println!("peer {actor_n}: {rows} rows x {cols} cols");
    println!(
        "peer {actor_n}: unacked local ops = {}",
        replica.unacked().len()
    );
    println!(
        "peer {actor_n}: quarantined remote ops = {}",
        replica.rejected.len()
    );
    println!("peer {actor_n}: log = {} ops", replica.log().len());
    println!("STATE HASH {actor_n}: {}", replica.state_hash().to_hex());
}

/// How much structure a script needs before its writes can bind. Alice creates
/// her own; Bob writes into Alice's.
fn required_grid(script: &str) -> (usize, usize) {
    match script {
        "alice" => (0, 0),
        _ => (4, 3),
    }
}

/// The two scripts build one grid together and then edit it concurrently: Alice
/// inserts a row in the middle of Bob's data while Bob is writing into it. That
/// is the canonical CRDT case (docs/15), so agreeing hashes mean the merge held.
fn edits(script: &str) -> Vec<Command> {
    let mut cmds = Vec::new();
    match script {
        "alice" => {
            for _ in 0..4 {
                cmds.push(Command::InsertRow { before: 0 });
            }
            for _ in 0..3 {
                cmds.push(Command::InsertCol { before: 0 });
            }
            for r in 0..4 {
                cmds.push(Command::SetValue {
                    row: r,
                    col: 0,
                    value: Value::Number((r as f64 + 1.0) * 10.0),
                });
            }
            // The concurrent structural edit.
            cmds.push(Command::InsertRow { before: 2 });
            cmds.push(Command::SetValue {
                row: 2,
                col: 0,
                value: Value::Text(String::from("alice-inserted")),
            });
        }
        _ => {
            for r in 0..4 {
                cmds.push(Command::SetValue {
                    row: r,
                    col: 1,
                    value: Value::Number((r as f64 + 1.0) * 100.0),
                });
            }
            cmds.push(Command::SetFormula {
                row: 0,
                col: 2,
                source: String::from("=SUM(B1:B4)"),
            });
            cmds.push(Command::SetValue {
                row: 3,
                col: 1,
                value: Value::Text(String::from("bob-overwrote")),
            });
        }
    }
    cmds
}
