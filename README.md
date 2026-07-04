# ws — headless IDE

Remote IDE server + CLI client. Provides file ops, shell execution, background processes, LSP diagnostics, and reproducible package management over HTTP.

## Quick start

```bash
# Server
cargo build --release
./target/release/ws serve

# Client (another terminal)
ws read src/main.rs
ws bash "cargo build"
ws pkg install ripgrep
```

## CLI

| Category | Commands |
|----------|----------|
| Files | `read`, `write`, `edit`, `ls`, `grep`, `find` |
| Shell | `bash`, `git` |
| Background | `spawn`, `logs`, `status`, `kill`, `sessions` |
| LSP | `diagnose`, `lsp <method> <path> <line> <col>`, `lsp-sessions` |
| Packages | `pkg install`, `search`, `list`, `remove`, `apply`, `sync` |
| MCP | `ws mcp` (JSON-RPC over stdio) |

Full reference: [SKILL.md](SKILL.md)

## Connecting

```bash
ws --server http://host:8321 read file.go
REMOTE_WS=http://host:8321 ws read file.go
ws --ssh user@host read file.go  # tunnel via SSH
```

## Server

```bash
ws serve              # listens on :8321
ws serve -p 3000      # custom port
```

HTTP API mirrors the CLI. `POST /read`, `POST /bash`, `POST /edit`, etc.

## Package management

Packages are installed on the server via the host's nix daemon. `ws.yaml` tracks user-installed packages, `ws.lock` pins exact versions and store paths for reproducibility.

```bash
ws pkg install go        # install latest
ws pkg list              # show installed
ws pkg remove go         # uninstall
ws pkg apply             # restore from ws.yaml (runs on server boot too)
ws pkg sync              # rebuild ws.yaml + ws.lock from current state
```

## LSP

Nine language servers, auto-started on file access:

| Language | Server |
|----------|--------|
| Go | gopls |
| Rust | rust-analyzer |
| TypeScript/JS | typescript-language-server |
| Python | pyright |
| C/C++ | clangd |
| Lua | lua-language-server |
| Bash/Zsh | bash-language-server |
| YAML | yaml-language-server |
| JSON | json-languageserver |

## Build

```bash
cargo build --release
```

Requires Rust 1.86+. Dependencies: axum, tokio, serde, serde_json, clap, reqwest.

## Docker

```bash
# Image is built via Dagger on self-hosted runner
docker pull ghcr.io/kloudlite/workspace-ide:latest

# Run with shared nix store and workspace
docker run -d --name ws \
  -p 8321:8321 \
  -v /nix:/nix \
  -v /path/to/code:/workspace \
  -v ~/.local/state/nix/ws-profile:/home/karthik/.local/state/nix \
  -e HOME=/home/karthik \
  -w /workspace \
  ghcr.io/kloudlite/workspace-ide:latest serve
```

MCP agent integration: [SKILL.md#mcp](SKILL.md#mcp)
