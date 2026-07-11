---
name: ws-harness
description: Use for all development in ws-pi's remote workspace. Covers semantic LSP navigation and refactoring, diagnostics, minimal file edits, shell verification, package tools, background processes, and evidence-based completion.
---

# ws-harness — Remote IDE Workflow

## Remote boundary

All workspace tools operate on the remote server and `/workspace`; relative paths resolve there. `upload` reads a requested local source file. `read` has one read-only local exception for pi-discovered skill directories so skills can load normally; every other path is remote.

## Core development loop

Use the shortest loop that proves the change:

1. **Explore semantically** — discover symbols, definitions, types, implementations, and references with `lsp`.
2. **Plan impact** — identify affected symbols/files and existing project patterns before editing.
3. **Edit minimally** — `read` exact source, then `edit`; avoid whole-file `write` unless replacing the whole file is genuinely simpler.
4. **Check immediately** — run `diagnose` on every changed supported code/config file.
5. **Verify behavior** — run the narrowest relevant formatter, test, type-check, build, or lint command.
6. **Inspect the diff** — verify only intended files/lines changed before claiming success.

Do not say a change works without reporting the check that passed. Fix root causes; do not hide diagnostics or weaken tests.

## Minimum sufficient implementation

Use this ladder before writing code; stop at the first rung that fully satisfies the request:

1. **No change** — the behavior already exists, the requirement is speculative, or documentation/configuration solves it.
2. **Delete/reuse** — remove obsolete code or reuse an existing symbol/pattern. Check with `workspace/symbol`, definitions, and references before adding a duplicate.
3. **Standard library** — prefer the language's maintained primitive over custom helpers.
4. **Native platform** — prefer browser/OS/runtime/database constraints and capabilities over application machinery.
5. **Installed dependency** — use an existing project dependency when it directly solves the problem.
6. **Direct local code** — write the smallest boring implementation that covers observed requirements.
7. **Abstraction/new dependency** — only when multiple real uses or measured constraints justify ownership cost.

### Complexity budget

- Shortest correct diff wins; deletion is better than addition.
- Do not create an interface for one implementation, a factory for one product, a wrapper around one direct call, or configuration for a value that never varies operationally.
- Do not add compatibility aliases, fallback paths, caches, retries, concurrency, plugins, hooks, feature flags, or “future-proof” extension points without a present requirement or measurement.
- Do not refactor neighboring code merely because it could be cleaner. Touch only what the requested behavior and its proof require.
- Prefer explicit, local, boring code over clever generic machinery. A future requirement can pay for its own abstraction.
- Before adding a dependency, confirm stdlib/native/existing dependencies cannot do it. Project dependencies increase install, update, security, and context cost.
- A custom branch/loop/parser for a common transformation is a last resort. First compose stdlib primitives for splitting, joining, mapping, sorting, grouping, cloning, URL/path/date handling, encoding, and validation. Example: separator/token normalization should prefer `FieldsFunc` + `Join` (+ case conversion) over a hand-written scanner when semantics match.
- Do not trade maintainability for speculative allocation/CPU gains (`Builder`, pooling, custom byte scans, concurrency) unless the user requests performance or a measurement shows the standard composition is inadequate.
- If a common operation takes more than roughly ten lines of new logic, pause before editing and check whether stdlib/native/existing code expresses it with fewer states and branches. Compare the complete diff, not cleverness.
- When a deliberate shortcut has a known ceiling, leave one concise `ponytail:` comment naming the ceiling and upgrade trigger, not an essay.

### Efficient proof

Minimal code still needs proportional proof:

- Trivial one-line changes need diagnostics and the nearest existing check, not a new test framework.
- New branching, parsing, state transitions, security/money/data-loss behavior, or bug fixes need the smallest runnable regression test that would fail before the change.
- Reuse existing test style and infrastructure; do not add fixtures/helpers for one test unless they reduce total code.
- Never simplify away trust-boundary validation, data-loss prevention, security controls, accessibility, required error handling, or physical calibration.

Before the first edit, silently answer: can this be deleted/reused, can stdlib/native do it, and what is the smallest check? If the proposed direct implementation has more moving parts than a standard composition, choose the standard composition.

In the final response, lead with what changed and the passing check. Mention a deliberately skipped larger design only when the user might reasonably expect it or when a measured threshold should trigger it.

## LSP: primary code-intelligence tool

Use LSP for semantic questions; use text tools for text questions.

| Need | LSP method |
|---|---|
| Understand a symbol/type/doc | `textDocument/hover` |
| Follow declaration | `textDocument/definition` |
| Find declared type | `textDocument/typeDefinition` |
| Find concrete implementation | `textDocument/implementation` |
| Assess usages/impact | `textDocument/references` |
| Inspect call signature | `textDocument/signatureHelp` |
| Ask for valid completions | `textDocument/completion` |
| Outline one file | `textDocument/documentSymbol` |
| Find a symbol across a project | `workspace/symbol` with `query` |
| Validate rename target | `textDocument/prepareRename` |
| Preview semantic rename edits | `textDocument/rename` with `new_name` |
| Discover quick fixes/refactors | `textDocument/codeAction` with a range |
| Request language formatting edits | `textDocument/formatting` |

Positions are **0-indexed** and must land on the identifier token.

### Routing rules

- Given file + line/column: call `lsp` directly; do not read/grep first.
- Given only a symbol name: prefer `workspace/symbol`. If unsupported/empty, `grep` only to locate its line, then use LSP.
- For definitions, implementations, types, references, signatures, and renames: LSP first.
- For literals, comments, log messages, config keys, generated text, or unsupported files: `grep` is correct.
- Use `read` for surrounding implementation context and exact edit text, not as a substitute for symbol intelligence.
- Do not call `lsp_servers` as routine preflight; methods auto-select by file. Use it only when asked about availability or after an unsupported/failing request.
- Do not call symbols/navigation methods when an explicit target plus diagnostics/source already establishes the needed edit. Every semantic call should answer a concrete uncertainty.
- If LSP fails or returns empty: verify support with `lsp_servers`, retry once after warmup, then fall back to `grep`/`read` and state the fallback.
- First request on a large project may be slow; warm sessions are reused and should remain running.

### Safe semantic refactoring

For rename/move/signature changes:

1. Locate the exact symbol (`workspace/symbol`, `documentSymbol`, or minimal `grep`).
2. Inspect `definition`/`typeDefinition`/`implementation` as relevant.
3. Call `references` to understand impact.
4. Use `prepareRename`, then `rename` to preview authoritative workspace edits.
5. Read only files named by the rename preview plus separately identified implementations; do not scan unrelated files.
6. Apply the smallest edits with `edit`.
7. Diagnose every changed file and run focused tests/type-checks.
8. Inspect `git diff`.

LSP rename, code-action, and formatting responses are previews; file mutation remains explicit through `edit`/`write`. Never perform a blind global text rename when symbols may be shadowed or share names.

### Diagnostics

- Run `diagnose` before changing broken code to establish the baseline.
- Run it after every code/config `edit`, `write`, or `upload` supported by an available server.
- Diagnostics are fast feedback, not a replacement for project tests/builds.
- Do not assume `(none)` proves runtime correctness.

## Tool selection

### Files and search

| Tool | Use |
|---|---|
| `read` | Exact source/context before editing; defaults to 400 lines, continue with `offset`/`limit` |
| `edit` | Small exact-text replacement; preferred mutation tool |
| `write` | New file or intentional whole-file replacement |
| `upload` | Local binary/image to remote workspace |
| `ls` / `find` | Discover paths/files |
| `grep` | POSIX basic-regex text search in one directory, or locating a symbol when semantic search fails; never pass space-separated paths |

`edit` exact text is case/whitespace-sensitive. `write` overwrites the entire file.

### LSP inventory

- `lsp_servers {}`: available servers, extensions, and root modes.
- `lsp_sessions {}`: currently warm `(server, root)` sessions.
- `diagnose { path }`: file diagnostics.
- `lsp { method, path, ... }`: all semantic requests; do not invent separate LSP tools.

### Shell and long-running work

- `bash`: finite commands; each invocation is a fresh shell, so chain dependent commands with `&&`.
- Never append `|| true` to tests, builds, diagnostics, or diff checks: it hides failure. Run optional searches separately.
- Use the `grep` tool for searches instead of assuming `rg` or another CLI is installed.
- `spawn`: dev servers, watchers, or any persistent process.
- `logs`, `status`, `kill`, `sessions`: manage spawned processes.
- Never block `bash` on a watcher/server that does not exit.

### Packages

Use `pkg_search`, `pkg_install`, `pkg_list`, and `pkg_remove` for **workspace system/developer tools** such as compilers, language servers, and CLIs. Do not use raw OS/Nix package commands.

Project dependencies still use the repository's existing lockfile/package manager through `bash` (`npm`, `pnpm`, `bun`, `cargo`, `go mod`, etc.). Do not introduce a new package manager or dependency without need.

## Efficient exploration

Keep context small and semantic:

1. In a monorepo, identify the likely project/service directory with `ls`/narrow `find` before content search. Never begin with workspace-wide `grep` for a common term.
2. Scope `find`/`grep` to that directory; if output says truncated, narrow again rather than consuming or acting on partial results.
3. `documentSymbol` for file structure or `workspace/symbol` for named concepts.
4. `hover` + `definition`/`implementation` for contracts and behavior.
5. `references` before changing public/shared code.
6. `read` only relevant implementations and nearby tests.

Avoid dumping entire repositories, generated files, lockfiles, dependency trees, or huge reference lists into context.

### Audit/review budget

For whole-repository audits, scope “the codebase” to tracked source/config (`git ls-files`) by default. Ignored/generated/build/untracked workspace artifacts are not codebase findings or line-reduction credit unless cleanup is explicitly requested. Breadth comes from bounded inventory/search—not reading every file. Text search discovers candidates; it does not prove semantic usage. Before calling a symbol/interface/helper dead, duplicated, single-implementation, or removable, use LSP `references`, `definition`, and/or `implementation` at the candidate token. Rank candidates, inspect at most 20 strongest files with ranged `read`, require concrete evidence per finding, return at most 10 highest-impact findings, and stop. Do not retry a harness grep with bash because of a malformed multi-path argument; use one common ancestor or separate bounded calls. Do not repeat equivalent searches or read whole large files when a matching range is enough.

## Verification ladder

Stop at the first set of checks that convincingly covers the change. Run one well-formed verification chain, preserve its exit status, and do not repeat equivalent checks, reread unchanged files, or continue editing after it passes:

1. `diagnose` changed files.
2. Focused formatter/linter/type-check for changed scope.
3. Nearest unit test or package test.
4. Wider build/test only when impact crosses package boundaries or the user requests it.
5. Run `git status --short` once, then inspect changes correctly:
   - tracked files: `git diff --check` and `git diff -- <files>`;
   - untracked files (`??`): `git diff` will always be empty, so inspect each once with `read` or `git diff --no-index /dev/null <file>`;
   - do not retry an empty tracked diff for an untracked or ignored path.

Security, data-loss, migration, concurrency, and public API changes require stronger checks; do not simplify those away.

## Common workflows

### Implement a feature request

Before editing, derive the smallest observable acceptance behavior and choose the existing layer that owns the public API. Use symbols/references to confirm that boundary.

- First ask whether existing behavior/API already covers the request; reuse or delete before adding.
- Add one API per required operation at the owning boundary; keep lower-layer helpers private/minimal where the language permits.
- Prefer stdlib/native/existing dependencies before local machinery or a new package.
- Do not add aliases, compatibility wrappers, alternate names, configuration, or extension points unless an existing caller/convention requires them.
- Prefer one coherent implementation pass: source + focused tests, then diagnose and verify.
- Once acceptance tests pass and the requested behavior is present, stop. Do not reopen the design to add speculative convenience APIs.

### Fix a diagnostic

```text
diagnose src/file.ts
lsp hover/definition at failing symbol
read src/file.ts
edit src/file.ts
 diagnose src/file.ts
bash focused-test-or-typecheck
bash git diff --check && git diff -- src/file.ts
```

### Understand unfamiliar code

```text
workspace/symbol query=Concept
definition + typeDefinition/implementation
references
read only relevant definitions and tests
```

### Add or change behavior

```text
documentSymbol target file
hover/signatureHelp around API
references if shared
read implementation + nearest tests
edit
 diagnose changed files
run focused tests
inspect diff
```

### Commit

Before committing: diagnostics clean for changed supported files, focused verification passes, and status/diff contains no accidental/generated/secret files. Check status before diff so new untracked files are reviewed rather than mistaken for an empty change. Then use the repository's existing commit conventions.

## Troubleshooting

| Symptom | Action |
|---|---|
| `fetch failed` | Server unreachable; check URL/container |
| LSP unsupported | Check `lsp_servers`, then use text tools |
| LSP empty on known symbol | Confirm token position, retry after warmup, then fallback |
| LSP slow first time | Wait for indexing; subsequent requests should be warm |
| Diagnostics clean but build fails | Trust the build; diagnostics are not full verification |
| Package tool succeeds but binary absent | Retry package operation/check profile, then verify PATH |
| Background command has no output | Check `status` then `logs` |
