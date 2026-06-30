pub mod server;

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

// ponytail: ensure binary via which() + nix fallback, no custom download logic
fn ensure_binary(binary: &str) -> Result<String, String> {
    if let Ok(out) = std::process::Command::new("which").arg(binary).output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }
    let _ = crate::nix::install(binary);
    if let Ok(out) = std::process::Command::new("which").arg(binary).output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }
    Err(format!("{} not found", binary))
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub severity: u8,
    pub message: String,
    pub code: Option<String>,
}

const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone)]
struct Session {
    server_id: String,
    root: String,
    process: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    diags: Arc<Mutex<Vec<Diagnostic>>>,
    last_active: Instant,
}

struct LspState {
    sessions: Vec<Session>,
}

fn state() -> &'static Mutex<LspState> {
    static STATE: OnceLock<Mutex<LspState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(LspState {
            sessions: Vec::new(),
        })
    })
}

pub async fn diagnose_file(file_path: &str) -> Result<Vec<Diagnostic>, String> {
    let ext = match Path::new(file_path).extension() {
        Some(e) => format!(".{}", e.to_string_lossy()),
        None => return Ok(vec![]),
    };
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
        let sid = svr.id.to_string();
        // Ensure binary is available
        let _ = ensure_binary(svr.binary);
        let root = find_root(file_path, svr.needs_lockfile);

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
                let (child, stdin) = start_server(svr, &root).await?;
                let session = Session {
                    server_id: sid.clone(),
                    root: root.clone(),
                    process: Arc::new(Mutex::new(Some(child))),
                    stdin: Arc::new(Mutex::new(Some(stdin))),
                    diags: Arc::new(Mutex::new(Vec::new())),
                    last_active: Instant::now(),
                };
                let cloned = session.clone();
                state().lock().unwrap().sessions.push(session);
                cloned
            }
        };

        let content = tokio::fs::read_to_string(file_path)
            .await
            .unwrap_or_default();
        let language_id = server::language_for_ext(&ext);
        let mut owned = session.stdin.lock().unwrap().take();
        if let Some(ref mut stdin) = owned {
            send_did_open(stdin, file_path, language_id, &content).await;
        }
        *session.stdin.lock().unwrap() = owned;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        all_diags.extend(session.diags.lock().unwrap().clone());
    }
    Ok(all_diags)
}

async fn start_server(svr: &server::LspServer, root: &str) -> Result<(Child, ChildStdin), String> {
    let bin_path = ensure_binary(svr.binary)?;
    let mut cmd = Command::new(&bin_path);
    cmd.args(svr.args);
    cmd.current_dir(root);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {}", svr.id, e))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;

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
                        process_message(&sid, &msg, &diags).await;
                    }
                }
                Err(_) => break,
            }
        }
    });

    send_initialize(&mut stdin).await?;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    Ok((child, stdin))
}

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
    let (start, len) = extract_content_length(data).ok_or(())?;
    let body = &data[start..start + len];
    let rest = &data[start + len..];
    let msg: serde_json::Value = serde_json::from_slice(body).map_err(|_| ())?;
    Ok((msg, rest))
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
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
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
    write_msg(
        stdin,
        &serde_json::json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;
    Ok(())
}

async fn send_did_open(stdin: &mut ChildStdin, path: &str, language_id: &str, content: &str) {
    write_msg(stdin, &serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": format!("file://{}", path), "languageId": language_id, "version": 1, "text": content } }
    })).await;
}

async fn process_message(_sid: &str, msg: &serde_json::Value, diags: &Arc<Mutex<Vec<Diagnostic>>>) {
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
                diags.lock().unwrap().extend(collected);
            }
        }
    }
}

fn find_root(file_path: &str, needs_lockfile: bool) -> String {
    let path = Path::new(file_path);
    let dir = path.parent().unwrap_or(Path::new("."));
    if !needs_lockfile {
        return dir.to_string_lossy().to_string();
    }
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

pub fn list_sessions() -> Vec<(String, String)> {
    state()
        .lock()
        .unwrap()
        .sessions
        .iter()
        .map(|s| (s.server_id.clone(), s.root.clone()))
        .collect()
}

pub fn cleanup_idle() -> usize {
    let mut s = state().lock().unwrap();
    let now = Instant::now();
    let mut alive = Vec::new();
    let mut killed = 0;
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
