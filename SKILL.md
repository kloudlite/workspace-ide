---
name: ws-cli
description: "Use the ws CLI for remote headless-IDE development: semantic LSP navigation/refactoring, diagnostics, file edits, shell verification, background sessions, and workspace package tools."
---

# ws CLI

`ws` talks to a remote headless IDE over HTTP. Resolve the server in this order: `--server <url>`, `REMOTE_WS`, then `http://localhost:18765`. Use `--ssh user@host` to tunnel.

## Development loop

1. Explore symbols through LSP.
2. Read only relevant implementations/tests.
3. Diagnose before editing broken code.
4. Make the smallest exact edit.
5. Diagnose changed supported files.
6. Run focused tests/type-check/build.
7. Inspect `git status --short`, then review tracked diffs or untracked files correctly.

Show verification evidence; do not claim success because code merely looks correct.

## Minimum sufficient implementation

Stop at the first rung that fully satisfies the request:

1. no change if behavior exists or the need is speculative;
2. delete/reuse existing code (confirm with symbols/references);
3. standard library;
4. native browser/OS/runtime/database capability;
5. already-installed dependency;
6. smallest direct local implementation;
7. abstraction or new dependency only for multiple real uses or measured constraints.

Shortest correct diff wins. Do not add one-use interfaces/factories/wrappers, constant-valued configuration, compatibility aliases, fallback paths, caches, retries, concurrency, plugins, or future extension points without a present requirement. Do not refactor unrelated neighbors. Before writing a custom branch/loop/parser for a common transformation, compose stdlib primitives for splitting, joining, mapping, sorting, grouping, URL/path/date handling, encoding, or validation (`FieldsFunc` + `Join` for separator normalization when semantics match). Do not hand-optimize allocations/CPU with builders, pools, byte scanners, or concurrency without a request or measurement. If common logic exceeds roughly ten lines, pause and compare it with stdlib/native options before editing.

Minimal is not negligent: preserve trust-boundary validation, data-loss protection, security, accessibility, required errors, and physical calibration. Non-trivial branches/parsers/state/bug fixes need the smallest runnable regression check; reuse existing test infrastructure. Mark a deliberate shortcut with one `ponytail:` comment only when it has a known ceiling and upgrade trigger.

## Files

```bash
ws read <path>                    # CLI reads whole file; HTTP/MCP accept offset/limit
ws edit <path> <old> <new>
ws write <path> <content>
ws upload <local> <remote>
ws ls <path>
ws find <path> --name '<glob>'
ws grep <pattern> [path]
```

Use `read` before exact-text `edit`. Prefer `edit` for small changes; `write` replaces the whole file. After code/config mutation, run `diagnose` when supported. HTTP/MCP ranged reads use 1-indexed `offset`/`limit`; ws-pi defaults to 400 lines. `grep`/`find` return at most 200 results; grep snippets are capped at 500 characters. If truncated, narrow the range/path/pattern instead of treating output as complete.

## LSP

Servers start by file type and remain warm for the server lifetime. Diagnostics and semantic requests reuse the same `(server, project root)` process, synchronize document versions, and support concurrent request routing.

```bash
ws diagnose <path>
ws lsp textDocument/hover <path> <line> <column>
ws lsp textDocument/definition <path> <line> <column>
ws lsp textDocument/typeDefinition <path> <line> <column>
ws lsp textDocument/implementation <path> <line> <column>
ws lsp textDocument/references <path> <line> <column>
ws lsp textDocument/completion <path> <line> <column>
ws lsp textDocument/signatureHelp <path> <line> <column>
ws lsp textDocument/documentSymbol <path>
ws lsp workspace/symbol <path> --query SymbolName
ws lsp textDocument/prepareRename <path> <line> <column>
ws lsp textDocument/rename <path> <line> <column> --new-name NewName
ws lsp textDocument/codeAction <path> <line> <column> --end-line N --end-column N
ws lsp textDocument/formatting <path> --tab-size 4 --insert-spaces true
ws lsp-sessions
```

Positions are 0-indexed and must land on the identifier.

### Semantic routing

- Symbol/type/docs → hover.
- Declaration/type/concrete implementation → definition/typeDefinition/implementation.
- Impact analysis → references.
- File/project discovery → documentSymbol/workspace symbol.
- Rename → prepareRename, references, then rename preview.
- Quick fixes/refactors → codeAction preview.
- Literal/config/comment search → grep.
- Read source for implementation context and exact edit text, not as a substitute for code intelligence.

Rename, code-action, and formatting results are previews. Apply reviewed changes explicitly with `edit`/`write`, then diagnose and test. Never blind-global-replace a symbol when semantic rename is available.

For feature requests, derive the smallest observable behavior, check whether existing behavior already covers it, and identify the existing public owner before editing. Prefer stdlib/native/existing dependencies. Add no aliases, compatibility wrappers, configuration, or extension points without an existing caller/convention. Once focused acceptance tests pass, stop.

If LSP is empty: verify the token position/server support, retry once after warmup, then use grep/read and state the fallback. Oversized results are explicitly truncated/refused; narrow the query before acting.

## Shell and background work

`bash` blocks until completion and each call is a fresh shell. Never hide test/build/diff failure with `|| true`:

```bash
ws bash 'cargo test -p package'
ws bash 'git diff --check && git diff -- src/file.rs'
```

Use background sessions for servers/watchers:

```bash
ws spawn 'pnpm dev'
ws status <session_id>
ws logs <session_id>
ws kill <session_id>
ws sessions
```

Never run a persistent process through blocking `bash`. After deleting tracked files, use `git grep` for source checks; do not pipe `git ls-files` into grep because the index still lists deleted paths until commit and causes ENOENT.

## Packages

Use workspace package commands for compilers, language servers, and developer CLIs; never raw OS/Nix package commands:

```bash
ws pkg search <query>
ws pkg install <pkg>[@version]
ws pkg list
ws pkg remove <pkg>
ws pkg apply
ws pkg sync
```

Use the repository's existing package manager for project dependencies (`pnpm`, `npm`, `bun`, `cargo`, `go mod`, etc.). Do not introduce a new dependency or package manager without need. Commit `ws.yaml` and `ws.lock` when package state changes.

## Audit/review budget

For repo-wide audits, scope the codebase to tracked source/config (`git ls-files`) by default; ignored/generated/build/untracked artifacts do not count unless cleanup is requested. Use bounded inventories/searches for breadth, but verify claims that symbols/interfaces/helpers are dead, duplicated, single-implementation, or removable with LSP references/definition/implementation—grep counts are not semantic proof. Inspect at most 20 strongest candidates with ranged reads, require concrete evidence, report at most 10 findings, and stop. Grep accepts one directory path, not a space-separated path list. Never ingest the repository or repeat equivalent searches.

## Verification ladder

Use the narrowest checks that prove the change:

1. LSP diagnostics on changed files.
2. Focused formatter/linter/type-check.
3. Nearest unit/package test.
4. Wider build/test for cross-package impact.
5. Run `git status --short` once. Use `git diff --check` and scoped `git diff` for tracked files. For `??` files, `git diff` is empty by design; inspect once with `read` or `git diff --no-index /dev/null <file>`.

Diagnostics are fast feedback, not a substitute for tests/builds. Security, migration, concurrency, public API, and data-loss-sensitive changes require stronger verification.

## MCP

```bash
ws mcp
```

MCP exposes the same file, shell, background, diagnostics, and generic LSP request capabilities over stdio JSON-RPC.
