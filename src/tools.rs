use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

// --- Shared types ---

#[derive(Deserialize)]
pub struct EditOp {
    pub old_text: String,
    pub new_text: String,
}

#[derive(Serialize)]
pub struct ReadResult {
    pub content: String,
    pub size: u64,
}

#[derive(Serialize)]
pub struct BashResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Serialize)]
pub struct EditResult {
    pub applied: usize,
    pub errors: Vec<String>,
}

#[derive(Serialize)]
pub struct WriteResult {
    pub size: u64,
}

#[derive(Serialize)]
pub struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub text: String,
}

#[derive(Serialize)]
pub struct GrepResult {
    pub matches: Vec<GrepMatch>,
}

#[derive(Serialize)]
pub struct FindResult {
    pub files: Vec<String>,
}

#[derive(Serialize)]
pub struct LsEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Serialize)]
pub struct LsResult {
    pub entries: Vec<LsEntry>,
}

#[derive(Debug)]
pub struct ToolError(pub String);

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<std::io::Error> for ToolError {
    fn from(e: std::io::Error) -> Self {
        ToolError(e.to_string())
    }
}

// --- Tool implementations ---

pub async fn read_file(path: &str) -> Result<ReadResult, ToolError> {
    let content = tokio::fs::read_to_string(path).await?;
    let size = content.len() as u64;
    Ok(ReadResult { content, size })
}

pub async fn run_bash(command: &str, timeout_secs: Option<u64>) -> BashResult {
    let mut child = Command::new("sh");
    child.arg("-c").arg(command);
    child.stdout(Stdio::piped()).stderr(Stdio::piped());
    // ponytail: include nix user profile bin so pkg_install'd tools are on PATH
    if let Ok(home) = std::env::var("HOME") {
        let nix_bin = format!("{}/.local/state/nix/profile/bin", home);
        if let Ok(existing) = std::env::var("PATH") {
            child.env("PATH", format!("{}:{}", nix_bin, existing));
        }
    }

    let output = match timeout_secs {
        Some(secs) => {
            match tokio::time::timeout(std::time::Duration::from_secs(secs), child.output()).await {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => {
                    return BashResult {
                        stdout: String::new(),
                        stderr: e.to_string(),
                        exit_code: -1,
                    };
                }
                Err(_) => {
                    return BashResult {
                        stdout: String::new(),
                        stderr: "command timed out".into(),
                        exit_code: -1,
                    };
                }
            }
        }
        None => match child.output().await {
            Ok(o) => o,
            Err(e) => {
                return BashResult {
                    stdout: String::new(),
                    stderr: e.to_string(),
                    exit_code: -1,
                };
            }
        },
    };

    BashResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    }
}

pub async fn edit_file(path: &str, edits: &[EditOp]) -> Result<EditResult, ToolError> {
    let content = tokio::fs::read_to_string(path).await?;
    let mut result = content;
    let mut applied = 0usize;
    let mut errors = Vec::new();

    for (i, edit) in edits.iter().enumerate() {
        if edit.old_text.is_empty() {
            errors.push(format!("edit[{}]: old_text is empty", i));
            continue;
        }
        if !result.contains(&edit.old_text) {
            errors.push(format!("edit[{}]: old_text not found", i));
            continue;
        }
        result = result.replace(&edit.old_text, &edit.new_text);
        applied += 1;
    }

    tokio::fs::write(path, &result).await?;
    Ok(EditResult { applied, errors })
}

pub async fn write_file(path: &str, content: &str) -> Result<WriteResult, ToolError> {
    if let Some(parent) = Path::new(path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let size = content.len() as u64;
    tokio::fs::write(path, content).await?;
    Ok(WriteResult { size })
}

pub async fn grep_files(pattern: &str, search_path: Option<&str>) -> Result<GrepResult, ToolError> {
    let path = search_path.unwrap_or(".");
    // ponytail: canonicalize to handle /tmp -> /private/tmp symlinks
    let canonical = std::path::Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(path));
    // ponytail: uses grep -rnHI; ripgrep (rg --json) is faster for large codebases
    let output = Command::new("grep")
        .args(["-rnHI", "--"])
        .arg(pattern)
        .arg(canonical.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    // ponytail: splitn(3,':') assumes path:line:text; breaks if path contains ':'
    let stdout = String::from_utf8_lossy(&output.stdout);
    let matches: Vec<GrepMatch> = stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ':');
            let path = parts.next()?;
            let line_number: usize = parts.next()?.parse().ok()?;
            let text = parts.next().unwrap_or("");
            Some(GrepMatch {
                path: path.to_string(),
                line_number,
                text: text.to_string(),
            })
        })
        .collect();

    Ok(GrepResult { matches })
}

pub async fn find_files(path: &str, name: Option<&str>) -> Result<FindResult, ToolError> {
    // ponytail: uses find (always available); fd is faster for large trees
    let canonical = std::path::Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(path));
    let mut cmd = Command::new("find");
    cmd.arg(canonical.as_os_str());
    cmd.arg("-type").arg("f");
    if let Some(name) = name {
        cmd.arg("-name").arg(name);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let files: Vec<String> = stdout.lines().map(|s| s.to_string()).collect();
    Ok(FindResult { files })
}

// --- Background process sessions ---

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Serialize)]
pub struct SpawnResult {
    pub session_id: String,
    pub pid: u32,
}

#[derive(Serialize)]
pub struct LogsResult {
    pub stdout: String,
    pub stderr: String,
    pub stdout_len: u64,
    pub stderr_len: u64,
    pub running: bool,
    pub exit_code: Option<i32>,
}

#[derive(Serialize)]
pub struct StatusResult {
    pub session_id: String,
    pub pid: u32,
    pub command: String,
    pub running: bool,
    pub exit_code: Option<i32>,
}

#[derive(Serialize)]
pub struct KillResult {
    pub killed: bool,
    pub message: String,
}

struct SessionState {
    pid: u32,
    command: String,
    log_dir: std::path::PathBuf,
    running: bool,
    exit_code: Option<i32>,
}

fn sessions() -> &'static Mutex<HashMap<String, SessionState>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, SessionState>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn generate_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:016x}", nanos)
}

fn pump_to_file(
    stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    path: std::path::PathBuf,
) {
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let mut file = tokio::fs::File::create(&path).await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) => break,
                Ok(_) => {
                    file.write_all(&buf).await.unwrap();
                }
                Err(_) => break,
            }
        }
    });
}

pub async fn spawn_bash(command: &str) -> Result<SpawnResult, ToolError> {
    let session_id = generate_session_id();
    let log_dir = std::env::temp_dir().join(format!("ws-{}", session_id));
    tokio::fs::create_dir_all(&log_dir).await?;

    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    // ponytail: same nix PATH fix as run_bash
    if let Ok(home) = std::env::var("HOME") {
        let nix_bin = format!("{}/.local/state/nix/profile/bin", home);
        if let Ok(existing) = std::env::var("PATH") {
            cmd.env("PATH", format!("{}:{}", nix_bin, existing));
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| ToolError(format!("failed to spawn: {}", e)))?;

    let pid = child.id().unwrap_or(0);

    // Pump stdout/stderr to log files
    if let Some(stream) = child.stdout.take() {
        pump_to_file(stream, log_dir.join("stdout"));
    }
    if let Some(stream) = child.stderr.take() {
        pump_to_file(stream, log_dir.join("stderr"));
    }

    // Register session
    {
        let mut map = sessions().lock().unwrap();
        map.insert(
            session_id.clone(),
            SessionState {
                pid,
                command: command.to_string(),
                log_dir,
                running: true,
                exit_code: None,
            },
        );
    }

    // Watch for completion
    let sid = session_id.clone();
    tokio::spawn(async move {
        let status = child.wait().await;
        let exit_code = status.ok().and_then(|s| s.code());
        let mut map = sessions().lock().unwrap();
        if let Some(s) = map.get_mut(&sid) {
            s.running = false;
            s.exit_code = exit_code;
        }
    });

    Ok(SpawnResult { session_id, pid })
}

pub async fn get_logs(session_id: &str) -> Result<LogsResult, ToolError> {
    let (running, exit_code, log_dir) = {
        let map = sessions().lock().unwrap();
        let s = map
            .get(session_id)
            .ok_or_else(|| ToolError("session not found".into()))?;
        (s.running, s.exit_code, s.log_dir.clone())
    };

    let stdout = tokio::fs::read_to_string(log_dir.join("stdout"))
        .await
        .unwrap_or_default();
    let stderr = tokio::fs::read_to_string(log_dir.join("stderr"))
        .await
        .unwrap_or_default();
    let stdout_len = stdout.len() as u64;
    let stderr_len = stderr.len() as u64;

    Ok(LogsResult {
        stdout,
        stderr,
        stdout_len,
        stderr_len,
        running,
        exit_code,
    })
}

pub async fn get_status(session_id: &str) -> Result<StatusResult, ToolError> {
    let map = sessions().lock().unwrap();
    let s = map
        .get(session_id)
        .ok_or_else(|| ToolError("session not found".into()))?;
    Ok(StatusResult {
        session_id: session_id.to_string(),
        pid: s.pid,
        command: s.command.clone(),
        running: s.running,
        exit_code: s.exit_code,
    })
}

pub async fn kill_session(session_id: &str) -> Result<KillResult, ToolError> {
    let pid = {
        let map = sessions().lock().unwrap();
        let s = map
            .get(session_id)
            .ok_or_else(|| ToolError("session not found".into()))?;
        if !s.running {
            return Ok(KillResult {
                killed: false,
                message: "already exited".into(),
            });
        }
        s.pid
    };

    if pid == 0 {
        return Ok(KillResult {
            killed: false,
            message: "no pid".into(),
        });
    }

    // ponytail: kill via `kill` command; signal(SIGTERM) more direct but adds dep
    let status = tokio::process::Command::new("kill")
        .arg(pid.to_string())
        .status()
        .await
        .map_err(|e| ToolError(format!("kill failed: {}", e)))?;

    if status.success() {
        let mut map = sessions().lock().unwrap();
        if let Some(s) = map.get_mut(session_id) {
            s.running = false;
            s.exit_code = s.exit_code.or(Some(-15));
        }
        Ok(KillResult {
            killed: true,
            message: "SIGTERM sent".into(),
        })
    } else {
        Ok(KillResult {
            killed: false,
            message: "kill command failed".into(),
        })
    }
}

pub async fn list_sessions() -> Vec<StatusResult> {
    let map = sessions().lock().unwrap();
    map.iter()
        .map(|(id, s)| StatusResult {
            session_id: id.clone(),
            pid: s.pid,
            command: s.command.clone(),
            running: s.running,
            exit_code: s.exit_code,
        })
        .collect()
}

pub async fn list_dir(path: &str) -> Result<LsResult, ToolError> {
    let mut dir = tokio::fs::read_dir(path).await?;
    let mut entries = Vec::new();

    while let Some(entry) = dir.next_entry().await? {
        let meta = match entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue,
        };
        entries.push(LsEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: meta.len(),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(LsResult { entries })
}
