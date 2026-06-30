mod lsp;
mod mcp;
mod nix;
mod server;
mod tools;
mod watch;

use clap::{Parser, Subcommand};
use serde_json::json;

const DEFAULT_SERVER: &str = "http://localhost:8321";

#[derive(Parser)]
#[command(name = "ws", version, about = "Headless IDE remote client & server")]
struct Cli {
    /// Server URL (default: http://localhost:8321, override: REMOTE_WS env)
    #[arg(short, long, default_value = DEFAULT_SERVER)]
    server: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP server (with file watching + LSP)
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "8321")]
        port: u16,
    },
    /// Start the MCP server (stdio JSON-RPC, runs tools locally)
    Mcp,
    /// Read a file
    Read { path: String },
    /// Execute a shell command
    Bash { command: String },
    /// Edit a file with text replacements
    Edit {
        path: String,
        old_text: String,
        new_text: String,
    },
    /// Write content to a file
    Write { path: String, content: String },
    /// Search for a pattern in files
    Grep {
        pattern: String,
        path: Option<String>,
    },
    /// Find files matching a pattern
    Find {
        path: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// List directory contents
    Ls { path: String },
    /// Start a background command on the server
    Spawn { command: String },
    /// Read logs from a background session
    Logs { session_id: String },
    /// Check status of a background session
    Status { session_id: String },
    /// Kill a background session
    Kill { session_id: String },
    /// List all background sessions
    Sessions,
    /// Diagnose a file using LSP (auto-downloads + runs LSP server)
    Diagnose { path: String },
    /// List running LSP sessions
    LspSessions,
    /// LSP hover at a position
    LspHover { path: String, line: usize, character: usize },
    /// LSP go to definition at a position
    LspDefinition { path: String, line: usize, character: usize },
    /// LSP find references at a position
    LspReferences { path: String, line: usize, character: usize },
    /// LSP code completion at a position
    LspCompletion { path: String, line: usize, character: usize },
    /// Run a git command on the server
    #[command(trailing_var_arg = true)]
    Git { args: Vec<String> },
    /// Nix package management
    Nix {
        #[command(subcommand)]
        action: NixAction,
    },
}

#[derive(clap::Subcommand)]
enum NixAction {
    /// Install a package from nixpkgs
    Install { package: String },
    /// Search nixpkgs
    Search { query: String },
    /// List installed packages
    List,
    /// Remove a package
    Remove { package: String },
    /// Apply packages from ws.yaml (install missing ones)
    Apply,
    /// Sync ws.yaml + ws.lock from current state
    Sync,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Set up Nix profile in PATH (for nix commands + installed bins)
    nix::setup_env();

    match cli.command {
        Command::Serve { port } => {
            start_server(port).await;
        }
        Command::Mcp => {
            mcp::run().await;
        }
        Command::Nix { action } => {
            let result = match action {
                NixAction::Install { package } => nix::install(&package),
                NixAction::Search { query } => nix::search(&query).map(|r| r.join("\n")),
                NixAction::List => nix::list().map(|r| r.join("\n")),
                NixAction::Remove { package } => nix::remove(&package),
                NixAction::Apply => nix::apply_yaml(),
                NixAction::Sync => nix::sync(),
            };
            match result {
                Ok(out) => println!("{}", out),
                Err(e) => eprintln!("error: {}", e),
            }
        }
        cmd => {
            let server = std::env::var("REMOTE_WS").unwrap_or(cli.server);
            remote_call(&server, &cmd).await;
        }
    }
}

async fn start_server(port: u16) {
    // Set up Nix profile in PATH
    nix::setup_env();

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind failed");

    let app = server::router();
    println!(
        "ws {} listening on http://{}",
        env!("CARGO_PKG_VERSION"),
        addr
    );

    // Start file watcher in background (non-blocking)
    let _ = watch::start_watch(".");

    axum::serve(listener, app).await.expect("serve failed");
}

async fn remote_call(server: &str, cmd: &Command) {
    let client = reqwest::Client::new();
    let base = server.trim_end_matches('/');

    match cmd {
        Command::Sessions | Command::LspSessions => {
            let endpoint = match cmd {
                Command::Sessions => "sessions",
                Command::LspSessions => "lsp-sessions",
                _ => unreachable!(),
            };
            let url = format!("{}/{}", base, endpoint);
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    println!("{}", resp.text().await.unwrap_or_default());
                }
                Ok(resp) => {
                    eprintln!(
                        "sessions {}: {}",
                        resp.status(),
                        resp.text().await.unwrap_or_default()
                    );
                    std::process::exit(1);
                }
                Err(e) => connection_error(server, e),
            }
        }
        cmd => {
            let (method, body) = build_request(cmd);
            let url = format!("{}/{}", base, method);
            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    println!("{}", resp.text().await.unwrap_or_default());
                }
                Ok(resp) => {
                    eprintln!(
                        "{} {}: {}",
                        method,
                        resp.status(),
                        resp.text().await.unwrap_or_default()
                    );
                    std::process::exit(1);
                }
                Err(e) => connection_error(server, e),
            }
        }
    }
}

fn connection_error(server: &str, e: reqwest::Error) -> ! {
    eprintln!("error: cannot reach {} — {}", server, e);
    eprintln!("  start server:  ws serve");
    eprintln!("  set server:    --server <url> or REMOTE_WS=<url>");
    std::process::exit(1);
}

fn build_request(cmd: &Command) -> (&'static str, serde_json::Value) {
    match cmd {
        Command::Read { path } => ("read", json!({ "path": path })),
        Command::Bash { command } => ("bash", json!({ "command": command })),
        Command::Edit {
            path,
            old_text,
            new_text,
        } => (
            "edit",
            json!({ "path": path, "edits": [{"old_text": old_text, "new_text": new_text}] }),
        ),
        Command::Write { path, content } => ("write", json!({ "path": path, "content": content })),
        Command::Grep { pattern, path } => ("grep", json!({ "pattern": pattern, "path": path })),
        Command::Find { path, name } => ("find", json!({ "path": path, "name": name })),
        Command::Ls { path } => ("ls", json!({ "path": path })),
        Command::Spawn { command } => ("spawn", json!({ "command": command })),
        Command::Logs { session_id } => ("logs", json!({ "session_id": session_id })),
        Command::Status { session_id } => ("status", json!({ "session_id": session_id })),
        Command::Kill { session_id } => ("kill", json!({ "session_id": session_id })),
        Command::Diagnose { path } => ("lsp/diagnose", json!({ "path": path })),
        Command::LspHover { path, line, character } => {
            ("lsp/hover", json!({ "path": path, "line": line, "character": character }))
        }
        Command::LspDefinition { path, line, character } => {
            ("lsp/definition", json!({ "path": path, "line": line, "character": character }))
        }
        Command::LspReferences { path, line, character } => {
            ("lsp/references", json!({ "path": path, "line": line, "character": character }))
        }
        Command::LspCompletion { path, line, character } => {
            ("lsp/completion", json!({ "path": path, "line": line, "character": character }))
        }
        Command::Git { args } => ("bash", json!({ "command": format!("git {}", args.join(" ")) })),
        Command::Nix { .. }
        | Command::LspSessions
        | Command::Sessions
        | Command::Serve { .. }
        | Command::Mcp => unreachable!(),
    }
}
