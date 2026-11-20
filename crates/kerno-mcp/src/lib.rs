//! An MCP server (JSON-RPC 2.0 over stdio, one message per line) that lets an
//! AI agent use a Kerno database as long-term memory. See the crate README.
//!
//! Errors reach the client in one of two ways, as the MCP specification asks:
//! a malformed request or arguments is a JSON-RPC error (`error.code`,
//! `error.message`); a well formed call that fails (no such unit, the engine
//! refused) is a tool result with `isError: true` that the model can read.
//! Nothing here panics on client input (the crate denies `unwrap`/`expect`).

mod tools;

use std::io::{self, BufRead, Write};

use serde::Deserialize;
use serde_json::{Value, json};

pub use tools::{Fail, Memory};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;

struct RpcError {
    code: i32,
    message: String,
}

impl RpcError {
    fn new(code: i32, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into() }
    }
}

fn reply(id: Value, result: Result<Value, RpcError>) -> Value {
    match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": e.code, "message": e.message } }),
    }
}

#[derive(Deserialize)]
struct CallParams {
    name: String,
    arguments: Option<Value>,
}

fn call_tool(memory: &Memory, params: Value) -> Result<Value, RpcError> {
    let p: CallParams = serde_json::from_value(params)
        .map_err(|e| RpcError::new(INVALID_PARAMS, format!("invalid tools/call params: {e}")))?;
    let (text, is_error) = match memory.call(&p.name, p.arguments.unwrap_or(Value::Null)) {
        Ok(v) => (v.to_string(), false),
        Err(Fail::Params(m)) => return Err(RpcError::new(INVALID_PARAMS, m)),
        Err(Fail::Tool(m)) => (m, true),
    };
    Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
}

fn dispatch(memory: &Memory, method: &str, params: Value) -> Result<Value, RpcError> {
    match method {
        "initialize" => Ok(json!({
            // This server speaks one version; the client decides whether it can too.
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "kerno-mcp", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Long-term memory on a Kerno database. `remember` notes, `relate` them, \
                `recall` by relation, type or tag. `forget` is logical deletion only (ADR-0009).",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::tool_list() })),
        "tools/call" => call_tool(memory, params),
        other => Err(RpcError::new(METHOD_NOT_FOUND, format!("method not found: {other}"))),
    }
}

/// The answer to one message, or `None` for a notification (never answered,
/// even when it is wrong: it has no id to answer to).
fn handle(memory: &Memory, msg: Value) -> Option<Value> {
    let Value::Object(obj) = msg else {
        let e = RpcError::new(INVALID_REQUEST, "invalid request: a message must be a JSON object");
        return Some(reply(Value::Null, Err(e)));
    };
    let id = match obj.get("id") {
        None => None,
        Some(id @ (Value::String(_) | Value::Number(_))) => Some(id.clone()),
        Some(_) => {
            let e = RpcError::new(INVALID_REQUEST, "invalid request: id must be a string or a number");
            return Some(reply(Value::Null, Err(e)));
        }
    };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        // A response to a request of ours; this server sends none.
        if obj.contains_key("result") || obj.contains_key("error") {
            return None;
        }
        let e = RpcError::new(INVALID_REQUEST, "invalid request: no `method`");
        return Some(reply(id.unwrap_or(Value::Null), Err(e)));
    };
    // A notification: no id, so no answer, not even to a wrong one.
    let id = id?;
    if obj.get("jsonrpc") != Some(&json!("2.0")) {
        let e = RpcError::new(INVALID_REQUEST, "invalid request: `jsonrpc` must be \"2.0\"");
        return Some(reply(id, Err(e)));
    }
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    Some(reply(id, dispatch(memory, method, params)))
}

/// Reads messages from `input`, one per line, and writes the answers to
/// `output`, one per line, until `input` ends.
///
/// ponytail: a line is read whole, so a client can make the server hold one
/// message of any size; fine for a local stdio peer, cap it if that changes.
pub fn serve(memory: &Memory, mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.trim_ascii().is_empty() {
            continue;
        }
        let answer = match serde_json::from_slice::<Value>(&line) {
            Ok(msg) => handle(memory, msg),
            Err(e) => Some(reply(Value::Null, Err(RpcError::new(PARSE_ERROR, format!("parse error: {e}"))))),
        };
        if let Some(answer) = answer {
            serde_json::to_writer(&mut output, &answer)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}
