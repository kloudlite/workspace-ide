# ws — headless IDE

Remote IDE server with HTTP API, CLI client, and an AI agent harness (`ws-pi`) backed by [pi](https://github.com/earendil-works/pi-coding-agent).

## Quick start

```bash
# Server (inside container or VM)
cargo build --release && ./target/release/ws serve

# CLI client
ws read src/main.rs
ws bash "cargo build"
ws pkg install ripgrep

# AI agent (harness)
cd harness && npm install && npm run build && npm link
ws-pi --server http://host:8321
```

## AI agent harness (`ws-pi`)

The `harness/` directory contains `ws-pi` — a pi-compatible harness that routes all tool calls to the ws server over HTTP. It boots pi's InteractiveMode TUI with 18 remote tools (read, bash, edit, write, grep, find, ls, spawn, logs, status, kill, sessions, lsp, diagnose, pkg_install, pkg_search, pkg_list, pkg_remove) plus a remote-bash extension for `!` commands and `@` file autocomplete.

```bash
cd harness && npm install && npm run build && npm link
ws-pi --server http://kmac.khost.dev:8321   # interactive
ws-pi --server http://host:8321 "fix the lint errors"   # single-shot
ws-pi --new                       # fresh session
ws-pi --list                      # list past sessions
```

Skill docs: [harness/skills/ws-harness/SKILL.md](harness/skills/ws-harness/SKILL.md)

## CLI

| Category | Commands |
|----------|----------|
| Files | `read`, `write`, `edit`, `ls`, `grep`, `find` |
| Shell | `bash` |
| Background | `spawn`, `logs`, `status`, `kill`, `sessions` |
| LSP | `diagnose`, `lsp <method> <path> <line> <col>`, `lsp-sessions` |
| Packages | `pkg install`, `search`, `list`, `remove`, `apply`, `sync` |
| MCP | `ws mcp` (JSON-RPC over stdio) |
| Git | `ws fs status`, `ws fs diff` |

Full reference: [SKILL.md](SKILL.md)

## Connecting

```bash
ws --server http://host:8321 read file.go
WS_SERVER_URL=http://host:8321 ws read file.go
ws --ssh user@host read file.go  # tunnel via SSH
```

## Server

```bash
ws serve              # listens on :8321
ws serve -p 3000      # custom port
```

HTTP API mirrors the CLI: `POST /read`, `POST /bash`, `POST /edit`, etc. All requests accept `AbortSignal` — pressing Escape in `ws-pi` cancels inflight operations and kills the server-side process tree.

## Package management

Packages are installed via the host's package manager. `ws.yaml` tracks user-installed packages, `ws.lock` pins versions.

```bash
ws pkg install go        # install
ws pkg list              # show installed
ws pkg remove go         # uninstall
ws pkg apply             # restore from ws.yaml (runs on server boot)
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
cargo build --release        # Rust server
cd harness && npm run build   # AI harness
```

## Docker

Image is built via Dagger (`dagger call publish`). Pre-built images on GHCR:

```bash
docker pull ghcr.io/kloudlite/workspace-ide:latest
docker run -d --name ws \
  -p 8321:8321 \
  -v /nix:/nix \
  -v /path/to/code:/workspace \
  -v ~/.local/state/nix/ws-profile:/home/karthik/.local/state/nix \
  -e HOME=/home/karthik \
  -w /workspace \
  ghcr.io/kloudlite/workspace-ide:latest serve
```
