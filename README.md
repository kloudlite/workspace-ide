# ws — headless IDE for AI agents

`ws` turns a remote workspace into an IDE-grade HTTP service. `ws-pi` runs the full [pi](https://github.com/earendil-works/pi-coding-agent) TUI against that service with remote file tools, shell execution, persistent language servers, diagnostics, semantic code navigation, package management, and session history.

```text
┌──────────── ws-pi / pi TUI ────────────┐       HTTP :8321       ┌──────────┐
│ read · edit · bash · LSP · packages    │ ─────────────────────▶ │ ws serve │
│ !commands · @paths · clipboard images  │                        │ workspace│
└─────────────────────────────────────────┘                        └──────────┘
```

## Why

AI coding agents should work like a good IDE user:

1. discover symbols and contracts semantically;
2. read only relevant source;
3. make minimal edits;
4. receive fresh diagnostics;
5. run focused tests/builds;
6. inspect status and diffs before completion.

`ws` provides those primitives without mounting the remote filesystem locally or requiring an interactive editor.

## Quick start

### Server

```bash
cargo build --release
./target/release/ws serve                  # http://0.0.0.0:8321
```

### Agent harness

```bash
cd harness
npm install
npm run build
npm link

ws-pi --server http://host:8321            # interactive TUI
ws-pi --server http://host:8321 "fix the failing checkout test"
```

Other harness options:

```bash
ws-pi --new                                # fresh session
ws-pi --session <path>                     # open a session file
ws-pi --list                               # list sessions for this server
```

Sessions are stored under `~/.ws-sessions/<sha256(server-url)[:12]>/`. Different servers have isolated histories; without `--new`, the latest session continues.

## ws-pi tools

All workspace operations go through the configured remote server. Two narrow local exceptions exist:

- `upload` reads the explicitly requested local source file;
- `read` may load files only from pi-discovered skill directories, allowing normal progressive skill loading.

Every other path is remote and relative paths resolve under `/workspace`.

| Tool | Endpoint | Purpose |
|---|---|---|
| `read` | `POST /read` | Read exact source/context with optional line range |
| `edit` | `POST /edit` | Exact-text replacement |
| `write` | `POST /write` | Create or replace a complete file |
| `upload` | `POST /upload` | Upload local bytes |
| `ls` | `POST /ls` | List a directory |
| `find` | `POST /find` | Find files by glob |
| `grep` | `POST /grep` | Literal recursive search |
| `bash` | `POST /bash` | Run a finite command |
| `spawn` | `POST /spawn` | Start a persistent command |
| `logs` | `POST /logs` | Read background output |
| `status` | `POST /status` | Check a background command |
| `kill` | `POST /kill` | Kill its process group |
| `sessions` | `GET /sessions` | List background commands |
| `lsp` | `POST /lsp/request` | Generic semantic LSP request |
| `diagnose` | `POST /lsp/diagnose` | Fresh per-file diagnostics |
| `lsp_servers` | `GET /lsp/servers` | Available servers/extensions/root modes |
| `lsp_sessions` | `GET /lsp/sessions` | Warm `(server, project root)` sessions |
| `pkg_install` | `POST /pkg/install` | Install a workspace developer tool |
| `pkg_search` | `POST /pkg/search` | Search developer tools |
| `pkg_list` | `POST /pkg/list` | List installed tools |
| `pkg_remove` | `POST /pkg/remove` | Remove a developer tool |

### Bounded context

Tool output is intentionally bounded:

- `read`: `ws-pi` defaults to 400 lines and returns a continuation offset; explicit `offset`/`limit` select a 1-indexed line range;
- `grep`: POSIX basic regex in one directory, at most 200 matches and 500 characters per matching line;
- `bash`: model-visible stdout/stderr is capped at 50,000 characters with head and tail preserved;
- `find`: at most 200 files;
- oversized LSP arrays: first 200 items with explicit truncation metadata;
- unsafe LSP results over 1 MB are refused with instructions to narrow the request.

Truncated results are never presented as complete. Narrow the project path, query, symbol, or range before acting.

## Language intelligence

Language servers are keyed by `(server, project root)`, initialized once, and retained for the `ws serve` lifetime. Diagnostics and semantic requests share the same process and synchronized document versions. Concurrent JSON-RPC requests are routed by request ID.

Typical warm requests complete in tens of milliseconds; the first request may wait for project indexing.

### Supported languages

| Language | Server | Root policy |
|---|---|---|
| TypeScript/JavaScript | `typescript-language-server` | nearest project marker |
| Rust | `rust-analyzer` | nearest project marker |
| Go | `gopls` | project marker, otherwise file directory |
| Python | `pyright` | project marker, otherwise file directory |
| C/C++ | `clangd` | project marker, otherwise file directory |
| Lua | `lua-language-server` | workspace |
| Bash/Zsh | `bash-language-server` | workspace |
| YAML | `yaml-language-server` | workspace |
| JSON | `json-languageserver` | workspace |

Simple filetype servers share one workspace session; project-aware servers remain isolated by project root.

### LSP methods

The single `lsp` tool avoids redundant per-method tools.

| Need | Method | Extra arguments |
|---|---|---|
| Type/docs | `textDocument/hover` | `line`, `column` |
| Declaration | `textDocument/definition` | `line`, `column` |
| Declared type | `textDocument/typeDefinition` | `line`, `column` |
| Concrete implementation | `textDocument/implementation` | `line`, `column` |
| Usages/impact | `textDocument/references` | `line`, `column` |
| Completion | `textDocument/completion` | `line`, `column` |
| Call signature | `textDocument/signatureHelp` | `line`, `column` |
| File outline | `textDocument/documentSymbol` | — |
| Project symbol search | `workspace/symbol` | `query` |
| Validate rename | `textDocument/prepareRename` | `line`, `column` |
| Preview semantic rename | `textDocument/rename` | `line`, `column`, `new_name` |
| Quick fixes/refactors | `textDocument/codeAction` | start/end range |
| Formatting edits | `textDocument/formatting` | formatting options |

Positions are zero-indexed and must land on the identifier token. Rename, code-action, and formatting responses are previews; reviewed file mutation remains explicit through `edit`/`write`.

### Diagnostics

`diagnose`:

- opens a document once and sends versioned `didChange` notifications thereafter;
- waits for a fresh `publishDiagnostics` notification for that file;
- stores diagnostics by path, including explicit clean results;
- never returns another file's or a stale diagnostic set.

Diagnostics are fast feedback, not a substitute for tests or builds.

## Agent development workflow

The bundled [`ws-harness` skill](harness/skills/ws-harness/SKILL.md) teaches pi to:

- use LSP for semantic questions and grep for text questions;
- follow a minimum-sufficient ladder: no change → delete/reuse → stdlib → native platform → installed dependency → direct code → justified abstraction;
- prefer the shortest correct diff and reject speculative wrappers, aliases, configuration, dependencies, and extension points;
- compose standard-library/native primitives before writing custom loops/parsers, and avoid unmeasured micro-optimizations;
- scope monorepo exploration before searching;
- inspect definitions/implementations/references before shared refactors;
- preview semantic renames rather than global text replacement;
- avoid speculative APIs and compatibility wrappers;
- diagnose every changed supported file;
- preserve command failures instead of masking them with `|| true`;
- stop after focused verification passes;
- distinguish tracked from untracked files when reviewing changes;
- keep repo-wide audits bounded: audit tracked source/config by default, inventory/search for breadth, inspect at most 20 strongest files with ranged reads, and report at most 10 evidence-backed findings. Ignored/generated/untracked workspace artifacts are excluded unless cleanup is requested.

Recommended loop:

```text
semantic exploration → relevant reads → minimal edits
→ diagnostics → focused test/build → status/diff review
```

For untracked files, remember that `git diff` is empty by design: check `git status --short`, then review with `read` or `git diff --no-index /dev/null <file>` once.

The minimum-sufficient implementation rules are adapted from [Ponytail](https://github.com/DietrichGebert/ponytail) (MIT): lazy means efficient, not careless. Validation, security, accessibility, data-loss protection, and proportional runnable checks remain non-negotiable.

## TUI integration

`harness/src/extensions/remote-bash.ts` adds:

- **`!command`** — execute an inline command remotely;
- **`@path` completion** — autocomplete paths from the remote workspace;
- **clipboard images** — attach pasted images to the model with a compact placeholder. Upload bytes only when requested.

Press Escape to abort an inflight tool request. Finite shell commands are killed on client disconnect; `spawn` sessions remain managed explicitly through `status`, `logs`, and `kill`.

Pi skill commands work normally: `/skill:name [arguments]` is treated as an explicit invocation and executes immediately against the remote workspace. `ws-pi` must not merely acknowledge that a skill was loaded or ask for a second command.

## Shell and packages

`bash` waits until the command exits. Use it for tests, builds, formatting, and finite repository commands. Use `spawn` for servers, watchers, and daemons.

Each `bash` call starts a fresh shell, so chain dependent commands with `&&`. Never hide verification failures with `|| true`.

Workspace system/developer tools use:

```bash
ws pkg search <query>
ws pkg install <package>[@version]
ws pkg list
ws pkg remove <package>
ws pkg apply
ws pkg sync
```

Project dependencies continue to use the repository's existing package manager (`pnpm`, `npm`, `bun`, `cargo`, `go mod`, etc.). `ws.yaml` records workspace tools and `ws.lock` pins them; commit both when tool state changes. Package restoration runs when the server starts.

## Standalone CLI

Server resolution order:

1. `--server <url>`
2. `REMOTE_WS`
3. `http://localhost:8321`

```bash
ws --server http://host:8321 read src/main.rs
REMOTE_WS=http://host:8321 ws diagnose src/main.rs
ws --ssh user@host bash "git status --short"
```

Generic LSP examples:

```bash
ws lsp textDocument/hover src/main.go 42 10
ws lsp textDocument/references src/main.go 42 10
ws lsp textDocument/documentSymbol src/main.go
ws lsp workspace/symbol src/main.go --query Service
ws lsp textDocument/rename src/main.go 42 10 --new-name NewName
```

Full CLI and workflow reference: [`SKILL.md`](SKILL.md).

## HTTP API

Core routes:

```text
POST /read /bash /edit /write /upload /grep /find /ls
POST /spawn /logs /status /kill       GET /sessions
POST /pkg/install /pkg/search /pkg/list /pkg/remove
POST /lsp/diagnose /lsp/request /lsp/reconcile
GET  /lsp/sessions /lsp/servers
POST /fs/tree                         GET /fs/status /fs/diff
```

`POST /upload` accepts a raw request body and destination in `x-ws-path`.

## MCP

```bash
ws mcp
```

MCP exposes the same file, shell, background, diagnostics, and generic LSP capabilities over stdio JSON-RPC.

## Build and test

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release

cd harness
npm install
npm run build
```

The container image is built with Dagger (`main.go`); there is no Dockerfile.

## Container

```bash
docker pull ghcr.io/kloudlite/workspace-ide:latest

docker run -d --name ws \
  --user 1000:1000 \
  -p 8321:8321 \
  -v /nix:/nix \
  -v /path/to/code:/workspace \
  -v ~/.local/state/nix/ws-profile:/home/karthik/.local/state/nix \
  -e HOME=/home/karthik \
  -w /workspace \
  ghcr.io/kloudlite/workspace-ide:latest serve
```

The image contains the `ws` binary, Git, UID 1000 user/home, and minimal runtime utilities. Mount `/nix` and the per-user profile state for persistent developer-tool installations.
