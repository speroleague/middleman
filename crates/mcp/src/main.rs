//! `middleman-mcp` binary: newline-delimited JSON-RPC over stdio.

#![allow(
    clippy::map_unwrap_or,
    clippy::needless_pass_by_value,
    clippy::uninlined_format_args
)]

use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    process::Command,
};

fn main() {
    let repo = std::env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .windows(2)
        .find(|pair| pair[0] == "--repo")
        .map(|pair| PathBuf::from(&pair[1]))
        .unwrap_or_else(|| PathBuf::from("."));
    let stdin = io::stdin();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if request.get("id").is_none() {
            continue;
        }
        let id = request["id"].clone();
        let response = match request["method"].as_str() {
            Some("initialize") => result(
                id,
                json!({"protocolVersion":"2025-03-26","serverInfo":{"name":"middleman","version":env!("CARGO_PKG_VERSION")},"capabilities":{"tools":{},"resources":{}}}),
            ),
            Some("tools/list") => result(
                id,
                json!({"tools":[
                    tool("middleman_prepare", "Return a bounded, relevant task context packet.", json!({"type":"object","required":["task"],"properties":{"task":{"type":"string"},"format":{"enum":["cir","markdown","json"]},"budget":{"type":"integer","minimum":1}}})),
                    tool("middleman_expand", "Expand one indexed node and its direct neighborhood.", json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"},"format":{"enum":["cir","markdown","json"]},"budget":{"type":"integer","minimum":1}}})),
                    tool("middleman_propose", "Create a reviewed durable-memory proposal from structured claims.", json!({"type":"object","required":["task_id","source"],"properties":{"task_id":{"type":"string"},"source":{"type":"object"}}}))
                ]}),
            ),
            Some("resources/list") => result(
                id,
                json!({"resources":[{"uri":"middleman://project/current","name":"Current project"}]}),
            ),
            Some("resources/templates/list") => result(
                id,
                json!({"resourceTemplates":[{"uriTemplate":"middleman://task/{id}","name":"Task"},{"uriTemplate":"middleman://node/{id}","name":"Indexed node"}]}),
            ),
            Some("resources/read") => cli_resource(id, &repo, &request["params"]),
            Some("tools/call") => cli_tool(id, &repo, &request["params"]),
            _ => error(id, -32601, "method not found"),
        };
        println!("{}", response);
        let _ = io::stdout().flush();
    }
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({"name":name,"description":description,"inputSchema":input_schema})
}
fn result(id: Value, value: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":value})
}
fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn cli_resource(id: Value, repo: &PathBuf, params: &Value) -> Value {
    let uri = params["uri"].as_str().unwrap_or("");
    let command = if uri == "middleman://project/current" {
        vec!["status"]
    } else if let Some(task) = uri.strip_prefix("middleman://task/") {
        vec!["task", "show", task]
    } else if let Some(node) = uri.strip_prefix("middleman://node/") {
        vec!["explain", node, "--format", "json"]
    } else {
        return error(id, -32602, "unknown resource");
    };
    match invoke(repo, &command) {
        Ok(text) => result(
            id,
            json!({"contents":[{"uri":uri,"mimeType":"text/plain","text":text}]}),
        ),
        Err(message) => error(id, -32000, &message),
    }
}

fn cli_tool(id: Value, repo: &PathBuf, params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];
    if name == "middleman_propose" {
        let Some(task_id) = args["task_id"].as_str() else {
            return tool_error(id, "Tool arguments are invalid.");
        };
        let state = repo.join(".middleman");
        let Ok(input) = tempfile::NamedTempFile::new_in(state) else {
            return tool_error(id, "Middleman state is unavailable; use the CLI fallback.");
        };
        let Ok(mut file) = input.reopen() else {
            return tool_error(id, "Middleman state is unavailable; use the CLI fallback.");
        };
        if serde_json::to_writer(&mut file, &args["source"]).is_err() {
            return tool_error(id, "Tool arguments are invalid.");
        }
        if file.sync_all().is_err() {
            return tool_error(id, "Middleman state is unavailable; use the CLI fallback.");
        }
        drop(file);
        let path = input.path().to_string_lossy().into_owned();
        let mut command = vec!["propose", "--task-id", task_id, "--input", &path];
        if args["from_git"].as_bool() == Some(true) {
            command.push("--from-git");
        }
        return tool_result(id, invoke(repo, &command));
    }
    let mut command = match name {
        "middleman_prepare" => vec!["prepare", "--task", args["task"].as_str().unwrap_or("")],
        "middleman_expand" => vec!["expand", args["id"].as_str().unwrap_or("")],
        _ => return tool_error(id, "Tool arguments are invalid."),
    };
    if let Some(format) = args["format"].as_str() {
        command.extend(["--format", format]);
    }
    let budget = args["budget"].as_u64().map(|value| value.to_string());
    if let Some(value) = budget.as_deref() {
        command.extend(["--budget", value]);
    }
    tool_result(id, invoke(repo, &command))
}

fn tool_result(id: Value, output: Result<String, String>) -> Value {
    match output {
        Ok(text) => result(id, json!({"content":[{"type":"text","text":text}]})),
        Err(message) => result(
            id,
            json!({"content":[{"type":"text","text":message}],"isError":true}),
        ),
    }
}

fn tool_error(id: Value, message: &str) -> Value {
    result(
        id,
        json!({"content":[{"type":"text","text":message}],"isError":true}),
    )
}

fn invoke(repo: &PathBuf, args: &[&str]) -> Result<String, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "Middleman CLI is unavailable".to_owned())?
        .with_file_name(if cfg!(windows) {
            "middleman.exe"
        } else {
            "middleman"
        });
    let output = Command::new(executable)
        .arg("--repo")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|_| "Middleman CLI is unavailable".to_owned())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}
