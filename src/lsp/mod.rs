pub mod download;
pub mod server;

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

// --- LSP lifecycle state ---

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub severity: u8, // 1=error 2=warn 3=info 4=hint
    pub message: String,
    pub code: Option<String>,
}

// ponytail: idle cleanup after 60s of no file activity
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone)]
struct Session {
    server_id: String,
    root: String,
    process: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    diags: Arc<Mutex<Vec<Diagnostic>>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
    last_active: Instant,
}

struct LspState {
    sessions: Vec<Session>,
    /// Track which servers we've installed/tried
    installed: HashMap<String, bool>,
}

fn state() -> &'static Mutex<LspState> {
    static STATE: OnceLock<Mutex<LspState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(LspState {
            sessions: Vec::new(),
            installed: HashMap::new(),
        })
    })
}

// --- Public API ---

/// Ensure a server is downloaded, start it, open a file, collect diagnostics.
pub async fn diagnose_file(file_path: &str) -> Result<Vec<Diagnostic>, String> {
    let ext = match Path::new(file_path).extension() {
        Some(e) => format!(".{}", e.to_string_lossy()),
        None => return Ok(vec![]),
    };

    // Handle Dockerfile (no extension)
    let ext = if ext == "." && file_path.ends_with("Dockerfile") {
        "Dockerfile".to_string()
    } else {
        ext
    };

    let servers = server::for_extension(&ext);
    if servers.is_empty() {
        return Ok(vec![]);
    }

    let mut all_diags = Vec::new();

    for svr in servers {
        // Install if needed
        let sid = svr.id.to_string();
        {
            let mut s = state().lock().unwrap();
            if !s.installed.contains_key(&sid) {
                let ok = download::ensure(&svr.install).is_ok();
                s.installed.insert(sid.clone(), ok);
            }
            if !s.installed.get(&sid).copied().unwrap_or(false) {
                continue;
            }
        }

        // Find project root
        let root = find_root(file_path, svr.needs_lockfile);

        // Check if we have a running session for this server+root
        let session = {
            let mut s = state().lock().unwrap();
            if let Some(existing) = s
                .sessions
                .iter_mut()
                .find(|sess| sess.server_id == sid && sess.root == root)
            {
                existing.last_active = Instant::now();
                Some(existing.clone())
            } else {
                None
            }
        };

        let session = match session {
            Some(s) => s,
            None => {
                // Start new session
                let (child, stdin, pending) = start_server(svr, &root).await?;
                let session = Session {
                    server_id: sid.clone(),
                    root: root.clone(),
                    process: Arc::new(Mutex::new(Some(child))),
                    stdin: Arc::new(Mutex::new(Some(stdin))),
                    diags: Arc::new(Mutex::new(Vec::new())),
                    pending: pending.clone(),
                    last_active: Instant::now(),
                };
                let cloned = session.clone();
                let mut s = state().lock().unwrap();
                s.sessions.push(session);
                cloned
            }
        };

        // Send didOpen
        let content = tokio::fs::read_to_string(file_path)
            .await
            .unwrap_or_default();
        let language_id = server::language_for_ext(&ext);
        // ponytail: take stdin, send, put back — avoids holding MutexGuard across await
        let mut owned = session.stdin.lock().unwrap().take();
        if let Some(ref mut stdin) = owned {
            send_did_open(stdin, file_path, language_id, &content).await;
        }
        *session.stdin.lock().unwrap() = owned;

        // Read diagnostics (simplified: wait a bit for push diagnostics or request them)
        // ponytail: simple approach — wait 500ms and read push diagnostics
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let diags = {
            let d = session.diags.lock().unwrap();
            d.clone()
        };
        all_diags.extend(diags);
    }

    Ok(all_diags)
}

// --- Server lifecycle ---

async fn start_server(
    svr: &server::LspServer,
    root: &str,
) -> Result<
    (
        Child,
        ChildStdin,
        Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
    ),
    String,
> {
    let bin_path = download::ensure(&svr.install)?;

    let mut cmd = Command::new(&bin_path);
    cmd.args(svr.args);
    cmd.current_dir(root);
    cmd.env("LC_ALL", "en_US.UTF-8");
    for (k, v) in svr.env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {}", svr.id, e))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;

    // Shared pending requests map
    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let pending_task = pending.clone();
    // Spawn a task to read LSP responses
    let diags_arc: Arc<Mutex<Vec<Diagnostic>>> = Arc::new(Mutex::new(Vec::new()));
    let diags = diags_arc.clone();
    let sid = svr.id.to_string();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::new();
        let mut partial = Vec::new();

        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) => break,
                Ok(_) => {
                    partial.extend_from_slice(&buf);
                    while let Ok((msg, rest)) = parse_jsonrpc(&partial) {
                        partial = rest.to_vec();
                        // Check if this is a response to a pending request
                        if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
                            let mut p = pending_task.lock().unwrap();
                            if let Some(sender) = p.remove(&id) {
                                let _ = sender.send(msg.clone());
                            }
                        }
                        process_message(&sid, &msg, &diags).await;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Send initialize request
    send_initialize(&mut stdin).await?;

    // Wait briefly for capabilities response
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    Ok((child, stdin, pending))
}

// --- LSP Protocol helpers ---

fn extract_content_length(data: &[u8]) -> Option<(usize, usize)> {
    let s = std::str::from_utf8(data).ok()?;
    if let Some(pos) = s.find("\r\n\r\n") {
        let header = &s[..pos];
        for line in header.lines() {
            if let Some(len_str) = line.strip_prefix("Content-Length: ") {
                let len: usize = len_str.trim().parse().ok()?;
                let body_start = pos + 4;
                if data.len() >= body_start + len {
                    return Some((body_start, len));
                }
            }
        }
    }
    None
}

fn parse_jsonrpc(data: &[u8]) -> Result<(serde_json::Value, &[u8]), ()> {
    if let Some((start, len)) = extract_content_length(data) {
        let body = &data[start..start + len];
        let rest = &data[start + len..];
        let msg: serde_json::Value = serde_json::from_slice(body).map_err(|_| ())?;
        Ok((msg, rest))
    } else {
        Err(())
    }
}

async fn write_msg(stdin: &mut ChildStdin, msg: &serde_json::Value) {
    let body = serde_json::to_string(msg).unwrap();
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let _ = stdin.write_all(header.as_bytes()).await;
    let _ = stdin.write_all(body.as_bytes()).await;
    let _ = stdin.flush().await;
}

async fn send_initialize(stdin: &mut ChildStdin) -> Result<(), String> {
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": std::process::id(),
            "clientInfo": { "name": "ws", "version": "0.1.0" },
            "capabilities": {
                "textDocument": {
                    "synchronization": { "didOpen": true, "didChange": true },
                    "diagnostic": { "dynamicRegistration": false }
                },
                "workspace": { "diagnostics": true }
            }
        }
    });
    write_msg(stdin, &msg).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let notified = serde_json::json!({
        "jsonrpc": "2.0", "method": "initialized", "params": {}
    });
    write_msg(stdin, &notified).await;
    Ok(())
}

async fn send_did_open(stdin: &mut ChildStdin, path: &str, language_id: &str, content: &str) {
    let uri = format!("file://{}", path);
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": 1,
                "text": content
            }
        }
    });
    write_msg(stdin, &msg).await;
}

async fn process_message(_sid: &str, msg: &serde_json::Value, diags: &Arc<Mutex<Vec<Diagnostic>>>) {
    // Handle textDocument/publishDiagnostics
    if let Some(params) = msg.get("params") {
        if let Some(uri) = params.get("uri").and_then(|v| v.as_str()) {
            if let Some(diagnostics) = params.get("diagnostics").and_then(|v| v.as_array()) {
                let path = uri.strip_prefix("file://").unwrap_or(uri).to_string();
                let mut collected = Vec::new();
                for d in diagnostics {
                    if let Some(range) = d.get("range").and_then(|r| r.get("start")) {
                        let line = range.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let col =
                            range.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let severity =
                            d.get("severity").and_then(|v| v.as_u64()).unwrap_or(1) as u8;
                        let message = d
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let code = d
                            .get("code")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        collected.push(Diagnostic {
                            path: path.clone(),
                            line,
                            column: col,
                            severity,
                            message,
                            code,
                        });
                    }
                }
                let mut d = diags.lock().unwrap();
                d.extend(collected);
            }
        }
    }
}

// --- Root detection ---

fn find_root(file_path: &str, needs_lockfile: bool) -> String {
    let path = Path::new(file_path);
    let dir = path.parent().unwrap_or(Path::new("."));

    if !needs_lockfile {
        // Use the nearest parent with a recognizable project marker
        // or the file's directory
        return dir.to_string_lossy().to_string();
    }

    // Walk up to find lockfile
    let lockfiles = [
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "bun.lockb",
        "bun.lock",
        "Cargo.lock",
    ];
    let mut current = Some(dir);
    while let Some(d) = current {
        for lf in &lockfiles {
            if d.join(lf).exists() {
                return d.to_string_lossy().to_string();
            }
        }
        current = d.parent();
    }
    dir.to_string_lossy().to_string()
}

// --- Status & management ---

/// Get a list of running LSP sessions
pub fn list_sessions() -> Vec<(String, String)> {
    let s = state().lock().unwrap();
    s.sessions
        .iter()
        .map(|sess| (sess.server_id.clone(), sess.root.clone()))
        .collect()
}

// --- LSP Request-Response API ---

async fn send_request(
    stdin: &mut ChildStdin,
    pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id: u64 = rand_id();
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });

    let (tx, rx) = oneshot::channel();
    {
        let mut p = pending.lock().unwrap();
        p.insert(id, tx);
    }

    write_msg(stdin, &msg).await;

    match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
        Ok(Ok(resp)) => {
            if let Some(error) = resp.get("error") {
                Err(format!("LSP error: {:?}", error))
            } else {
                Ok(resp
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null))
            }
        }
        Ok(Err(_)) => Err("response channel closed".into()),
        Err(_) => Err("LSP request timed out".into()),
    }
}

fn rand_id() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

fn get_session_for(sid: &str, root: &str) -> Option<Session> {
    let s = state().lock().unwrap();
    s.sessions
        .iter()
        .find(|sess| sess.server_id == sid && sess.root == root)
        .cloned()
}

fn update_session_active(sid: &str, root: &str) {
    if let Ok(mut s) = state().lock() {
        if let Some(sess) = s
            .sessions
            .iter_mut()
            .find(|s| s.server_id == sid && s.root == root)
        {
            sess.last_active = Instant::now();
        }
    }
}

async fn ensure_file_open(session: &Session, path: &str) {
    let content = tokio::fs::read_to_string(path).await.unwrap_or_default();
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let language_id = server::language_for_ext(&format!(".{}", ext));
    let mut owned = session.stdin.lock().unwrap().take();
    if let Some(ref mut stdin) = owned {
        send_did_open(stdin, path, language_id, &content).await;
    }
    *session.stdin.lock().unwrap() = owned;
}

macro_rules! lsp_request {
    ($session:expr, $method:expr, $path:expr, $params:expr) => {{
        let mut owned = $session.stdin.lock().unwrap().take();
        let result = match owned {
            Some(ref mut stdin) => send_request(stdin, &$session.pending, $method, $params).await,
            None => Err("no stdin available".into()),
        };
        *$session.stdin.lock().unwrap() = owned;
        result
    }};
}

pub async fn hover(
    file_path: &str,
    line: usize,
    character: usize,
) -> Result<serde_json::Value, String> {
    let (session, uri) = resolve_session(file_path).await?;
    ensure_file_open(&session, file_path).await;
    let params = serde_json::json!({
        "textDocument": { "uri": uri },
        "position": { "line": line, "character": character },
    });
    lsp_request!(session, "textDocument/hover", file_path, params)
}

pub async fn definition(
    file_path: &str,
    line: usize,
    character: usize,
) -> Result<serde_json::Value, String> {
    let (session, uri) = resolve_session(file_path).await?;
    ensure_file_open(&session, file_path).await;
    let params = serde_json::json!({
        "textDocument": { "uri": uri },
        "position": { "line": line, "character": character },
    });
    lsp_request!(session, "textDocument/definition", file_path, params)
}

pub async fn references(
    file_path: &str,
    line: usize,
    character: usize,
) -> Result<serde_json::Value, String> {
    let (session, uri) = resolve_session(file_path).await?;
    ensure_file_open(&session, file_path).await;
    let params = serde_json::json!({
        "textDocument": { "uri": uri },
        "position": { "line": line, "character": character },
        "context": { "includeDeclaration": true },
    });
    lsp_request!(session, "textDocument/references", file_path, params)
}

pub async fn completion(
    file_path: &str,
    line: usize,
    character: usize,
) -> Result<serde_json::Value, String> {
    let (session, uri) = resolve_session(file_path).await?;
    ensure_file_open(&session, file_path).await;
    let params = serde_json::json!({
        "textDocument": { "uri": uri },
        "position": { "line": line, "character": character },
    });
    lsp_request!(session, "textDocument/completion", file_path, params)
}

async fn resolve_session(file_path: &str) -> Result<(Session, String), String> {
    let ext = Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();
    let servers = server::for_extension(&ext);
    if servers.is_empty() {
        return Err(format!("no LSP server for {}", ext));
    }
    let svr = servers[0];
    let root = find_root(file_path, svr.needs_lockfile);
    match get_session_for(svr.id, &root) {
        Some(s) => {
            update_session_active(svr.id, &root);
            Ok((s, format!("file://{}", file_path)))
        }
        None => {
            diagnose_file(file_path).await?;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            match get_session_for(svr.id, &root) {
                Some(s) => {
                    update_session_active(svr.id, &root);
                    Ok((s, format!("file://{}", file_path)))
                }
                None => Err("failed to start LSP session".into()),
            }
        }
    }
}

/// Kill sessions idle longer than IDLE_TIMEOUT
pub fn cleanup_idle() -> usize {
    let mut s = state().lock().unwrap();
    let now = Instant::now();
    let mut alive = Vec::new();
    let mut killed = 0usize;
    for sess in s.sessions.drain(..) {
        if now.duration_since(sess.last_active) > IDLE_TIMEOUT {
            if let Some(child) = sess.process.lock().unwrap().take() {
                let _ = std::process::Command::new("kill")
                    .arg(child.id().unwrap_or(0).to_string())
                    .status();
            }
            killed += 1;
        } else {
            alive.push(sess);
        }
    }
    s.sessions = alive;
    killed
}
