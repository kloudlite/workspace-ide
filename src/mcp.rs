// Model Context Protocol — JSON-RPC 2.0
// Supports stdio transport (ws mcp) and HTTP transport (POST /mcp on ws serve)
// ponytail: tools-only MCP; no resources, prompts, or sampling
// spec: https://spec.modelcontextprotocol.io

use crate::tools;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

/// Process one MCP JSON-RPC request, return the response value.
/// For notifications (no `id`), returns None.
pub async fn handle_request(req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");

    match method {
        "initialize" => Some(json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "ws", "version": "0.1.0" }
            }
        })),
        "notifications/initialized" => None,
        "tools/list" => Some(json!({
            "jsonrpc": "2.0", "id": id,
            "result": { "tools": tool_definitions() }
        })),
        "tools/call" => {
            let name = req
                .get("params")
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = req
                .get("params")
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(json!({}));

            match dispatch_tool(name, &args).await {
                Ok(content) => Some(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": serde_json::to_string(&content).unwrap_or_default() }]
                    }
                })),
                Err(e) => Some(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": format!("Error: {}", e) }],
                        "isError": true
                    }
                })),
            }
        }
        _ => Some(json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32601, "message": format!("Method not found: {}", method) }
        })),
    }
}

/// Stdio transport loop — for AI agents that spawn ws mcp as a subprocess.
pub async fn run() {
    use tokio::io::AsyncBufReadExt;
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut line = String::new();

    loop {
        line.clear();
        let n = stdin.read_line(&mut line).await.unwrap_or(0);
        if n == 0 {
            break;
        }

        let req: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({
                    "jsonrpc": "2.0", "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                });
                let _ = writeln_json(&err).await;
                continue;
            }
        };

        if let Some(resp) = handle_request(&req).await {
            let _ = writeln_json(&resp).await;
        }
    }
}

async fn writeln_json(val: &Value) -> Result<(), std::io::Error> {
    let mut s = serde_json::to_string(val).unwrap();
    s.push('\n');
    tokio::io::stdout().write_all(s.as_bytes()).await?;
    tokio::io::stdout().flush().await
}

// --- Tool definitions & dispatch (shared by stdio and HTTP) ---

fn tool(name: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": desc,
        "inputSchema": {
            "type": "object",
            "properties": props,
            "required": required,
        }
    })
}

fn tool_noprops(name: &str, desc: &str) -> Value {
    tool(name, desc, json!({}), &[])
}

fn tool_str(name: &str, desc: &str, field: &str) -> Value {
    let props = json!({ field: { "type": "string", "description": desc } });
    tool(name, desc, props, &[field])
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "read",
            "Read file contents with optional 1-indexed line range",
            json!({
                "path": { "type": "string", "description": "File path" },
                "offset": { "type": "number", "description": "First line, 1-indexed" },
                "limit": { "type": "number", "description": "Maximum lines" },
            }),
            &["path"],
        ),
        tool(
            "bash",
            "Execute a shell command",
            json!({
                "command": { "type": "string", "description": "Shell command to run" },
                "timeout_secs": { "type": "number", "description": "Optional timeout in seconds" },
            }),
            &["command"],
        ),
        tool(
            "edit",
            "Edit a file with text replacements",
            json!({
                "path": { "type": "string", "description": "File path" },
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_text": { "type": "string" },
                            "new_text": { "type": "string" },
                        },
                        "required": ["old_text", "new_text"],
                    },
                    "description": "List of text replacements",
                },
            }),
            &["path", "edits"],
        ),
        tool(
            "write",
            "Write content to a file",
            json!({
                "path": { "type": "string", "description": "File path" },
                "content": { "type": "string", "description": "File content" },
            }),
            &["path", "content"],
        ),
        tool(
            "grep",
            "Search for a pattern in files",
            json!({
                "pattern": { "type": "string", "description": "Search pattern" },
                "path": { "type": "string", "description": "Directory to search (default: .)" },
            }),
            &["pattern"],
        ),
        tool(
            "find",
            "Find files matching a pattern",
            json!({
                "path": { "type": "string", "description": "Directory to search" },
                "name": { "type": "string", "description": "Glob pattern for filename" },
            }),
            &["path"],
        ),
        tool_str("ls", "List directory contents", "path"),
        tool_str(
            "spawn",
            "Start a long-running background command",
            "command",
        ),
        tool_str(
            "logs",
            "Read stdout/stderr from a background session",
            "session_id",
        ),
        tool_str(
            "status",
            "Check if a background session is still running",
            "session_id",
        ),
        tool_str("kill", "Stop a background session", "session_id"),
        tool_noprops("sessions", "List all background sessions"),
        tool_str("diagnose", "Run LSP diagnostics on a file", "path"),
        tool(
            "lsp_request",
            "Query code intelligence through LSP: navigation, symbols, rename/code-action previews, completion, signatures, or formatting edits",
            json!({
                "method": { "type": "string", "description": "LSP method" },
                "path": { "type": "string", "description": "File path used to select the language server" },
                "line": { "type": "number", "description": "Start line (0-indexed)" },
                "column": { "type": "number", "description": "Start column (0-indexed)" },
                "end_line": { "type": "number", "description": "Optional end line for code actions" },
                "end_column": { "type": "number", "description": "Optional end column for code actions" },
                "query": { "type": "string", "description": "Workspace symbol query" },
                "new_name": { "type": "string", "description": "New symbol name for rename preview" },
            }),
            &["method", "path"],
        ),
        tool_noprops("lsp_sessions", "List running LSP server sessions"),
    ]
}

async fn dispatch_tool(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "read" => {
            let path = get_str(args, "path")?;
            let offset = args
                .get("offset")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            tools::read_file(path, offset, limit)
                .await
                .map(|r| json!(r))
                .map_err(|e| e.0)
        }
        "bash" => {
            let command = get_str(args, "command")?;
            let timeout = args.get("timeout_secs").and_then(|v| v.as_u64());
            Ok(json!(tools::run_bash(command, timeout).await))
        }
        "edit" => {
            let path = get_str(args, "path")?;
            let edits: Vec<tools::EditOp> =
                serde_json::from_value(args.get("edits").ok_or("missing field: edits")?.clone())
                    .map_err(|e| format!("invalid edits: {}", e))?;
            tools::edit_file(path, &edits)
                .await
                .map(|r| json!(r))
                .map_err(|e| e.0)
        }
        "write" => {
            let path = get_str(args, "path")?;
            let content = get_str(args, "content")?;
            tools::write_file(path, content)
                .await
                .map(|r| json!(r))
                .map_err(|e| e.0)
        }
        "grep" => {
            let pattern = get_str(args, "pattern")?;
            let path = args.get("path").and_then(|v| v.as_str());
            tools::grep_files(pattern, path)
                .await
                .map(|r| json!(r))
                .map_err(|e| e.0)
        }
        "find" => {
            let path = get_str(args, "path")?;
            let name = args.get("name").and_then(|v| v.as_str());
            tools::find_files(path, name)
                .await
                .map(|r| json!(r))
                .map_err(|e| e.0)
        }
        "ls" => {
            let path = get_str(args, "path")?;
            tools::list_dir(path)
                .await
                .map(|r| json!(r))
                .map_err(|e| e.0)
        }
        "spawn" => {
            let command = get_str(args, "command")?;
            tools::spawn_bash(command)
                .await
                .map(|r| json!(r))
                .map_err(|e| e.0)
        }
        "logs" => {
            let session_id = get_str(args, "session_id")?;
            tools::get_logs(session_id)
                .await
                .map(|r| json!(r))
                .map_err(|e| e.0)
        }
        "status" => {
            let session_id = get_str(args, "session_id")?;
            tools::get_status(session_id)
                .await
                .map(|r| json!(r))
                .map_err(|e| e.0)
        }
        "kill" => {
            let session_id = get_str(args, "session_id")?;
            tools::kill_session(session_id)
                .await
                .map(|r| json!(r))
                .map_err(|e| e.0)
        }
        "sessions" => Ok(json!(tools::list_sessions().await)),
        "diagnose" => {
            let path = get_str(args, "path")?;
            Ok(json!(crate::lsp::diagnose_file(path).await?))
        }
        "lsp_request" => {
            let method = get_str(args, "method")?;
            let path = get_str(args, "path")?;
            let params = crate::lsp::lsp_params(path, method, args)?;
            crate::lsp::lsp_request(path, method, params)
                .await
                .map(|v| json!(v))
        }
        "lsp_sessions" => Ok(json!(crate::lsp::list_sessions())),
        _ => Err(format!("Unknown tool: {}", name)),
    }
}

fn get_str<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing field: {}", key))
}
