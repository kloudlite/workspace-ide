# ws — headless IDE

Remote IDE server with an AI agent harness (`ws-pi`) powered by [pi](https://github.com/earendil-works/pi-coding-agent). Also ships a standalone CLI.

## AI agent harness (`ws-pi`)

`ws-pi` boots pi's full InteractiveMode TUI with every tool routed to the remote ws server over HTTP. You get the pi experience — code editing, shell, LSP, package management — backed by a remote workspace, not your local filesystem.

```
┌─ ws-pi ──────────────────────────────────┐
│  ┌─ pi TUI ──────────────────────────┐   │
│  │  read bash edit write upload find │   │    HTTP     ┌──────────┐
│  │  ls spawn logs kill lsp pkg_*     │───│───────────│ ws serve │
│  │  !commands  @file-autocomplete    │   │    :8321   └──────────┘
│  └───────────────────────────────────┘   │
└──────────────────────────────────────────┘
```

### Setup

```bash
cd harness && npm install && npm run build && npm link
```

### Usage

```bash
ws-pi --server http://host:8321                    # interactive TUI
ws-pi --server http://host:8321 "fix the errors"   # single-shot print mode
ws-pi --new                                        # fresh session
ws-pi --session <path>                             # open specific session
ws-pi --list                                       # list past sessions
```

Sessions are persisted in `~/.ws-sessions/<hash-of-server-url>/` — each server connection gets isolated session history. Default: continues the most recent session.

### 19 remote tools

All tool calls are HTTP requests to the ws server. No local filesystem access.

| Tool | Endpoint | Description |
|------|----------|-------------|
| `read` | `/read` | Read a file |
| `bash` | `/bash` | Run shell command (short-lived; see spawn) |
| `edit` | `/edit` | Edit file by exact text replacement |
| `write` | `/write` | Write/create file |
| `upload` | `/upload` | Upload local file to remote workspace |
| `grep` | `/grep` | Recursive pattern search |
| `find` | `/find` | Find files by name |
| `ls` | `/ls` | List directory |
| `spawn` | `/spawn` | Background process |
| `logs` | `/logs` | Get spawn output |
| `status` | `/status` | Check spawn status |
| `kill` | `/kill` | Kill spawn (kills process group, no orphans) |
| `sessions` | `/sessions` | List all background sessions |
| `lsp` | `/lsp/request` | LSP hover/definition/references/completion |
| `diagnose` | `/lsp/diagnose` | LSP diagnostics for a file |
| `pkg_install` | `/pkg/install` | Install a package |
| `pkg_search` | `/pkg/search` | Search packages |
| `pkg_list` | `/pkg/list` | List installed packages |
| `pkg_remove` | `/pkg/remove` | Uninstall a package |

### Remote extension

`harness/src/extensions/remote-bash.ts` adds three features inside the pi TUI:

- **`!command`** — inline bash: type `!cargo build` and it runs on the server, output streams back
- **`@` autocomplete** — file path completion from the remote workspace (`@src/m` → `src/main.rs`)
- **Clipboard images** — paste screenshots, injected as base64 into the message

### Escape / interrupt

Pressing Escape in `ws-pi` cancels inflight tool calls. The harness passes `AbortSignal` to every `fetch`, and the ws server kills the child process group on disconnect — no orphaned dev servers holding ports. Background sessions (`spawn`) are killed via process group (`kill -TERM -<pid>`), not just the shell.

---

## Standalone CLI

The `ws` binary also works as a standalone CLI. Sends HTTP requests to the server.

```bash
ws --server http://host:8321 read src/main.rs
ws --server http://host:8321 bash "cargo build"
WS_SERVER_URL=http://host:8321 ws read file.go
ws --ssh user@host read file.go   # tunnel via SSH
```

| Category | Commands |
|----------|----------|
| Files | `read`, `write`, `upload`, `edit`, `ls`, `grep`, `find` |
| Shell | `bash` |
| Background | `spawn`, `logs`, `status`, `kill`, `sessions` |
| LSP | `diagnose`, `lsp <method> <path> <line> <col>`, `lsp-sessions` |
| Packages | `pkg install`, `search`, `list`, `remove`, `apply`, `sync` |
| Git | `bash "git status"`, `bash "git diff"` |
| MCP | `ws mcp` (JSON-RPC over stdio) |

Full CLI reference: [SKILL.md](SKILL.md)

---

## Server

```bash
ws serve              # listens on :8321
ws serve -p 3000      # custom port
```

HTTP API mirrors the CLI tool-for-tool. UI-only filesystem endpoints are also available: `POST /fs/tree`, `GET /fs/status`, `GET /fs/diff`.

---

## Package management

Packages are installed via the host's package manager. `ws.yaml` tracks user-installed packages, `ws.lock` pins versions.

```bash
ws pkg install go        # install
ws pkg list              # show installed
ws pkg remove go         # uninstall
ws pkg apply             # restore from ws.yaml (runs on server boot too)
```

---

## LSP

Nine language servers, auto-started on file access. LSP deps auto-installed and cleaned up (~10min reconcile cycle).

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

---

## Build

```bash
cargo build --release          # Rust server
cd harness && npm run build     # AI agent harness
```

---

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
