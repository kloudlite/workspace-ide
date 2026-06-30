# ws CLI Tool

`ws` is a CLI tool that communicates with a headless IDE server over HTTP. It provides file operations, shell execution, background process management, LSP diagnostics, and Nix package management.

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
ws write <path> <content>         # Write/create a file
ws edit <path> <old> <new>        # Replace text in a file
ws ls <path>                      # List directory entries
ws grep <pattern> [path]          # Search for pattern in files
ws find <path> [--name <glob>]    # Find files matching a glob
```

### Shell

```bash
ws bash "<command>"               # Execute any shell command
```

### Background Sessions

```bash
ws spawn "<command>"              # Start a long-running process
ws logs <session_id>              # Read stdout/stderr from a session
ws status <session_id>            # Check if session is running
ws kill <session_id>              # Kill a session
ws sessions                       # List all sessions
```

### LSP (Language Server Protocol)

LSP servers auto-start based on file extension. Supported: typescript, rust, go, python, c/c++, lua, bash, yaml, json, dockerfile, terraform, svelte, vue, astro, css, html, zig, elixir, php.

```bash
ws lsp-diagnose <path>            # Get diagnostics for a file
ws lsp-hover <path> <line> <col>  # Hover information
ws lsp-definition <path> <l> <c>  # Go to definition
ws lsp-references <path> <l> <c>  # Find references
ws lsp-completion <path> <l> <c>  # Code completion
ws lsp-sessions                   # List active LSP sessions
```

### Nix Packages

```bash
ws nix install <pkg>              # Install from nixpkgs
ws nix search <query>             # Search nixpkgs
ws nix list                       # List installed packages
ws nix remove <pkg>               # Remove a package
```

### MCP

For AI agents (Claude Desktop, OpenCode, Codex) that support MCP:

```bash
ws mcp                            # Exposes all tools via stdio JSON-RPC
```

## Best Practices

### Read Before Edit

Always read a file before editing it. This confirms the current content and avoids race conditions:

```bash
ws read src/main.rs               # Review current content
ws edit src/main.rs "old" "new"   # Then make the edit
```

For multiple edits, chain them as separate `ws edit` calls — each reads, replaces, and writes atomically.

### LSP Diagnostics First

Before fixing a file, run diagnostics to understand what's wrong:

```bash
ws lsp-diagnose src/main.rs       # Get all errors and warnings
# ... fix issues based on diagnostics ...
ws lsp-diagnose src/main.rs       # Verify fixes resolved the issues
```

### Hover for Understanding

When you encounter an unfamiliar symbol or type, use hover to learn about it:

```bash
ws read src/main.rs               # Find the symbol at line 42, column 10
ws lsp-hover src/main.rs 42 10    # Get type signature and docs
```

### Background Processes for Builds

For long-running commands (build watchers, dev servers, test suites), use spawn instead of bash:

```bash
ws spawn "cargo watch -x build"   # Start build watcher
ws logs <session_id>              # Check build output
```

This keeps the shell free for other commands. Check `ws sessions` periodically to clean up finished processes.

### Nix Packages Before Access

If a language/tool isn't available, install it first via nix before trying to use it:

```bash
ws nix install go                 # Install Go compiler
ws bash "go version"              # Verify it works
ws lsp-diagnose main.go           # LSP uses the installed tool
```

### Iterative Edit-Compile Loop

For compiled languages, use the edit-compile-diagnose cycle:

```bash
ws edit src/main.rs "func old" "func new"
ws bash "cargo build"              # Compile
ws lsp-diagnose src/main.rs        # Check for new errors
```

### Writing Short Content

For small files or scripts, prefer `ws write` over `ws edit` — it's a single atomic operation:

```bash
ws write Dockerfile "FROM rust:latest\nCOPY . /app\n"
```

For larger files, read first, edit in sections, and verify with diagnostics after each section.

## Workflows

### Fix a Compilation Error

```bash
# 1. Read the file with the error
ws read src/main.rs

# 2. Get diagnostics to see what's wrong
ws lsp-diagnose src/main.rs

# 3. Fix the error
ws edit src/main.rs "wrong_code" "fixed_code"

# 4. Rebuild and verify
ws bash "cargo build"
ws lsp-diagnose src/main.rs
```

### Explore an Unknown Codebase

```bash
# 1. List the project structure
ws ls .
ws ls src/

# 2. Find relevant files
ws grep "interesting_function" src/

# 3. Read the file
ws read src/interesting.rs

# 4. Use hover on unfamiliar types
ws lsp-hover src/interesting.rs 15 20

# 5. Find references to understand usage
ws lsp-references src/interesting.rs 15 20

# 6. Go to definition to see where things are defined
ws lsp-definition src/interesting.rs 15 20
```

### Add a New Feature

```bash
# 1. Find relevant files
ws grep "existing_feature" src/

# 2. Read them to understand patterns
ws read src/existing.rs

# 3. Make changes
ws edit src/existing.rs "pattern A" "pattern B"

# 4. Check diagnostics
ws lsp-diagnose src/existing.rs

# 5. Run the test suite in background
ws spawn "cargo test"

# 6. Check results
ws logs <session_id>
```

### Set Up a New Project from Scratch

```bash
# 1. Install required toolchain
ws nix install go nodejs rust-analyzer

# 2. Initialize project
ws bash "go mod init myproject"

# 3. Write initial code
ws write main.go "package main\nfunc main() {\n  println(\"hello\")\n}\n"

# 4. Verify
ws lsp-diagnose main.go
ws bash "go build"
```
