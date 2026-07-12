# ws — remote workspace tools for pi

`ws` lets [pi](https://github.com/earendil-works/pi-coding-agent) work against code on another machine. Files, commands, diagnostics, language servers, and installed developer tools stay on the remote workspace; `ws-pi` is the local terminal UI.

## Get started

### 1. Run the workspace server

Mount your source at `/workspace`, plus `/nix` and the profile directory so installed language servers survive restarts:

```bash
docker run -d --name ws --restart unless-stopped \
  --user 1000:1000 -p 18765:18765 \
  -v /nix:/nix \
  -v /path/to/code:/workspace \
  -v ~/.local/state/nix/ws-profile:/home/kl/.local/state/nix \
  -e HOME=/home/kl -w /workspace \
  ghcr.io/kloudlite/workspace-ide:latest serve
```

Open inbound TCP `18765` in your host/provider firewall. The image is intentionally small: missing language servers install into the mounted profile when `ws` starts.

### 2. Install `ws-pi` once

```bash
git clone https://github.com/kloudlite/workspace-ide.git
cd workspace-ide/harness
npm install
npm run build
npm link
```

### 3. Start coding

Set your remote server once in your shell; a hostname is enough because `ws-pi` supplies HTTP and port `18765`:

```bash
export WS_SERVER_URL=dev.example.com
ws-pi
ws-pi "fix the failing checkout test"
```

Without `WS_SERVER_URL`, `ws-pi` uses `localhost:18765`. You can override either for one run:

```bash
ws-pi --server dev.example.com --new       # start a fresh conversation
ws-pi --list                               # list saved conversations for this server
```

Use normal language with the agent: “find usages of this function”, “rename this symbol”, “fix the failing test”, or “review this diff”. `!command` runs a remote shell command; `@` completes remote paths.

Sessions live under `~/.ws-sessions/`, isolated by server. Running `ws-pi` again continues the most recent session; use `--new` when you want a clean conversation. The footer identifies the remote workspace as `host:18765:/workspace`.

## What you get

- remote file reads, exact edits, and writes;
- remote shell commands and managed background processes;
- persistent LSP navigation, references, rename previews, and diagnostics;
- remote path completion and clipboard-image support;
- workspace package installation that persists in the mounted profile.

## Reference

Detailed tool behavior, LSP methods, package management, HTTP API, MCP, and image build/deployment notes are in [docs/reference.md](docs/reference.md).
