//! `ehkatra-mcp` — the MCP server over stdio (BOOTSTRAP row 14, docs/21).
//!
//! **This binary is a transport and nothing else.** Every tool, every schema,
//! every refusal lives in `usk-mcp`, which is `no_std` and has no I/O — so all
//! of it is proven by ordinary unit tests rather than by driving an agent. What
//! is left here is: read a line, parse it, hand it over, write a line.
//!
//! Transport is newline-delimited JSON-RPC 2.0 on stdin/stdout, which is what
//! MCP's stdio transport specifies. Two consequences are worth stating because
//! they are easy to get wrong and impossible to notice:
//! * **Nothing but protocol goes to stdout.** Diagnostics go to stderr. A
//!   stray `println!` in a stdio server corrupts the stream and the client
//!   reports a parse error somewhere unrelated.
//! * **A notification gets no reply.** `usk_mcp::Server::handle` returns `None`
//!   for a request without an `id`, and this loop writes nothing — a server
//!   that answers anyway desynchronises a client that is not expecting it.
//!
//! # Scope, and the loopback question
//! docs/21 lists two deployment shapes: a governed server endpoint, and a
//! desktop-local server the running app hosts on loopback. This is neither yet
//! — it is the stdio server a client spawns as a child process, which is the
//! shape BOOTSTRAP row 14 asks for and the one with no listening socket at all
//! (DP-S5). The loopback shape arrives with the app.

use std::io::{self, BufRead, Write};

use usk_json::Json;
use usk_mcp::Server;
use usk_types::ActorId;

/// The actor id agent edits are attributed to.
///
/// Fixed, and deliberately **not** the human's: docs/21 wants every agent edit
/// attributable, and sharing the user's actor id would make an agent's writes
/// indistinguishable from the user's in the op log — which is exactly the
/// attribution the layer exists to provide.
const AGENT_ACTOR: u128 = 0x0A6E_0000_0000_0001;

fn main() {
    let mut server = Server::new(ActorId(AGENT_ACTOR));
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                eprintln!("ehkatra-mcp: stdin: {err}");
                return;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = match usk_json::parse_str(&line) {
            Ok(request) => server.handle(&request),
            // A parse failure has no id to answer against, so JSON-RPC's null-id
            // error response is the only correct reply. Dropping it silently
            // would leave a client waiting forever.
            Err(err) => Some(parse_error(&format!("{err:?}"))),
        };

        let Some(response) = response else {
            continue;
        };
        if writeln!(stdout, "{}", response.to_json_string()).is_err() || stdout.flush().is_err() {
            // The client has gone. There is nobody to report it to.
            return;
        }
    }
}

fn parse_error(detail: &str) -> Json {
    Json::Object(vec![
        (String::from("jsonrpc"), usk_json::string("2.0")),
        (String::from("id"), Json::Null),
        (
            String::from("error"),
            Json::Object(vec![
                (String::from("code"), usk_json::number(-32700.0)),
                (
                    String::from("message"),
                    usk_json::string(format!("parse error: {detail}")),
                ),
            ]),
        ),
    ])
}
