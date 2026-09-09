#![allow(clippy::unwrap_used)]
use std::{
    io::Write,
    process::{Command, Stdio},
};

use serde_json::Value;

#[test]
fn stdio_initialization_and_discovery_are_line_delimited_json_rpc() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_middleman-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    writeln!(
        stdin,
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{}}}}"
    )
    .unwrap();
    writeln!(
        stdin,
        "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{{}}}}"
    )
    .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(lines[0]["result"]["protocolVersion"], "2025-03-26");
    let tools = lines[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3);
    assert_eq!(tools[0]["name"], "middleman_prepare");
    assert_eq!(tools[1]["name"], "middleman_expand");
    assert_eq!(tools[2]["name"], "middleman_propose");
}
