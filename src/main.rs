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
    #[arg(short, long, default_value = DEFAULT_SERVER)]
    server: String,
    #[arg(long)]
    ssh: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(short, long, default_value = "8321")]
        port: u16,
    },
    Mcp,
    Read {
        path: String,
    },
    Bash {
        command: String,
    },
    Edit {
        path: String,
        old_text: String,
        new_text: String,
    },
    Write {
        path: String,
        content: String,
    },
    Grep {
        pattern: String,
        path: Option<String>,
    },
    Find {
        path: String,
        #[arg(long)]
        name: Option<String>,
    },
    Ls {
        path: String,
    },
    Spawn {
        command: String,
    },
    Logs {
        session_id: String,
    },
    Status {
        session_id: String,
    },
    Kill {
        session_id: String,
    },
    Sessions,
    Diagnose {
        path: String,
    },
    Lsp {
        method: String,
        path: String,
        line: u32,
        column: u32,
    },
    LspSessions,
    #[command(trailing_var_arg = true)]
    Git {
        args: Vec<String>,
    },
    Pkg {
        #[command(subcommand)]
        action: PkgAction,
    },
}

#[derive(clap::Subcommand)]
enum PkgAction {
    Install { package: String },
    Search { query: String },
    List,
    Remove { package: String },
    Apply,
    Sync,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    nix::setup_env();

    match cli.command {
        Command::Serve { port } => start_server(port).await,
        Command::Mcp => mcp::run().await,
        Command::Pkg { action } => {
            let result = match action {
                PkgAction::Install { package } => nix::install(&package),
                PkgAction::Search { query } => nix::search(&query).map(|r| r.join("\n")),
                PkgAction::List => nix::list().map(|r| r.join("\n")),
                PkgAction::Remove { package } => nix::remove(&package),
                PkgAction::Apply => nix::apply_yaml(),
                PkgAction::Sync => nix::sync(),
            };
            match result {
                Ok(out) => println!("{}", out),
                Err(e) => eprintln!("error: {}", e),
            }
        }
        cmd => {
            if let Some(ref ssh_host) = cli.ssh.or_else(|| std::env::var("WS_SSH").ok()) {
                ssh_call(ssh_host, &cmd);
            } else {
                let server = std::env::var("REMOTE_WS").unwrap_or(cli.server);
                remote_call(&server, &cmd).await;
            }
        }
    }
}

async fn start_server(port: u16) {
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
    watch::start_watch(".");
    // ponytail: restore user-installed packages on startup (reinstalls from cache)
    match crate::nix::apply_yaml() {
        Ok(msg) => eprintln!("ws: {}", msg),
        Err(e) => eprintln!("ws: apply_yaml error: {}", e),
    }
    axum::serve(listener, app).await.expect("serve failed");
}

async fn remote_call(server: &str, cmd: &Command) {
    let client = reqwest::Client::new();
    let base = server.trim_end_matches('/');

    match cmd {
        Command::Sessions | Command::LspSessions => {
            let endpoint = match cmd {
                Command::Sessions => "sessions",
                Command::LspSessions => "lsp/sessions",
                _ => unreachable!(),
            };
            let url = format!("{}/{}", base, endpoint);
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    println!("{}", resp.text().await.unwrap_or_default())
                }
                Ok(resp) => {
                    eprintln!(
                        "{} {}: {}",
                        endpoint,
                        resp.status(),
                        resp.text().await.unwrap_or_default()
                    );
                    std::process::exit(1);
                }
                Err(e) => connection_error(server, e),
            }
        }
        cmd => {
            let (method, body) = command_route(cmd);
            let url = format!("{}/{}", base, method);
            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    println!("{}", resp.text().await.unwrap_or_default())
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

fn ssh_call(host: &str, cmd: &Command) {
    // Build CLI args directly from Command — JSON map iteration is alphabetical, not insertion order
    let args: Vec<String> = match cmd {
        Command::Lsp {
            method,
            path,
            line,
            column,
        } => {
            vec![
                "lsp".into(),
                method.clone(),
                path.clone(),
                line.to_string(),
                column.to_string(),
            ]
        }
        Command::Find { path, name } => {
            let mut v = vec!["find".into(), path.clone()];
            if let Some(n) = name {
                v.push("--name".into());
                v.push(n.clone());
            }
            v
        }
        cmd => {
            let (route, body) = command_route(cmd);
            let mut v = vec![route.to_string()];
            if let Some(obj) = body.as_object() {
                for val in obj.values() {
                    match val {
                        serde_json::Value::String(s) => v.push(s.clone()),
                        serde_json::Value::Number(n) => v.push(n.to_string()),
                        _ => {}
                    }
                }
            }
            v
        }
    };
    let cmdline: String = args
        .iter()
        .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");
    let status = std::process::Command::new("ssh")
        .args([
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
        ])
        .arg(host)
        .arg(format!("ws {}", cmdline))
        .status()
        .expect("ssh failed");
    std::process::exit(status.code().unwrap_or(1));
}

fn connection_error(server: &str, e: reqwest::Error) -> ! {
    eprintln!("error: cannot reach {} — {}", server, e);
    eprintln!("  start server:  ws serve");
    eprintln!("  set server:    --server <url> or REMOTE_WS=<url>");
    std::process::exit(1);
}

// ponytail: single match arm for both HTTP and SSH paths.
fn command_route(cmd: &Command) -> (&'static str, serde_json::Value) {
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
        Command::Lsp {
            method,
            path,
            line,
            column,
        } => (
            "lsp/request",
            json!({ "method": method, "path": path, "line": line, "column": column }),
        ),
        Command::Git { args } => (
            "bash",
            json!({ "command": format!("git {}", args.join(" ")) }),
        ),
        Command::Sessions => ("sessions", json!({})),
        Command::LspSessions => ("lsp-sessions", json!({})),
        Command::Pkg { .. } | Command::Serve { .. } | Command::Mcp => unreachable!(),
    }
}
