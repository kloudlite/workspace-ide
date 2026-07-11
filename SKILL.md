# ws CLI Tool

`ws` is a CLI tool that communicates with a headless IDE server over HTTP. It provides file operations, shell execution, background process management, LSP diagnostics, and package management with version pinning.

## Connecting

The server address is resolved in order:
1. `--server <url>` flag
2. `REMOTE_WS` environment variable
3. `http://localhost:8321` (default)

```bash
ws --server http://host:8321 read file.go
REMOTE_WS=http://host:8321 ws read file.go
```

## Commands

### File Operations

```bash
ws read <path>                    # Read file contents
ws write <path> <content>         # Write/create a text file
ws upload <local> <remote>        # Upload a local file to the remote workspace
ws edit <path> <old> <new>        # Replace text in a file
ws ls <path>                      # List directory entries
ws grep <pattern> [path]          # Search for pattern in files
ws find <path> [--name <glob>]    # Find files matching a glob
```

### Shell

**IMPORTANT: `bash` blocks until the command exits.** Use it only for short-lived commands that complete within seconds (compiles, tests, file ops). For anything that runs indefinitely — dev servers, watchers, daemons — use `spawn` instead.

```bash
ws bash "<command>"               # Short-lived shell command (blocks until done)
ws bash "git status"              # Run git through bash
ws bash "git diff"                # Show git diff through bash
```

### Background Sessions

Use `spawn` for any command that won't exit on its own — dev servers, file watchers, build daemons. The command runs in the background; check output later with `logs`.

```bash
ws spawn "<command>"              # Start a long-running process
ws logs <session_id>              # Read stdout/stderr from a session
ws status <session_id>            # Check if session is running
ws kill <session_id>              # Kill a session
ws sessions                       # List all sessions
```

### LSP (Language Server Protocol)

LSP servers auto-start based on file extension. Supported languages: rust, go, typescript, python, c/c++, lua, bash, yaml, json.

```bash
ws diagnose <path>                                      # Get diagnostics for a file
ws lsp textDocument/hover <path> <line> <col>           # Hover information
ws lsp textDocument/definition <path> <line> <col>      # Go to definition
ws lsp textDocument/references <path> <line> <col>      # Find references
ws lsp textDocument/completion <path> <line> <col>      # Code completion
ws lsp-sessions                                         # List active LSP sessions
```

### Packages

Packages are managed with version pinning. Each workspace has `ws.yaml` (manifest) and `ws.lock` (lockfile) — commit both to your repo for reproducibility.

```bash
ws pkg install <pkg>[@version]    # Install (CLI blocks; harness runs this in background)
ws pkg search <query>             # Search available packages
ws pkg list                       # List installed packages
ws pkg remove <pkg>               # Remove a package (CLI blocks; harness runs this in background)
ws pkg apply                      # Install packages from ws.yaml
ws pkg sync                       # Sync ws.yaml + ws.lock from current state
```

### MCP

For AI agents (Claude Desktop, OpenCode, Codex):

```bash
ws mcp                            # Exposes all tools via stdio JSON-RPC
```

## Best Practices

### Read Before Edit

```bash
ws read src/main.rs               # Review current content first
ws edit src/main.rs "old" "new"   # Then make the edit
```

Chain multiple edits as separate `ws edit` calls — each is atomic. After every code/config `edit`, `write`, or `upload`, run `ws diagnose <path>` when the file type has LSP support.

### LSP Diagnostics First

```bash
ws diagnose src/main.rs       # Understand what's wrong
# ... fix issues ...
ws diagnose src/main.rs       # Verify fixes
```

### Hover for Understanding

```bash
ws lsp-hover src/main.rs 42 10    # Get type signature and docs at line 42, col 10
ws lsp-definition src/main.rs 42 10   # Go to definition
ws lsp-references src/main.rs 42 10   # Find usages
```

### Background Builds

```bash
ws spawn "cargo watch -x build"   # Long-running build watcher
ws logs <session_id>              # Check output later
```

### Install Before Use

```bash
ws pkg install go                 # Install Go compiler
ws bash "go version"              # Verify
ws diagnose main.go           # LSP uses the installed tool
```

### Version Pinning for Reproducibility

```bash
ws pkg install go@1.26            # Pin exact version
ws pkg install nodejs@22          # Pin Node.js 22
# → ws.yaml + ws.lock auto-updated
# → Commit both files for reproducible workspaces
```

### Edit-Compile-Diagnose Loop

```bash
ws edit src/main.rs "func old" "func new"
ws bash "cargo build"
ws diagnose src/main.rs
```

## Workflows

### Fix a Compilation Error

```bash
ws read src/main.rs
ws diagnose src/main.rs
ws edit src/main.rs "wrong_code" "fixed_code"
ws bash "cargo build"
ws diagnose src/main.rs
```

### Explore an Unknown Codebase

```bash
ws ls .
ws grep "interesting_function" src/
ws read src/interesting.rs
ws lsp-hover src/interesting.rs 15 20
ws lsp-references src/interesting.rs 15 20
ws lsp-definition src/interesting.rs 15 20
```

### Set Up a New Project

```bash
ws pkg install go@1.26 nodejs@22
ws bash "go mod init myproject"
ws write main.go "package main\nfunc main() {\n  println(\"hello\")\n}\n"
ws diagnose main.go
ws bash "go build"
ws pkg sync                        # Generate ws.yaml + ws.lock
```

### Reproduce a Workspace from ws.yaml

```bash
ws pkg apply                       # Installs all packages from ws.yaml
# Uses pinned versions from ws.lock
# Same versions, every time
```
