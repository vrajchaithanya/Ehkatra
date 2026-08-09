//! The stdio transport, driven as a real client would drive it.
//!
//! `usk-mcp`'s tests prove the tools. These prove only what the process adds:
//! newline framing, notification silence, stdout purity, and that the whole
//! BOOTSTRAP row 14 loop — describe → read → preview → apply → undo — completes
//! through a pipe.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use usk_json::{parse_str, Json};

fn binary() -> PathBuf {
    let exe = std::env::current_exe().expect("test binary path");
    let dir = exe.parent().expect("deps dir");
    let name = if cfg!(windows) {
        "ehkatra-mcp.exe"
    } else {
        "ehkatra-mcp"
    };
    let beside = dir.join(name);
    if beside.exists() {
        return beside;
    }
    dir.parent().expect("target dir").join(name)
}

/// Feeds a script of requests and returns one parsed response per output line.
fn drive(requests: &[&str]) -> Vec<Json> {
    let mut child = Command::new(binary())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the server starts");

    let script = requests.join("\n") + "\n";
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write");

    let output = child.wait_with_output().expect("the server exits on EOF");
    assert!(
        output.stderr.is_empty(),
        "nothing should reach stderr in a clean run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("stdout is UTF-8")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| parse_str(line).unwrap_or_else(|e| panic!("{line}: {e:?}")))
        .collect()
}

fn call(id: u32, tool: &str, args: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#
    )
}

fn payload(response: &Json) -> &Json {
    response
        .get("result")
        .and_then(|r| r.get("structuredContent"))
        .expect("structuredContent")
}

fn refused(response: &Json) -> bool {
    response
        .get("result")
        .and_then(|r| r.get("isError"))
        .and_then(Json::as_bool)
        .unwrap_or(false)
}

#[test]
fn the_server_speaks_newline_delimited_json_rpc() {
    let responses = drive(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    ]);
    assert_eq!(responses.len(), 2, "one line in, one line out");
    assert_eq!(responses[0].get("id").and_then(Json::as_f64), Some(1.0));
    assert_eq!(
        responses[0].get("jsonrpc").and_then(Json::as_str),
        Some("2.0")
    );
    let tools = responses[1]
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(Json::as_array)
        .expect("tools");
    assert_eq!(tools.len(), usk_mcp::TOOLS.len());
}

/// A notification has no `id` and must produce no output. A server that answers
/// anyway leaves the client one response ahead for the rest of the session.
#[test]
fn a_notification_produces_no_line_at_all() {
    let responses = drive(&[
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":7,"method":"ping","params":{}}"#,
    ]);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].get("id").and_then(Json::as_f64), Some(7.0));
}

/// A line that is not JSON has no id to answer against, so JSON-RPC's null-id
/// error is the only correct reply — and dropping it would leave a client
/// waiting forever.
#[test]
fn unparseable_input_gets_a_null_id_error_and_the_server_keeps_going() {
    let responses = drive(&[
        "this is not json",
        r#"{"jsonrpc":"2.0","id":9,"method":"ping","params":{}}"#,
    ]);
    assert_eq!(responses.len(), 2);
    assert!(responses[0].get("id").is_some_and(Json::is_null));
    assert_eq!(
        responses[0]
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Json::as_f64),
        Some(-32700.0)
    );
    assert_eq!(
        responses[1].get("id").and_then(Json::as_f64),
        Some(9.0),
        "a bad line must not end the session"
    );
}

/// **The loop BOOTSTRAP row 14 asks a client to complete**, through a real
/// pipe: describe → read → preview → apply → undo, on a workbook the agent
/// builds itself.
#[test]
fn an_mcp_client_completes_describe_preview_apply_undo() {
    let responses = drive(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        // Build a small grid: two columns, two rows, then values.
        &call(
            2,
            "apply_edits",
            r#"{"edits":[{"cell":"A1","value":2}],"label":"seed"}"#,
        ),
        &call(3, "describe_workbook", "{}"),
        &call(4, "describe_sheet", "{}"),
        &call(5, "read_range", r#"{"range":"A1:A1"}"#),
        &call(
            6,
            "preview_edits",
            r#"{"edits":[{"cell":"A1","value":40}]}"#,
        ),
        &call(
            7,
            "apply_edits",
            r#"{"edits":[{"cell":"A1","value":40}],"label":"agent edit"}"#,
        ),
        &call(8, "explain_cell", r#"{"cell":"A1"}"#),
        &call(9, "undo", "{}"),
        &call(10, "read_range", r#"{"range":"A1:A1"}"#),
    ]);
    assert_eq!(responses.len(), 10);
    for response in &responses {
        assert!(
            !refused(response),
            "nothing in the loop should refuse: {response:?}"
        );
    }

    // The seed landed, the preview did not mutate, the apply did, and the undo
    // put it back.
    let before = payload(&responses[4]);
    let previewed = payload(&responses[5]);
    assert!(previewed
        .get("preview_hash")
        .and_then(Json::as_str)
        .is_some());

    let explained = payload(&responses[7]);
    assert_eq!(
        explained
            .get("value")
            .and_then(|v| v.get("value"))
            .and_then(Json::as_f64),
        Some(40.0),
        "the applied edit is visible"
    );

    let undone = payload(&responses[8]);
    assert_eq!(undone.get("undone").and_then(Json::as_bool), Some(true));

    let after = payload(&responses[9]);
    assert_eq!(
        after.get("cells"),
        before.get("cells"),
        "the workbook is back where the agent found it"
    );
}

/// A refusal has to survive the transport as a refusal: `isError` set, the
/// session still alive, and the next request answered normally.
#[test]
fn a_refusal_crosses_the_transport_and_the_session_survives_it() {
    let responses = drive(&[
        &call(
            1,
            "apply_edits",
            r#"{"edits":[{"cell":"A1","value":1}],"expected_version":"deadbeef"}"#,
        ),
        &call(2, "describe_workbook", "{}"),
    ]);
    assert!(refused(&responses[0]));
    assert_eq!(
        payload(&responses[0]).get("code").and_then(Json::as_str),
        Some("version_mismatch")
    );
    assert!(!refused(&responses[1]), "the session is still usable");
}
