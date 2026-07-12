pub mod server;

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

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

#[derive(Clone)]
struct Session {
    server_id: String,
    root: String,
    _process: Arc<Mutex<Option<Child>>>,
    stdin: Arc<AsyncMutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
    diags: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
    versions: Arc<Mutex<HashMap<String, i32>>>,
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

fn absolute_path(path: &str) -> std::path::PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| Path::new(".").to_path_buf())
            .join(path)
    }
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

fn language_id<'a>(path: &str, default: &'a str) -> &'a str {
    match extension_for(path).as_str() {
        ".tsx" => "typescriptreact",
        ".jsx" => "javascriptreact",
        _ => default,
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

async fn send_did_change(stdin: &mut ChildStdin, path: &str, version: i32, content: &str) {
    write_msg(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": format!("file://{}", path), "version": version },
                "contentChanges": [{ "text": content }],
            }
        }),
    )
    .await;
}

static NEXT_REQ_ID: AtomicU64 = AtomicU64::new(2);

async fn get_session(svr: &server::LspServer, file_path: &str) -> Result<Session, String> {
    let sid = svr.id.to_string();
    let root = find_root(file_path, svr.root_mode);
    if let Some(session) = state()
        .lock()
        .unwrap()
        .sessions
        .iter_mut()
        .find(|session| session.server_id == sid && session.root == root)
    {
        session.last_active = Instant::now();
        return Ok(session.clone());
    }

    let (child, stdin, pending, diags) = start_server(svr, &root).await?;
    let session = Session {
        server_id: sid,
        root,
        _process: Arc::new(Mutex::new(Some(child))),
        stdin: Arc::new(AsyncMutex::new(stdin)),
        pending,
        diags,
        versions: Arc::new(Mutex::new(HashMap::new())),
        last_active: Instant::now(),
    };
    state().lock().unwrap().sessions.push(session.clone());
    Ok(session)
}

async fn sync_document(
    session: &Session,
    svr: &server::LspServer,
    file_path: &str,
) -> Result<(), String> {
    let content = tokio::fs::read_to_string(file_path)
        .await
        .map_err(|e| format!("read {}: {}", file_path, e))?;
    let version = {
        let mut versions = session.versions.lock().unwrap();
        let version = versions.entry(file_path.to_string()).or_insert(0);
        *version += 1;
        *version
    };
    let mut stdin = session.stdin.lock().await;
    if version == 1 {
        send_did_open(
            &mut stdin,
            file_path,
            language_id(file_path, svr.language_id),
            &content,
        )
        .await;
    } else {
        send_did_change(&mut stdin, file_path, version, &content).await;
    }
    Ok(())
}

pub async fn lsp_request(
    file_path: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let file_path = absolute_path(file_path).to_string_lossy().to_string();
    let ext = extension_for(&file_path);
    let svr = server::for_extension(&ext)
        .into_iter()
        .next()
        .ok_or_else(|| format!("no LSP server for {}", ext))?;
    let session = get_session(svr, &file_path).await?;
    sync_document(&session, svr, &file_path).await?;

    let id = NEXT_REQ_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    session.pending.lock().unwrap().insert(id, tx);
    {
        let mut stdin = session.stdin.lock().await;
        write_msg(
            &mut stdin,
            &serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await;
    }

    match tokio::time::timeout(Duration::from_secs(300), rx).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(_)) => Err("LSP response channel closed".to_string()),
        Err(_) => {
            session.pending.lock().unwrap().remove(&id);
            Err(format!("timed out waiting for {}", method))
        }
    }
}

// --- Existing functions (diagnose, server lifecycle) ---

pub async fn diagnose_file(file_path: &str) -> Result<Vec<Diagnostic>, String> {
    let file_path = absolute_path(file_path).to_string_lossy().to_string();
    let ext = extension_for(&file_path);
    let servers = server::for_extension(&ext);
    if servers.is_empty() {
        return Ok(vec![]);
    }

    let mut all_diags = Vec::new();
    for svr in servers {
        let sid = svr.id.to_string();
        let session = get_session(svr, &file_path).await?;
        session.diags.lock().unwrap().remove(&file_path);
        sync_document(&session, svr, &file_path).await?;

        let mut published = None;
        for _ in 0..600 {
            if let Some(diags) = session.diags.lock().unwrap().get(&file_path).cloned() {
                published = Some(diags);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        all_diags.extend(published.ok_or_else(|| {
            format!(
                "timed out waiting for {} diagnostics for {}",
                sid, file_path
            )
        })?);
    }
    Ok(all_diags)
}

async fn start_server(
    svr: &server::LspServer,
    root: &str,
) -> Result<
    (
        Child,
        ChildStdin,
        Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
        Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
    ),
    String,
> {
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
        .map_err(|e| format!("spawn {} ({}) in {}: {}", svr.id, bin_path, root, e))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut data = Vec::new();

    send_initialize(&mut stdin, root).await?;
    tokio::time::timeout(Duration::from_secs(30), async {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 8192];
        loop {
            let n = reader
                .read(&mut buf)
                .await
                .map_err(|e| format!("read initialize response: {}", e))?;
            if n == 0 {
                return Err("LSP exited during initialization".to_string());
            }
            data.extend_from_slice(&buf[..n]);
            while let Some((msg, rest)) = parse_jsonrpc(&data) {
                let initialized = msg.get("id").and_then(|v| v.as_u64()) == Some(1);
                data = rest.to_vec();
                if initialized {
                    return Ok(());
                }
            }
        }
    })
    .await
    .map_err(|_| "timed out initializing LSP".to_string())??;
    write_msg(
        &mut stdin,
        &serde_json::json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;

    let pending_arc: Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let pending = pending_arc.clone();
    let diags_arc: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let diags = diags_arc.clone();
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 8192];
        loop {
            while let Some((msg, rest)) = parse_jsonrpc(&data) {
                if msg.get("method").and_then(|v| v.as_str()).is_some() {
                    process_message(&msg, &diags).await;
                } else if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
                    if let Some(tx) = pending.lock().unwrap().remove(&id) {
                        let _ = tx.send(msg);
                    }
                }
                data = rest.to_vec();
            }
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => data.extend_from_slice(&buf[..n]),
            }
        }
    });

    Ok((child, stdin, pending_arc, diags_arc))
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

async fn send_initialize(stdin: &mut ChildStdin, root: &str) -> Result<(), String> {
    let msg = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "processId": std::process::id(),
            "rootUri": format!("file://{}", root),
            "workspaceFolders": [{ "uri": format!("file://{}", root), "name": "workspace" }],
            "clientInfo": { "name": "ws", "version": "0.1.0" },
            "capabilities": {
                "textDocument": {
                    "publishDiagnostics": { "relatedInformation": true }
                }
            }
        }
    });
    write_msg(stdin, &msg).await;
    Ok(())
}

async fn process_message(
    msg: &serde_json::Value,
    diags: &Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
) {
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
                diags.lock().unwrap().insert(path, collected);
            }
        }
    }
}

fn find_root(file_path: &str, root_mode: server::RootMode) -> String {
    let workspace = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    if root_mode == server::RootMode::Workspace {
        return workspace.to_string_lossy().to_string();
    }

    let path = absolute_path(file_path);
    let dir = path.parent().unwrap_or(&workspace);
    let markers = [
        "package.json",
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "bun.lockb",
        "bun.lock",
        "Cargo.toml",
        "Cargo.lock",
        "go.mod",
        "go.work",
        "pyproject.toml",
        "requirements.txt",
        "setup.py",
        "compile_commands.json",
        ".clangd",
    ];
    let mut current = Some(dir);
    while let Some(d) = current {
        if markers.iter().any(|m| d.join(m).exists()) {
            return d.to_string_lossy().to_string();
        }
        current = d.parent();
    }
    match root_mode {
        server::RootMode::Project => workspace.to_string_lossy().to_string(),
        server::RootMode::ProjectOrDir => dir.to_string_lossy().to_string(),
        server::RootMode::Workspace => unreachable!(),
    }
}

/// Build method-specific LSP params from an HTTP/MCP request.
pub fn lsp_params(
    path: &str,
    method: &str,
    req: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let uri = format!("file://{}", absolute_path(path).to_string_lossy());
    let position = || -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({
            "line": req.get("line").and_then(|v| v.as_u64()).ok_or("missing field: line")?,
            "character": req.get("column").and_then(|v| v.as_u64()).ok_or("missing field: column")?,
        }))
    };
    let text_document = || serde_json::json!({ "uri": uri.clone() });

    Ok(match method {
        "textDocument/documentSymbol" => serde_json::json!({ "textDocument": text_document() }),
        "workspace/symbol" => serde_json::json!({
            "query": req.get("query").and_then(|v| v.as_str()).ok_or("missing field: query")?
        }),
        "textDocument/rename" => serde_json::json!({
            "textDocument": text_document(),
            "position": position()?,
            "newName": req.get("new_name").and_then(|v| v.as_str()).ok_or("missing field: new_name")?,
        }),
        "textDocument/codeAction" => serde_json::json!({
            "textDocument": text_document(),
            "range": {
                "start": position()?,
                "end": {
                    "line": req.get("end_line").and_then(|v| v.as_u64()).or_else(|| req.get("line").and_then(|v| v.as_u64())).ok_or("missing field: line")?,
                    "character": req.get("end_column").and_then(|v| v.as_u64()).or_else(|| req.get("column").and_then(|v| v.as_u64())).ok_or("missing field: column")?,
                }
            },
            "context": { "diagnostics": [] },
        }),
        "textDocument/formatting" => serde_json::json!({
            "textDocument": text_document(),
            "options": {
                "tabSize": req.get("tab_size").and_then(|v| v.as_u64()).unwrap_or(4),
                "insertSpaces": req.get("insert_spaces").and_then(|v| v.as_bool()).unwrap_or(true),
            }
        }),
        "textDocument/hover"
        | "textDocument/definition"
        | "textDocument/typeDefinition"
        | "textDocument/implementation"
        | "textDocument/references"
        | "textDocument/completion"
        | "textDocument/signatureHelp"
        | "textDocument/prepareRename" => {
            let mut p = serde_json::json!({
                "textDocument": text_document(),
                "position": position()?,
            });
            if method == "textDocument/references" {
                p["context"] = serde_json::json!({ "includeDeclaration": true });
            }
            p
        }
        _ => return Err(format!("unsupported LSP method: {}", method)),
    })
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

pub fn list_servers() -> Vec<serde_json::Value> {
    server::SERVERS
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "language_id": s.language_id,
                "extensions": s.extensions,
                "binary": s.binary,
                "root_mode": match s.root_mode {
                    server::RootMode::Workspace => "workspace",
                    server::RootMode::Project => "project",
                    server::RootMode::ProjectOrDir => "project_or_dir",
                },
            })
        })
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
    let mut needed: HashSet<&str> = HashSet::new();
    for svr in server::SERVERS {
        for pkg in svr.nix_packages {
            needed.insert(pkg);
        }
        needed.insert(svr.binary);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    // Only keep LSP binaries+debs for languages with files in the workspace
    let mut active: HashSet<&str> = HashSet::new();
    walk_files(&cwd, &mut |p| {
        let ext = extension_for(&p.to_string_lossy());
        for svr in server::for_extension(&ext) {
            for pkg in svr.nix_packages {
                active.insert(pkg);
            }
            active.insert(svr.binary);
        }
    });
    let mut installed_count = 0;
    let mut uninstalled = 0;
    for pkg in needed {
        let on_path = std::process::Command::new("which")
            .arg(pkg)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let installed = crate::nix::is_installed(pkg);
        if active.contains(pkg) {
            if !on_path && !installed {
                eprintln!("ws: installing LSP pkg: {}", pkg);
                let _ = crate::nix::install_auto(pkg);
                installed_count += 1;
            }
        } else if installed {
            eprintln!("ws: uninstalling unused LSP pkg: {}", pkg);
            let _ = crate::nix::remove(pkg);
            uninstalled += 1;
        }
    }
    (installed_count, uninstalled)
}

#[cfg(test)]
mod tests {
    use super::{
        find_root, language_id, lsp_params, process_message, server::RootMode, Diagnostic,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn builds_method_specific_params_and_rejects_bad_requests() {
        let rename = serde_json::json!({ "line": 2, "column": 4, "new_name": "renamed" });
        let params = lsp_params("/workspace/main.go", "textDocument/rename", &rename).unwrap();
        assert_eq!(params["position"]["line"], 2);
        assert_eq!(params["newName"], "renamed");

        let symbols = serde_json::json!({ "query": "Widget" });
        let params = lsp_params("/workspace/main.go", "workspace/symbol", &symbols).unwrap();
        assert_eq!(params["query"], "Widget");

        assert!(lsp_params("/workspace/main.go", "textDocument/hover", &symbols).is_err());
        assert!(lsp_params("/workspace/main.go", "unknown", &serde_json::json!({})).is_err());

        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            find_root("src/main.rs", RootMode::ProjectOrDir),
            cwd.to_string_lossy()
        );
        let relative = serde_json::json!({ "line": 0, "column": 0 });
        let params = lsp_params("src/main.rs", "textDocument/hover", &relative).unwrap();
        assert_eq!(
            params["textDocument"]["uri"],
            format!("file://{}/src/main.rs", cwd.display())
        );
        assert_eq!(
            language_id("component.tsx", "typescript"),
            "typescriptreact"
        );
        assert_eq!(
            language_id("component.jsx", "typescript"),
            "javascriptreact"
        );
        assert_eq!(language_id("component.ts", "typescript"), "typescript");
    }

    #[tokio::test]
    async fn stores_latest_diagnostics_by_file_even_when_clean() {
        let diags: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        process_message(
            &serde_json::json!({
                "params": {
                    "uri": "file:///workspace/main.go",
                    "diagnostics": [{
                        "range": { "start": { "line": 2, "character": 4 } },
                        "severity": 1,
                        "message": "broken"
                    }]
                }
            }),
            &diags,
        )
        .await;
        assert_eq!(
            diags.lock().unwrap()["/workspace/main.go"][0].message,
            "broken"
        );

        process_message(
            &serde_json::json!({
                "params": { "uri": "file:///workspace/main.go", "diagnostics": [] }
            }),
            &diags,
        )
        .await;
        assert!(diags.lock().unwrap()["/workspace/main.go"].is_empty());
    }
}
