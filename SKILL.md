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

## File Operations

```bash
ws read <path>                    # Read file contents
ws write <path> <content>         # Write/create a file
ws edit <path> <old> <new>        # Replace text in a file
ws ls <path>                      # List directory entries
ws grep <pattern> [path]          # Search for pattern in files
ws find <path> [--name <glob>]    # Find files matching a glob
```

## Shell

```bash
ws bash "<command>"               # Execute any shell command
```

## Background Sessions

```bash
ws spawn "<command>"              # Start a long-running process
ws logs <session_id>              # Read stdout/stderr from a session
ws status <session_id>            # Check if session is running
ws kill <session_id>              # Kill a session
ws sessions                       # List all sessions
```

## LSP (Language Server Protocol)

LSP servers auto-start based on file extension. Supported: typescript, rust, go, python, c/c++, lua, bash, yaml, json, dockerfile, terraform, svelte, vue, astro, css, html, zig, elixir, php.

```bash
ws lsp-diagnose <path>            # Get diagnostics for a file
ws lsp-hover <path> <line> <col>  # Hover information
ws lsp-definition <path> <l> <c>  # Go to definition
ws lsp-references <path> <l> <c>  # Find references
ws lsp-completion <path> <l> <c>  # Code completion
ws lsp-sessions                   # List active LSP sessions
```

## Nix Packages

```bash
ws nix install <pkg>              # Install from nixpkgs
ws nix search <query>             # Search nixpkgs
ws nix list                       # List installed packages
ws nix remove <pkg>               # Remove a package
```

## MCP

For AI agents (Claude Desktop, OpenCode, Codex) that support MCP:

```bash
ws mcp                            # Exposes all tools via stdio JSON-RPC
```
