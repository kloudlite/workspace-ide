import { Type } from "@sinclair/typebox";
import { defineTool } from "@earendil-works/pi-coding-agent";
import { readFile } from "fs/promises";
import { tmpdir } from "os";
import { join, resolve, sep } from "path";

export interface WsConfig {
  /** HTTP server URL (e.g. "http://localhost:8321") */
  serverUrl: string;
  /** Locally discovered skill directories; read-only exception to remote file access. */
  localSkillDirs?: string[];
}

function localPath(path: string): string {
  return path.includes("/") ? path : join(tmpdir(), path);
}

function rangedText(content: string, offset = 1, limit = 400) {
  const lines = content.split(/\r?\n/);
  const start = Math.max(1, offset);
  const selected = lines.slice(start - 1, start - 1 + limit);
  const truncated = start - 1 + selected.length < lines.length;
  const suffix = truncated ? `\n[lines ${start}-${start + selected.length - 1} of ${lines.length}; continue with offset=${start + selected.length}]` : "";
  return { text: selected.join("\n") + suffix, lines: selected.length, totalLines: lines.length, truncated };
}

function boundedOutput(text: string, limit = 50_000): string {
  if (text.length <= limit) return text;
  const half = Math.floor(limit / 2);
  return `${text.slice(0, half)}\n[truncated ${text.length - limit} characters]\n${text.slice(-half)}`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

function formatProcessOutput(stdout = "", stderr = "", footer?: string): string {
  stdout = boundedOutput(stdout);
  stderr = boundedOutput(stderr);
  const sections = [stdout && (stderr ? `[stdout]\n${stdout}` : stdout), stderr && `[stderr]\n${stderr}`, footer].filter(Boolean);
  return sections.join("\n") || "(no output)";
}

function compactLspResult(result: any): any {
  if (Array.isArray(result) && result.length > 200) {
    return { items: result.slice(0, 200), truncated: true, total: result.length, message: "Narrow the query before acting." };
  }
  if (Array.isArray(result?.items) && result.items.length > 200) {
    return { ...result, items: result.items.slice(0, 200), truncated: true, total: result.items.length };
  }
  const size = JSON.stringify(result).length;
  if (size > 1_000_000) {
    return { truncated: true, size, message: "LSP result too large to use safely; narrow the file, symbol, or range." };
  }
  return result;
}

const symbolKinds = ["", "file", "module", "namespace", "package", "class", "method", "property", "field", "constructor", "enum", "interface", "function", "variable", "constant", "string", "number", "boolean", "array", "object", "key", "null", "enum-member", "struct", "event", "operator", "type-parameter"];

function lspPath(uri = ""): string {
  try { return decodeURIComponent(uri.replace(/^file:\/\//, "")); } catch { return uri.replace(/^file:\/\//, ""); }
}

function lspLocation(item: any): string {
  const location = item?.location || item;
  const uri = location?.uri || item?.targetUri || "";
  const range = location?.range || item?.targetSelectionRange || item?.targetRange;
  if (!uri) return "";
  const start = range?.start;
  return `${lspPath(uri)}${start ? `:${start.line + 1}:${start.character + 1}` : ""}`;
}

function renderEdits(result: any): string[] {
  const lines: string[] = [];
  for (const [uri, edits] of Object.entries(result?.changes || {})) {
    for (const edit of edits as any[]) lines.push(`${lspLocation({ uri, range: edit.range })} → ${JSON.stringify(edit.newText)}`);
  }
  for (const change of result?.documentChanges || []) {
    const uri = change?.textDocument?.uri || change?.uri;
    for (const edit of change?.edits || []) lines.push(`${lspLocation({ uri, range: edit.range })} → ${JSON.stringify(edit.newText)}`);
  }
  return lines;
}

function formatLspResult(method: string, raw: any): string {
  const result = compactLspResult(raw);
  if (result == null) return "(none)";
  const array = Array.isArray(result) ? result : Array.isArray(result.items) ? result.items : null;
  const footer = result?.truncated ? `\n[truncated: ${array?.length || 0} of ${result.total || "many"}; narrow the query]` : "";
  if (result?.truncated && !array && result.message) return `[truncated ${result.size || "large"}-character result] ${result.message}`;

  if (method === "textDocument/documentSymbol" || method === "workspace/symbol") {
    const walk = (items: any[], depth = 0): string[] => items.flatMap((item) => {
      const range = item.selectionRange || item.range;
      const location = lspLocation(item) || (range ? `${range.start.line + 1}:${range.start.character + 1}` : "");
      const line = `${"  ".repeat(depth)}${item.name} [${symbolKinds[item.kind] || `kind-${item.kind}`}]${location ? ` — ${location}` : ""}`;
      return [line, ...walk(item.children || [], depth + 1)];
    });
    return (walk(array || []).join("\n") || "(none)") + footer;
  }
  if (["textDocument/definition", "textDocument/typeDefinition", "textDocument/implementation", "textDocument/references"].includes(method)) {
    return ((array || [result]).map(lspLocation).filter(Boolean).join("\n") || "(none)") + footer;
  }
  if (method === "textDocument/hover") {
    const contents = result.contents;
    if (typeof contents === "string") return contents;
    if (typeof contents?.value === "string") return contents.value;
    if (Array.isArray(contents)) return contents.map((x: any) => typeof x === "string" ? x : x.value || JSON.stringify(x)).join("\n");
  }
  if (method === "textDocument/signatureHelp") {
    return (result.signatures || []).map((s: any, i: number) => `${i === result.activeSignature ? "*" : "-"} ${s.label}${s.documentation?.value ? `\n  ${s.documentation.value}` : ""}`).join("\n") || "(none)";
  }
  if (method === "textDocument/completion") {
    return (array || []).map((item: any) => `${item.label}${item.detail ? ` — ${item.detail}` : ""}`).join("\n") + footer || "(none)";
  }
  if (method === "textDocument/rename" || method === "textDocument/formatting") {
    const edits = method === "textDocument/formatting" ? (array || []).map((edit: any) => `${edit.range.start.line + 1}:${edit.range.start.character + 1} → ${JSON.stringify(edit.newText)}`) : renderEdits(result);
    return edits.join("\n") || "(none)";
  }
  if (method === "textDocument/codeAction") {
    const actions = (array || []).flatMap((action: any, i: number) => [
      `${i + 1}. ${action.title}${action.kind ? ` [${action.kind}]` : ""}`,
      ...renderEdits(action.edit).map((edit) => `   ${edit}`),
    ]);
    return actions.join("\n") + footer || "(none)";
  }
  if (method === "textDocument/prepareRename") {
    const range = result.range || result;
    return `${result.placeholder || "renameable"} — ${range.start.line + 1}:${range.start.character + 1}-${range.end.line + 1}:${range.end.character + 1}`;
  }
  return JSON.stringify(result, null, 2);
}

function textComponent(text: string) {
  return { render: () => text.split("\n"), invalidate: () => {} };
}

function toolCallSummary(name: string, args: any): string {
  switch (name) {
    case "read": return `read ${args.path}${args.offset ? `:${args.offset}` : ""}`;
    case "bash": return `$ ${args.command}`;
    case "edit": return `edit ${args.path}`;
    case "write": return `write ${args.path}`;
    case "upload": return `upload ${args.local_path} → ${args.remote_path}`;
    case "grep": return `grep ${JSON.stringify(args.pattern)} ${args.path || "."}`;
    case "find": return `find ${args.path}${args.name ? ` -name ${args.name}` : ""}`;
    case "ls": return `ls ${args.path}`;
    case "spawn": return `spawn ${args.command}`;
    case "logs": case "status": case "kill": return `${name} ${args.session_id}`;
    case "sessions": return "sessions";
    case "lsp": return `lsp ${args.method} ${args.path}${args.line != null ? `:${args.line + 1}:${(args.column || 0) + 1}` : ""}`;
    case "diagnose": return `diagnose ${args.path}`;
    case "lsp_servers": case "lsp_sessions": return name;
    case "pkg_install": case "pkg_remove": return `${name} ${args.package}`;
    case "pkg_search": return `pkg_search ${JSON.stringify(args.query)}`;
    case "pkg_list": return "pkg_list";
    default: return name;
  }
}

function toolResultText(result: any): string {
  return (result.content || []).map((item: any) => item.type === "text" ? item.text : `[${item.type}]`).join("\n") || "(no output)";
}

function collapsedToolResult(name: string, args: any, result: any, text: string): string {
  const details = result.details || {};
  if (name === "read") return "";
  if (text === "(no diagnostics)" || text === "(no output)") return text;

  const lines = text.split("\n");
  const limit = name === "grep" ? 15 : ["find", "ls", "sessions", "lsp_servers", "lsp_sessions", "pkg_search", "pkg_list"].includes(name) ? 20 : 10;
  if (lines.length <= limit && text.length <= 800) return text;

  const tail = name === "bash" || name === "logs";
  const preview = tail ? lines.slice(-limit) : lines.slice(0, limit);
  const omitted = lines.length - preview.length;
  const position = tail ? "earlier" : "more";
  const truncated = details.truncated || text.includes("[truncated");
  return `${preview.join("\n")}\n… (${omitted} ${position} lines${truncated ? ", output truncated" : ""}; expand to view)`;
}

function renderedTools(tools: any[]) {
  return tools.map((tool) => {
    // Pi's built-in read renderer only formats the result; unlike edit, it never reads cwd.
    if (tool.name === "read") return tool;
    return {
      ...tool,
      renderCall: (args: any) => textComponent(toolCallSummary(tool.name, args)),
      renderResult: (result: any, options: any, _theme: any, context: any) => {
        const text = toolResultText(result);
        return textComponent(options.expanded || context.isError ? text : collapsedToolResult(tool.name, context.args, result, text));
      },
    };
  });
}

function postJson(url: string, body: unknown, signal?: AbortSignal): Promise<any> {
  return fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  }).then(async (r) => {
    if (!r.ok) {
      const text = await r.text();
      throw new Error(`${r.status} ${r.statusText}: ${text}`);
    }
    return r.json();
  });
}

export function createWsTools(config: WsConfig) {
  const base = config.serverUrl.replace(/\/+$/, "");

  return renderedTools([
    // --- Core file ops ---
    defineTool({
      name: "read",
      label: "Read",
      description: "Read file text, defaulting to at most 400 lines. Use offset/limit to continue large files. Use LSP first for symbol intelligence.",
      parameters: Type.Object({
        path: Type.String({ description: "File path" }),
        offset: Type.Optional(Type.Number({ description: "First line, 1-indexed (default 1)" })),
        limit: Type.Optional(Type.Number({ description: "Maximum lines (default 400)" })),
      }),
      execute: async (_id, params: { path: string; offset?: number; limit?: number }, signal) => {
        const path = resolve(params.path);
        if (config.localSkillDirs?.some((dir) => path === resolve(dir) || path.startsWith(resolve(dir) + sep))) {
          const content = await readFile(path, "utf8");
          const ranged = rangedText(content, params.offset, params.limit);
          return { content: [{ type: "text", text: ranged.text }], details: { size: content.length, skill: true, ...ranged } };
        }
        const r: any = await postJson(`${base}/read`, { path: params.path, offset: params.offset ?? 1, limit: params.limit ?? 400 }, signal);
        const suffix = r.truncated ? `\n[lines ${r.offset}-${r.offset + r.lines - 1} of ${r.total_lines}; continue with offset=${r.offset + r.lines}]` : "";
        return { content: [{ type: "text", text: r.content + suffix }], details: { size: r.size, skill: false, lines: r.lines, totalLines: r.total_lines, truncated: r.truncated } };
      },
    }),
    defineTool({
      name: "bash",
      label: "Bash",
      description: "Run a finite shell command remotely. Never mask verification with || true. Check git status before diff: git diff omits untracked files. Use grep tool for searches and spawn for persistent commands.",
      parameters: Type.Object({
        command: Type.String({ description: "Command to run" }),
      }),
      execute: async (_id, params: { command: string }, signal) => {
        const r: any = await postJson(`${base}/bash`, { command: params.command }, signal);
        const text = formatProcessOutput(r.stdout, r.stderr, `[exit ${r.exit_code}]`);
        return {
          content: [{ type: "text", text }],
          details: { exitCode: r.exit_code, truncated: text.includes("[truncated ") },
        };
      },
    }),
    defineTool({
      name: "edit",
      label: "Edit",
      description: "Edit a file on the remote workspace by replacing exact text. After editing code/config, run diagnose on the changed file.",
      parameters: Type.Object({
        path: Type.String({ description: "File path" }),
        oldText: Type.String({ description: "Exact text to replace" }),
        newText: Type.String({ description: "Replacement text" }),
      }),
      execute: async (_id, params: { path: string; oldText: string; newText: string }, signal) => {
        const r: any = await postJson(`${base}/edit`, {
          path: params.path,
          edits: [{ old_text: params.oldText, new_text: params.newText }],
        }, signal);
        const errors = r.errors?.length ? `\n${r.errors.map((e: string) => `! ${e}`).join("\n")}` : "";
        return { content: [{ type: "text", text: `updated ${params.path} — ${r.applied} replacement${r.applied === 1 ? "" : "s"}${errors}` }], details: { applied: r.applied } };
      },
    }),
    defineTool({
      name: "write",
      label: "Write",
      description: "Write content to a file on the remote workspace (creates parent dirs). After writing code/config, run diagnose on the changed file.",
      parameters: Type.Object({
        path: Type.String({ description: "File path" }),
        content: Type.String({ description: "Content to write" }),
      }),
      execute: async (_id, params: { path: string; content: string }, signal) => {
        const r: any = await postJson(`${base}/write`, { path: params.path, content: params.content }, signal);
        return { content: [{ type: "text", text: `wrote ${params.path} — ${formatBytes(r.size)}` }], details: { size: r.size } };
      },
    }),
    defineTool({
      name: "upload",
      label: "Upload",
      description: "Upload a local file to the remote workspace. For pasted images, pass the displayed pi-clipboard filename as local_path.",
      parameters: Type.Object({
        local_path: Type.String({ description: "Local file path" }),
        remote_path: Type.String({ description: "Remote destination path" }),
      }),
      execute: async (_id, params: { local_path: string; remote_path: string }, signal) => {
        const resp = await fetch(`${base}/upload`, {
          method: "POST",
          headers: { "x-ws-path": params.remote_path },
          body: await readFile(localPath(params.local_path)),
          signal,
        });
        if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}: ${await resp.text()}`);
        const r: any = await resp.json();
        return { content: [{ type: "text", text: `uploaded ${params.local_path} → ${params.remote_path} — ${formatBytes(r.size)}` }], details: { size: r.size } };
      },
    }),

    // --- Search ---
    defineTool({
      name: "grep",
      label: "Grep",
      description: "Search file text using POSIX basic regex in one directory path. Do not pass space-separated paths; use a common ancestor or separate calls. Use LSP for semantic references.",
      parameters: Type.Object({
        pattern: Type.String({ description: "Search pattern" }),
        path: Type.Optional(Type.String({ description: "Directory to search (default: cwd)" })),
      }),
      execute: async (_id, params: { pattern: string; path?: string }, signal) => {
        const body: Record<string, string> = { pattern: params.pattern, ...(params.path ? { path: params.path } : {}) };
        const r: any = await postJson(`${base}/grep`, body, signal);
        const matches = (r.matches || []).map((m: any) => `${m.path}:${m.line_number}: ${m.text}`).join("\n");
        const text = (matches || "(no matches)") + (r.truncated ? "\n[truncated at 200 matches; narrow path or pattern]" : "");
        return { content: [{ type: "text", text }], details: {} };
      },
    }),
    defineTool({
      name: "find",
      label: "Find",
      description: "Find files matching a name pattern on the remote workspace",
      parameters: Type.Object({
        path: Type.String({ description: "Directory to search" }),
        name: Type.Optional(Type.String({ description: "Filename pattern (e.g. *.rs)" })),
      }),
      execute: async (_id, params: { path: string; name?: string }, signal) => {
        const body: Record<string, string> = { path: params.path, ...(params.name ? { name: params.name } : {}) };
        const r: any = await postJson(`${base}/find`, body, signal);
        const files = (r.files || []).join("\n");
        const text = (files || "(no files)") + (r.truncated ? "\n[truncated at 200 files; narrow path or pattern]" : "");
        return { content: [{ type: "text", text }], details: {} };
      },
    }),
    defineTool({
      name: "ls",
      label: "List",
      description: "List directory contents on the remote workspace",
      parameters: Type.Object({ path: Type.String({ description: "Directory path" }) }),
      execute: async (_id, params: { path: string }, signal) => {
        const r: any = await postJson(`${base}/ls`, { path: params.path }, signal);
        const text = (r.entries || []).map((e: any) => e.is_dir ? `${e.name}/` : `${e.name}  ${formatBytes(e.size)}`).join("\n") || "(empty directory)";
        return { content: [{ type: "text", text }], details: {} };
      },
    }),

    // --- Background processes ---
    defineTool({
      name: "spawn",
      label: "Spawn",
      description: "Start a long-running command in the background on the remote workspace",
      parameters: Type.Object({ command: Type.String({ description: "Command to run" }) }),
      execute: async (_id, params: { command: string }, signal) => {
        const r: any = await postJson(`${base}/spawn`, { command: params.command }, signal);
        return { content: [{ type: "text", text: `started ${r.session_id}\npid ${r.pid}\n${params.command}` }], details: { sessionId: r.session_id, pid: r.pid } };
      },
    }),
    defineTool({
      name: "logs",
      label: "Logs",
      description: "Get stdout/stderr from a background session on the remote workspace",
      parameters: Type.Object({ session_id: Type.String({ description: "Session ID from spawn" }) }),
      execute: async (_id, params: { session_id: string }, signal) => {
        const r: any = await postJson(`${base}/logs`, { session_id: params.session_id }, signal);
        const state = r.running ? "running" : `done, exit ${r.exit_code ?? "unknown"}`;
        const text = formatProcessOutput(r.stdout, r.stderr, `[${state}]`);
        return { content: [{ type: "text", text }], details: { running: r.running, exitCode: r.exit_code } };
      },
    }),
    defineTool({
      name: "status",
      label: "Status",
      description: "Check status of a background session on the remote workspace",
      parameters: Type.Object({ session_id: Type.String({ description: "Session ID from spawn" }) }),
      execute: async (_id, params: { session_id: string }, signal) => {
        const r: any = await postJson(`${base}/status`, { session_id: params.session_id }, signal);
        const text = `${params.session_id}\n${r.running ? `running — pid ${r.pid}` : `done — exit ${r.exit_code ?? "unknown"}`}\n${r.command}`;
        return { content: [{ type: "text", text }], details: { running: r.running, exitCode: r.exit_code } };
      },
    }),
    defineTool({
      name: "kill",
      label: "Kill",
      description: "Kill a background session on the remote workspace",
      parameters: Type.Object({ session_id: Type.String({ description: "Session ID from spawn" }) }),
      execute: async (_id, params: { session_id: string }, signal) => {
        const r: any = await postJson(`${base}/kill`, { session_id: params.session_id }, signal);
        return { content: [{ type: "text", text: `${params.session_id} — ${r.killed ? "killed" : r.message || "not running"}` }], details: { killed: r.killed } };
      },
    }),
    defineTool({
      name: "sessions",
      label: "Sessions",
      description: "List all background sessions on the remote workspace",
      parameters: Type.Object({}),
      execute: async (_id, _params: {}, signal) => {
        const resp = await fetch(`${base}/sessions`, { signal });
        const r = await resp.json();
        const text = (r || []).map((s: any) =>
          `${s.session_id}\n  ${s.running ? `running, pid ${s.pid}` : `done, exit ${s.exit_code ?? "unknown"}`}\n  ${s.command}`
        ).join("\n") || "(no background sessions)";
        return { content: [{ type: "text", text }], details: {} };
      },
    }),

    // --- LSP ---
    defineTool({
      name: "lsp_servers",
      label: "LSP Servers",
      description: "List available LSP servers and supported file extensions",
      parameters: Type.Object({}),
      execute: async (_id, _params: {}, signal) => {
        const resp = await fetch(`${base}/lsp/servers`, { signal });
        if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}: ${await resp.text()}`);
        const r = await resp.json();
        const text = (r || []).map((s: any) => `${s.id} [${s.language_id}, ${s.root_mode}]\n  ${s.extensions.join(" ")}\n  ${s.binary}`).join("\n") || "(no LSP servers)";
        return { content: [{ type: "text", text }], details: {} };
      },
    }),
    defineTool({
      name: "lsp_sessions",
      label: "LSP Sessions",
      description: "List running LSP server sessions on the remote workspace",
      parameters: Type.Object({}),
      execute: async (_id, _params: {}, signal) => {
        const resp = await fetch(`${base}/lsp/sessions`, { signal });
        if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}: ${await resp.text()}`);
        const r = await resp.json();
        const text = (r || []).map(([server, root]: [string, string]) => `${server}\n  ${root}`).join("\n") || "(no running LSP sessions)";
        return { content: [{ type: "text", text }], details: {} };
      },
    }),
    defineTool({
      name: "lsp",
      label: "LSP",
      description: "Primary code-intelligence tool. Navigate symbols, inspect types/signatures, find usages, discover symbols, preview semantic renames/code actions, request completions, or obtain formatting edits. Prefer this over text search for supported code.",
      parameters: Type.Object({
        method: Type.Union([
          Type.Literal("textDocument/hover"),
          Type.Literal("textDocument/definition"),
          Type.Literal("textDocument/typeDefinition"),
          Type.Literal("textDocument/implementation"),
          Type.Literal("textDocument/references"),
          Type.Literal("textDocument/completion"),
          Type.Literal("textDocument/signatureHelp"),
          Type.Literal("textDocument/documentSymbol"),
          Type.Literal("workspace/symbol"),
          Type.Literal("textDocument/prepareRename"),
          Type.Literal("textDocument/rename"),
          Type.Literal("textDocument/codeAction"),
          Type.Literal("textDocument/formatting"),
        ], { description: "LSP method" }),
        path: Type.String({ description: "File path used to select the language server" }),
        line: Type.Optional(Type.Number({ description: "Start line (0-indexed); required for position methods" })),
        column: Type.Optional(Type.Number({ description: "Start column (0-indexed); required for position methods" })),
        end_line: Type.Optional(Type.Number({ description: "End line for codeAction range" })),
        end_column: Type.Optional(Type.Number({ description: "End column for codeAction range" })),
        query: Type.Optional(Type.String({ description: "Query for workspace/symbol" })),
        new_name: Type.Optional(Type.String({ description: "New name for rename preview" })),
        tab_size: Type.Optional(Type.Number({ description: "Formatting tab size (default 4)" })),
        insert_spaces: Type.Optional(Type.Boolean({ description: "Formatting uses spaces (default true)" })),
      }),
      execute: async (_id, params: Record<string, any>, signal) => {
        const r: any = await postJson(`${base}/lsp/request`, params, signal);
        const text = formatLspResult(params.method, r.result ?? r);
        return { content: [{ type: "text", text }], details: {} };
      },
    }),
    defineTool({
      name: "diagnose",
      label: "Diagnose",
      description: "Get LSP diagnostics (errors, warnings) for a file on the remote workspace",
      parameters: Type.Object({ path: Type.String({ description: "File path" }) }),
      execute: async (_id, params: { path: string }, signal) => {
        const r: any = await postJson(`${base}/lsp/diagnose`, { path: params.path }, signal);
        const severity = ["", "error", "warning", "info", "hint"];
        const text = (r || []).map((d: any) =>
          `${d.path}:${d.line + 1}:${d.column + 1} — ${severity[d.severity] || `severity-${d.severity}`}: ${d.message}${d.code ? ` [${d.code}]` : ""}`).join("\n") || "(no diagnostics)";
        return { content: [{ type: "text", text }], details: {} };
      },
    }),

    // --- Package management ---
    defineTool({
      name: "pkg_install",
      label: "Pkg Install",
      description: "Install a package on the remote workspace. Use this instead of raw package-manager commands.",
      parameters: Type.Object({
        package: Type.String({ description: "Package name (e.g. go, nodejs, python3, gcc)" }),
      }),
      execute: async (_id, params: { package: string }, signal, onUpdate) => {
        onUpdate?.({ content: [{ type: "text", text: `installing ${params.package}… this can take a few minutes` }], details: { running: true } });
        const r: any = await postJson(`${base}/pkg/install`, { package: params.package }, signal);
        return { content: [{ type: "text", text: r.ok ? `installed ${params.package}` : `failed ${params.package}: ${r.error || "unknown error"}` }], details: { running: false, ok: r.ok } };
      },
      renderCall: (args) => ({ render: () => [`pkg_install: ${args.package}`], invalidate: () => {} }),
    }),
    defineTool({
      name: "pkg_search",
      label: "Pkg Search",
      description: "Search for available packages on the remote workspace",
      parameters: Type.Object({ query: Type.String({ description: "Search query" }) }),
      execute: async (_id, params: { query: string }, signal) => {
        const r: any = await postJson(`${base}/pkg/search`, { query: params.query }, signal);
        const text = (r.packages || []).join("\n") || `(no packages matching ${JSON.stringify(params.query)})`;
        return { content: [{ type: "text", text }], details: {} };
      },
    }),
    defineTool({
      name: "pkg_list",
      label: "Pkg List",
      description: "List installed packages on the remote workspace",
      parameters: Type.Object({}),
      execute: async (_id, _params: {}, signal) => {
        const r: any = await postJson(`${base}/pkg/list`, {}, signal);
        const text = (r.packages || []).join("\n") || "(no installed packages)";
        return { content: [{ type: "text", text }], details: {} };
      },
    }),
    defineTool({
      name: "pkg_remove",
      label: "Pkg Remove",
      description: "Uninstall a package from the remote workspace. Use this instead of raw package-manager commands.",
      parameters: Type.Object({ package: Type.String({ description: "Package name to uninstall" }) }),
      execute: async (_id, params: { package: string }, signal, onUpdate) => {
        onUpdate?.({ content: [{ type: "text", text: `removing ${params.package}… this can take a few minutes` }], details: { running: true } });
        const r: any = await postJson(`${base}/pkg/remove`, { package: params.package }, signal);
        return { content: [{ type: "text", text: r.ok ? `removed ${params.package}` : `failed ${params.package}: ${r.error || "unknown error"}` }], details: { running: false, ok: r.ok } };
      },
      renderCall: (args) => ({ render: () => [`pkg_remove: ${args.package}`], invalidate: () => {} }),
    }),
  ]);
}
