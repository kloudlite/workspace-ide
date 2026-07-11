---
name: ws-harness
description: Use when working with a remote workspace via ws-pi — file operations, shell commands, package management, LSP diagnostics, and background sessions are backed by a remote ws HTTP server.
---

# ws-harness — Remote Workspace Agent

## ⚠️ CRITICAL: THIS IS REMOTE ONLY

All 21 tools talk to the **remote workspace server** via HTTP API. Workspace paths are remote. `upload` is the only tool that reads a local source file.

- The cwd is `/workspace` — pi tells the agent it works at `/workspace`, so it naturally uses `/workspace` paths.
- You don't need path remapping. Use workspace-relative paths normally.

### File Operations

| Tool | Parameters | Behaviour |
|------|-----------|-----------|
| `read` | `{ path }` | Returns file content + size. Use before edit to get exact text. |
| `write` | `{ path, content }` | Creates text file (incl. parent dirs) or overwrites entirely. |
| `upload` | `{ local_path, remote_path }` | Uploads a local file to the remote workspace. For pasted images, use the displayed `pi-clipboard-...` filename as `local_path`. |
| `edit` | `{ path, oldText, newText }` | Exact-text replacement — whitespace matters! One edit per call. |
| `ls` | `{ path }` | Lists entries with `name`, `is_dir`, `size`. |

**Best practice:** Always `read` before `edit` to see the exact text. After every `edit`, `write`, or `upload` of code/config, run `diagnose <path>` to catch LSP issues. `write` replaces the whole file — use `edit` for partial changes.

### Shell

| Tool | Parameters | Behaviour |
|------|-----------|-----------|
| `bash` | `{ command, description? }` | Runs any shell command. Returns `{ stdout, stderr, exitCode }`. |

**Best practice:**
- chain commands with `&&`
- each call is a **fresh shell** — exported vars don't persist

### Background Processes

| Tool | Parameters | Behaviour |
|------|-----------|-----------|
| `spawn` | `{ command }` | Start a long-running command. Returns session ID. |
| `logs` | `{ session_id }` | Get stdout/stderr from a session. |
| `status` | `{ session_id }` | Check if a session is still running. |
| `kill` | `{ session_id }` | Stop a running session. |
| `sessions` | `{}` | List all background sessions. |

**Best practice:** Use `spawn` for dev servers (`pnpm dev`), watchers (`cargo watch`), and other long-running processes.

### Search

| Tool | Parameters | Behaviour |
|------|-----------|-----------|
| `grep` | `{ pattern, path? }` | Recursive case-sensitive content search, returns matches with line numbers. |
| `find` | `{ path, name? }` | Filename search by glob (e.g. `*.go`, `*.ts`). |

**Best practice:** `grep` for content, `find` for filenames. Omit `path` in grep to search whole cwd.

### LSP

#### Code-intelligence routing rule

For questions like “what is this symbol/type/function?”, “where is this defined?”, “who references this?”, “what completions are available?”, or “what does hover say?” on a supported code file, use LSP.

- If the user gives a file + line/column: call `lsp` directly. Do not `read`/`grep` first.
- If the user gives a file + symbol but no line: use `grep` only to locate the symbol line, then call `lsp textDocument/hover` on that symbol. Do not answer from `grep` alone.
- For definitions and references: use `lsp textDocument/definition` / `textDocument/references`, not `grep`, unless LSP fails or the file type is unsupported.
- Use `read` only when you need surrounding source after LSP, or before editing.

| Tool | Parameters | Behaviour |
|------|-----------|-----------|
| `lsp_servers` | `{}` | Lists available LSP servers and supported extensions. Use this for “which LSPs are available?” |
| `lsp_sessions` | `{}` | Lists running LSP server sessions. Use this for “which LSP servers are running?” |
| `lsp` | `{ method, path, line, column }` | Methods: `textDocument/hover`, `textDocument/definition`, `textDocument/references`, `textDocument/completion`. Line/col are **0-indexed**. |
| `diagnose` | `{ path }` | Returns errors/warnings/hints. `[]` = clean. |

**Best practice:** For code-intelligence questions about a symbol/type/function/definition/references/completions/hover, use `lsp` first; only `read`/`grep` if LSP is unsupported, empty, or you need surrounding source. Always `diagnose` first when helping with compilation errors, and again after every file change that LSP can check. First LSP request can be slow (gopls indexing takes 30s+). If LSP returns empty, check if file extension is supported (Go, Rust, TS, Python, C/C++, Lua, Bash, YAML, JSON).

### Package Management

**CRITICAL: Use these tools for ALL package management. NEVER run raw package-manager commands via `bash`.**

| Tool | Parameters | Behaviour |
|------|-----------|-----------|
| `pkg_install` | `{ package }` | Install a package in foreground. UI shows an installing message while it runs. |
| `pkg_remove` | `{ package }` | Uninstall a package in foreground. UI shows a removing message while it runs. |
| `pkg_search` | `{ query }` | Search available packages |
| `pkg_list` | `{}` | List installed packages |

**If a package management tool is not in this table, it does not exist. Do not use bash-based package workarounds. Wait for `pkg_install`/`pkg_remove` to finish before using the package.**

## Common Workflows

### Coding / refactoring rule

When changing code, use LSP as part of the workflow, not just text search:

1. `diagnose <file>` before editing when the file type is LSP-supported.
2. For renames/refactors, use `lsp textDocument/references` or `definition` to understand affected symbols. `grep` may help find the first line/column, but do not rely on grep alone.
3. `read` the file before `edit` so exact text matches.
4. Use `edit` for small changes, `write` only for whole-file rewrites.
5. `diagnose <file>` after every code/config `edit`, `write`, or `upload`.

### Fix compilation errors
```
diagnose src/main.go           # show errors
read src/main.go               # read exact content
edit src/main.go "bug" "fix"   # fix the error
bash "go build ./..."           # verify
diagnose src/main.go           # confirm clean
```

### Explore unfamiliar codebase
```
ls /workspace
read path/to/module/main.go
```

### Answer “what is this symbol?”
```
# With line/column from the user: go straight to LSP
lsp textDocument/hover path/to/main.go 42 10

# With only a symbol name: grep only to find the line, then LSP hover
grep "type InterestingType" path/to/main.go
lsp textDocument/hover path/to/main.go 42 10
```

### Set up new Go project dependencies
```
# Check Go needs CGo
pkg_search gcc
pkg_install gcc
pkg_install golangci-lint
pkg_install protobuf
```

### Set up web frontend (bun/pnpm)
```
# web dir already has node_modules? run build
bash "cd /workspace/web && bun install"
```

### Commit and push
```
git status
git diff
git add -A && git commit -m "fix: description"
git push
```

### Check project health
```
diagnose src/main.go
bash "go vet ./..."
bash "go test ./..."
bash "go build ./..."
```

## Troubleshooting

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Tool returns `fetch failed` | ws server unreachable | Check server URL, SSH into host, restart container (`docker restart kloudlite-ws`) |
| `bash` returns empty stdout | Command produced no output | Check exitCode in details, add `echo done` to command |
| `diagnose` returns `[]` but code has errors | LSP not started yet | Wait 10s and retry, or read the file to trigger watcher |
| `pkg_install` succeeds but binary not found | Profile not refreshed | Run `pkg_install` again, then retry the command |
| `go build ./...` fails with `gcc not found` | CGo dependency | `pkg_install gcc` — the Go project uses CGo packages |
| `lsp` returns empty | File extension not supported | Check if extension has a server (Go/TS/Python/Rust/C/C++/Lua/Bash/YAML/JSON) |
| `git push` fails auth | No SSH key / token | Use `git remote set-url origin https://token@github.com/...` |
