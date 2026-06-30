# ws — Headless IDE Server CLI

`ws` is a headless IDE server that provides file operations, shell execution, LSP diagnostics, and Nix package management over HTTP and MCP. It runs as a daemon (`ws serve`) and exposes a CLI that communicates with the server via HTTP.

## Quick Start

```bash
# Start the server (daemon)
ws serve

# Run commands against the server (default: http://localhost:8321)
ws read path/to/file
ws bash "command"
ws ls path/to/dir
```

## Server URL Resolution

The CLI connects to a remote ws server. The server URL is resolved in order:
1. `--server <url>` flag
2. `REMOTE_WS` environment variable
3. `http://localhost:8321` (default)

```bash
ws --server http://10.0.0.5:8321 read file.go
REMOTE_WS=http://10.0.0.5:8321 ws read file.go
```

## Commands

### File Operations

| Command | Description | Example |
|---------|-------------|---------|
| `ws read <path>` | Read file contents | `ws read src/main.rs` |
| `ws write <path> <content>` | Write/create a file | `ws write main.go 'package main'` |
| `ws edit <path> <old> <new>` | Replace text in a file | `ws edit main.go 'foo' 'bar'` |
| `ws ls <path>` | List directory entries | `ws ls src/` |
| `ws grep <pattern> [path]` | Search files for pattern | `ws grep "fn main" src/` |
| `ws find <path> [--name <glob>]` | Find files matching glob | `ws find . --name "*.rs"` |

### Shell Execution

| Command | Description | Example |
|---------|-------------|---------|
| `ws bash <command>` | Execute shell command | `ws bash "cargo build"` |

### Background Sessions

Long-running processes managed by the server:

| Command | Description | Example |
|---------|-------------|---------|
| `ws spawn <command>` | Start a background process | `ws spawn "npm run dev"` |
| `ws logs <session_id>` | Read stdout/stderr from a session | `ws logs abc123` |
| `ws status <session_id>` | Check if a session is still running | `ws status abc123` |
| `ws kill <session_id>` | Stop a background session | `ws kill abc123` |
| `ws sessions` | List all background sessions | `ws sessions` |

### LSP (Language Server Protocol)

LSP servers auto-start when files matching their extensions are accessed. Servers are auto-downloaded (npm, cargo, GitHub releases) if not found on PATH.

Supported LSP servers: typescript, rust-analyzer, gopls, pyright, clangd, lua-ls, bash-language-server, yaml-language-server, json-languageserver, dockerfile-language-server, terraform-ls, svelte, vue, astro, css-languageserver, html-languageserver, zls, elixir-ls, intelephense.

| Command | Description | Example |
|---------|-------------|---------|
| `ws lsp-diagnose <path>` | Run diagnostics on a file | `ws lsp-diagnose src/main.rs` |
| `ws lsp-hover <path> <line> <col>` | Get hover info at position | `ws lsp-hover src/main.rs 10 5` |
| `ws lsp-definition <path> <line> <col>` | Go to definition | `ws lsp-definition src/main.rs 10 5` |
| `ws lsp-references <path> <line> <col>` | Find references | `ws lsp-references src/main.rs 10 5` |
| `ws lsp-completion <path> <line> <col>` | Get code completions | `ws lsp-completion src/main.rs 10 5` |
| `ws lsp-sessions` | List active LSP sessions | `ws lsp-sessions` |

### Nix Package Management

Packages are installed via the host's `nix-daemon` (requires `/nix` mount). Each container gets its own profile — packages installed in one container don't leak to others. The `/nix/store` is shared across all containers.

| Command | Description | Example |
|---------|-------------|---------|
| `ws nix install <package>` | Install a package from nixpkgs | `ws nix install go` |
| `ws nix search <query>` | Search nixpkgs | `ws nix search python` |
| `ws nix list` | List installed packages | `ws nix list` |
| `ws nix remove <package>` | Remove a package | `ws nix remove go` |

### MCP Server

The MCP server exposes all tools via JSON-RPC over stdio, for AI agents (Claude Desktop, OpenCode, Codex):

```bash
ws mcp
```

Available tools: read, bash, edit, write, grep, find, ls, spawn, logs, status, kill, sessions, diagnose, lsp_hover, lsp_definition, lsp_references, lsp_completion, lsp_sessions

## Architecture

```
ws serve (HTTP server on :8321)
  ├── File operations (read, write, edit, ls, grep, find)
  ├── Shell execution (bash)
  ├── Background sessions (spawn, logs, status, kill)
  ├── LSP (diagnose, hover, definition, references, completion)
  └── Nix package management (install, search, list, remove)

ws mcp (MCP stdio server)
  └── Same tools via JSON-RPC for AI agents

ws CLI client
  └── All commands → HTTP POST → ws serve
```

## Container Usage

```bash
# Run with Nix support (mount /nix from host)
docker run -d \
  -v /nix:/nix \
  -v $(pwd):/workspace \
  -w /workspace \
  --name my-ws \
  ghcr.io/kloudlite/workspace-ide:latest serve

# Run commands inside the container
docker exec my-ws ws read /workspace/main.go
docker exec my-ws ws bash "go build"
docker exec my-ws ws nix install nodejs
```
