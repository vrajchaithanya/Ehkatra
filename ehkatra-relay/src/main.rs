//! The relay server: loopback-only, fail-fast, fanout + admission control.
//!
//! DP-S5 host isolation is not advice here, it is the binding contract: the
//! listener is `127.0.0.1` and nothing else, on the port docs/07 reserves, and
//! the process exits rather than competing for a port something else already
//! holds. No service registration, no admin, no firewall change.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use ehkatra_relay::endpoint::RelayEndpoint;
use ehkatra_relay::frame::{read_frame, write_frame, Message};
use ehkatra_relay::RELAY_PORT;
use usk_types::ActorId;

/// Everything the connection threads share.
struct Hub {
    endpoint: RelayEndpoint,
    /// Outbound queues, one per live connection.
    peers: HashMap<u64, Sender<Message>>,
}

fn main() {
    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, RELAY_PORT);
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            eprintln!(
                "ehkatra-relay: port {RELAY_PORT} is already in use — refusing to \
                 fight for it (DP-S5 fail-fast). Stop the other process and retry."
            );
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("ehkatra-relay: could not bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    println!("ehkatra-relay listening on {addr} (loopback only)");

    let hub = Arc::new(Mutex::new(Hub {
        endpoint: RelayEndpoint::new(),
        peers: HashMap::new(),
    }));
    let started = Instant::now();
    let mut next_conn = 0u64;

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("accept failed: {e}");
                continue;
            }
        };
        next_conn += 1;
        let conn = next_conn;
        let hub = Arc::clone(&hub);
        thread::spawn(move || {
            if let Err(e) = serve(conn, stream, hub, started) {
                if e.kind() != ErrorKind::UnexpectedEof && e.kind() != ErrorKind::ConnectionReset {
                    eprintln!("connection {conn} ended: {e}");
                }
            }
        });
    }
}

fn serve(
    conn: u64,
    stream: TcpStream,
    hub: Arc<Mutex<Hub>>,
    started: Instant,
) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut reader = stream.try_clone()?;
    let mut writer = stream;

    let (tx, rx) = channel::<Message>();
    if let Ok(mut h) = hub.lock() {
        h.peers.insert(conn, tx);
    }

    // One writer thread per connection, so a slow peer cannot block the relay.
    let writer_thread = thread::spawn(move || {
        for msg in rx {
            if write_frame(&mut writer, &msg).is_err() {
                break;
            }
        }
    });

    // The principal is bound at HELLO and never re-read: every later op from
    // this socket is checked against it, which is what makes actor spoofing
    // structurally impossible rather than merely detected (docs/37 boundary 2).
    let mut principal: Option<ActorId> = None;

    let result = loop {
        let msg = match read_frame(&mut reader) {
            Ok(m) => m,
            Err(e) => break Err(e),
        };
        if let Message::Hello(h) = &msg {
            if principal.is_none() {
                principal = Some(h.actor);
                println!("connection {conn}: actor {:#x} joined", h.actor.0);
            }
        }
        let Some(actor) = principal else {
            eprintln!("connection {conn}: message before HELLO — closing");
            break Ok(());
        };

        let now_ms = started.elapsed().as_millis() as u64;
        let out = {
            let Ok(mut h) = hub.lock() else {
                break Ok(());
            };
            let out = h.endpoint.handle(actor, msg, now_ms);
            for m in &out.to_others {
                for (id, peer) in h.peers.iter() {
                    if *id != conn {
                        let _ = peer.send(m.clone());
                    }
                }
            }
            out
        };
        if let Ok(h) = hub.lock() {
            if let Some(me) = h.peers.get(&conn) {
                for m in out.to_sender {
                    let _ = me.send(m);
                }
            }
        }
    };

    if let Ok(mut h) = hub.lock() {
        h.peers.remove(&conn);
        let s = h.endpoint.stats;
        println!(
            "connection {conn} closed — relay admitted {} ops (refused: {} spoofed, {} invalid, \
             {} rate, {} bytes)",
            s.admitted, s.spoofed, s.invalid, s.rate_limited, s.byte_limited
        );
    }
    // The peer's Sender is gone, so the writer thread's channel is closed and
    // it exits on its own; joining here just keeps the shutdown ordered.
    let _ = writer_thread.join();
    result
}
