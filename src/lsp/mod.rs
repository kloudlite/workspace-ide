pub mod server;

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};

// ponytail: which() + nix fallback, no custom download
fn ensure_binary(binary: &str) -> Result<String, String> {
    if let Ok(out) = std::process::Command::new("which").arg(binary).output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }
    let _ = crate::nix::install_auto(binary);
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

const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

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

pub(crate) fn extension_for(path: &str) -> String {
    let ext = match Path::new(path).extension() {
        Some(e) => format!(".{}", e.to_string_lossy()),
        None => return String::new(),
    };
    if ext == "." && path.ends_with("Dockerfile") {
        "Dockerfile".to_string()
    } else {
        ext
    }
}

/// Open a file in the LSP session (textDocument/didOpen).
async fn send_did_open(stdin: &mut ChildStdin, path: &str, language_id: &str, content: &str) {
    write_msg(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": format!("file://{}", path),
                    "languageId": language_id,
                    "version": 1,
                    "text": content,
                }
            }
        }),
    )
    .await;
}

/// Send a JSON-RPC request to an LSP server and wait for the response.
/// Fresh process per request (no session caching). Uses blocking I/O to match Python-test behavior.
static NEXT_REQ_ID: AtomicU64 = AtomicU64::new(2);

pub async fn lsp_request(
    file_path: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let ext = extension_for(file_path);
    let servers = server::for_extension(&ext);
    if servers.is_empty() {
        return Err(format!("no LSP server for {}", ext));
    }
    let svr = servers.into_iter().next().unwrap().clone();
    let _ = ensure_binary(svr.binary);
    for pkg in svr.nix_packages {
        let _ = crate::nix::install_auto(pkg);
    }
    let bin_path = ensure_binary(svr.binary)?;
    let root = find_root(file_path, svr.needs_lockfile);
    let language_id = svr.language_id;
    let content = tokio::fs::read_to_string(file_path)
        .await
        .unwrap_or_default();
    let id = NEXT_REQ_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let f_path = file_path.to_string();
    let method = method.to_string();
    let params_json = serde_json::to_string(&params).unwrap_or_default();

    // Use std::process::Command (blocking) to avoid tokio pipe bugs
    tokio::task::spawn_blocking(move || {
        let mut child = match std::process::Command::new(&bin_path)
            .args(svr.args)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return Err(format!("spawn: {}", e)),
        };
        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => return Err("no stdin".to_string()),
        };
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => return Err("no stdout".to_string()),
        };

        use std::io::{Write, Read};
        let mut w = |msg: &serde_json::Value| {
            let body = serde_json::to_string(msg).unwrap();
            let header = format!("Content-Length: {}\r\n\r\n", body.len());
            stdin.write_all(header.as_bytes()).ok();
            stdin.write_all(body.as_bytes()).ok();
            stdin.flush().ok();
        };

        // Initialize
        w(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"capabilities":{}}}));
        std::thread::sleep(std::time::Duration::from_millis(50));
        w(&serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
        std::thread::sleep(std::time::Duration::from_millis(500));

        // didOpen
        w(&serde_json::json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":format!("file://{}",f_path),"languageId":language_id,"version":1,"text":content}}
        }));

        // Parse params from JSON string, then build request
        let params_val: serde_json::Value = serde_json::from_str(&params_json).unwrap_or(serde_json::Value::Null);
        w(&serde_json::json!({
            "jsonrpc":"2.0","id":id,"method":method,"params":params_val
        }));

        // Keep stdin open. Read one message at a time from stdout.
        // This gives gopls time to process each request and produce output.
        // ponytail: no background threads, no oneshot channels. One-byte-at-a-time is
        // inefficient for large messages but fine for LSP (hover/def/completion are small).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);

        /// Read one JSON-RPC message (header + body) from a BufRead.
        fn read_next<R: Read>(r: &mut std::io::BufReader<R>) -> Option<serde_json::Value> {
            let mut header = Vec::new();
            let mut cr = 0;
            loop {
                let mut byte = [0u8; 1];
                r.read_exact(&mut byte).ok()?;
                match (cr, byte[0]) {
                    (0, b'\r') => cr = 1,
                    (1, b'\n') => cr = 2,
                    (2, b'\r') => cr = 3,
                    (3, b'\n') => {
                        if let Ok(s) = std::str::from_utf8(&header[..header.len() - 2]) {
                            for line in s.lines() {
                                if let Some(len_str) = line.strip_prefix("Content-Length: ") {
                                    if let Ok(len) = len_str.trim().parse::<usize>() {
                                        let mut body = vec![0u8; len];
                                        r.read_exact(&mut body).ok()?;
                                        return serde_json::from_slice(&body).ok();
                                    }
                                }
                            }
                        }
                        return None;
                    }
                    _ => { cr = 0; }
                }
                header.push(byte[0]);
            }
        }

        let mut reader = std::io::BufReader::new(stdout);

        // Read initialize response
        let _init_resp = read_next(&mut reader).ok_or("no init response")?;

        // Read messages until we find our response
        loop {
            if std::time::Instant::now() >= deadline {
                break;
            }
            match read_next(&mut reader) {
                Some(msg) => {
                    if msg.get("id").and_then(|v| v.as_u64()) == Some(id) {
                        let _ = child.kill();
                        return Ok(msg);
                    }
                }
                None => break,
            }
        }
        let _ = child.kill();
        Err("response not found".to_string())
    })
    .await
    .map_err(|_| "join error".to_string())?
}

// --- Existing functions (diagnose, server lifecycle) ---

pub async fn diagnose_file(file_path: &str) -> Result<Vec<Diagnostic>, String> {
    let ext = extension_for(file_path);
    let servers = server::for_extension(&ext);
    if servers.is_empty() {
        return Ok(vec![]);
    }

    let mut all_diags = Vec::new();
    for svr in servers {
        let sid = svr.id.to_string();
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
        let language_id = svr.language_id;
        let mut owned = session.stdin.lock().unwrap().take();
        if let Some(ref mut stdin) = owned {
            send_did_open(stdin, file_path, language_id, &content).await;
        }
        *session.stdin.lock().unwrap() = owned;
        tokio::time::sleep(Duration::from_millis(500)).await;
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
    cmd.stderr(Stdio::inherit());
    // Debug: check if PATH includes nix profile bin
    eprintln!(
        "ws: start_server PATH={}",
        std::env::var("PATH").unwrap_or_default()
    );
    eprintln!("ws: spawning {} in {} with {}", svr.id, root, bin_path);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {}", svr.id, e))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;

    let diags_arc: Arc<Mutex<Vec<Diagnostic>>> = Arc::new(Mutex::new(Vec::new()));
    let diags = diags_arc.clone();
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut reader = tokio::io::BufReader::new(stdout);
        let mut buf = vec![0u8; 8192];
        let mut data = Vec::new();
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    data.extend_from_slice(&buf[..n]);
                    while let Some((msg, rest)) = parse_jsonrpc(&data) {
                        if msg.get("method").and_then(|v| v.as_str()).is_some() {
                            process_message(&msg, &diags).await;
                        }
                        data = rest.to_vec();
                    }
                }
                Err(_) => break,
            }
        }
    });

    send_initialize(&mut stdin).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    Ok((child, stdin))
}

fn parse_jsonrpc(data: &[u8]) -> Option<(serde_json::Value, &[u8])> {
    let s = std::str::from_utf8(data).ok()?;
    let pos = s.find("\r\n\r\n")?;
    let header = &s[..pos];
    let len: usize = header
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: ")?.trim().parse().ok())?;
    let body_start = pos + 4;
    if data.len() < body_start + len {
        return None;
    }
    let body = &data[body_start..body_start + len];
    let rest = &data[body_start + len..];
    let msg = serde_json::from_slice(body).ok()?;
    Some((msg, rest))
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
    tokio::time::sleep(Duration::from_millis(50)).await;
    write_msg(
        stdin,
        &serde_json::json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;
    Ok(())
}

async fn process_message(msg: &serde_json::Value, diags: &Arc<Mutex<Vec<Diagnostic>>>) {
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

/// Build LSP request params for a position. Shared by server.rs and mcp.rs.
pub fn lsp_params(path: &str, line: u32, col: u32, method: &str) -> serde_json::Value {
    let mut p = serde_json::json!({
        "textDocument": { "uri": format!("file://{}", path) },
        "position": { "line": line, "character": col },
    });
    if method.ends_with("/references") {
        p["context"] = serde_json::json!({ "includeDeclaration": true });
    }
    p
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

/// Scan workspace for files with known LSP extensions, call f for each match.
pub fn walk_files(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') || name == "node_modules" || name == "target" {
                    continue;
                }
            }
            walk_files(&path, f);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        let ext = extension_for(&path_str);
        if ext.is_empty() || server::for_extension(&ext).is_empty() {
            continue;
        }
        f(&path);
    }
}

/// Install missing LSP servers, uninstall unused ones.
/// ponytail: called every ~10min from watch loop.
pub fn reconcile_lsp() -> (usize, usize) {
    use std::collections::HashSet;
    let mut needed = HashSet::new();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    walk_files(&cwd, &mut |p| {
        let ext = extension_for(&p.to_string_lossy());
        for svr in server::for_extension(&ext) {
            needed.insert(svr.binary);
        }
    });
    let mut installed_count = 0;
    let mut uninstalled = 0;
    for svr in server::SERVERS {
        let on_path = std::process::Command::new("which")
            .arg(svr.binary)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if needed.contains(svr.binary) {
            if !on_path {
                eprintln!("ws: installing LSP: {}", svr.binary);
                let _ = crate::nix::install_auto(svr.binary);
                installed_count += 1;
            }
        } else if on_path {
            eprintln!("ws: uninstalling unused LSP: {}", svr.binary);
            let _ = crate::nix::remove(svr.binary);
            uninstalled += 1;
        }
    }
    (installed_count, uninstalled)
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
