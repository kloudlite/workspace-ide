import { Type } from "@sinclair/typebox";
import { defineTool } from "@earendil-works/pi-coding-agent";
import { readFile } from "fs/promises";
import { tmpdir } from "os";
import { join } from "path";

export interface WsConfig {
  /** HTTP server URL (e.g. "http://localhost:8321") */
  serverUrl: string;
}

function localPath(path: string): string {
  return path.includes("/") ? path : join(tmpdir(), path);
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

  return [
    // --- Core file ops ---
    defineTool({
      name: "read",
      label: "Read",
      description: "Read file text from the remote workspace. Do NOT use for symbol/type/function meaning, hover, definition, references, or completion when LSP supports the file; use lsp first for code intelligence.",
      parameters: Type.Object({ path: Type.String({ description: "File path" }) }),
      execute: async (_id, params: { path: string }, signal) => {
        const r: any = await postJson(`${base}/read`, { path: params.path }, signal);
        return { content: [{ type: "text", text: r.content }], details: { size: r.size } };
      },
    }),
    defineTool({
      name: "bash",
      label: "Bash",
      description: "Run a shell command on the remote workspace",
      parameters: Type.Object({
        command: Type.String({ description: "Command to run" }),
      }),
      execute: async (_id, params: { command: string }, signal) => {
        const r: any = await postJson(`${base}/bash`, { command: params.command }, signal);
        return {
          content: [{ type: "text", text: r.stdout || r.stderr }],
          details: { exitCode: r.exit_code, stdout: r.stdout, stderr: r.stderr },
        };
      },
    }),
    defineTool({
      name: "edit",
      label: "Edit",
      description: "Edit a file on the remote workspace by replacing exact text",
      parameters: Type.Object({
        path: Type.String({ description: "File path" }),
        oldText: Type.String({ description: "Exact text to replace" }),
        newText: Type.String({ description: "Replacement text" }),
      }),
      execute: async (_id, params: { path: string; oldText: string; newText: string }, signal) => {
        await postJson(`${base}/edit`, {
          path: params.path,
          edits: [{ oldText: params.oldText, newText: params.newText }],
        }, signal);
        return { content: [{ type: "text", text: "ok" }], details: {} };
      },
    }),
    defineTool({
      name: "write",
      label: "Write",
      description: "Write content to a file on the remote workspace (creates parent dirs)",
      parameters: Type.Object({
        path: Type.String({ description: "File path" }),
        content: Type.String({ description: "Content to write" }),
      }),
      execute: async (_id, params: { path: string; content: string }, signal) => {
        await postJson(`${base}/write`, { path: params.path, content: params.content }, signal);
        return { content: [{ type: "text", text: "ok" }], details: {} };
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
        return { content: [{ type: "text", text: `uploaded ${r.size} bytes` }], details: { size: r.size } };
      },
    }),

    // --- Search ---
    defineTool({
      name: "grep",
      label: "Grep",
      description: "Search file text on the remote workspace. Do NOT use for definition/references when LSP supports the file; use lsp first for code intelligence.",
      parameters: Type.Object({
        pattern: Type.String({ description: "Search pattern" }),
        path: Type.Optional(Type.String({ description: "Directory to search (default: cwd)" })),
      }),
      execute: async (_id, params: { pattern: string; path?: string }, signal) => {
        const body: Record<string, string> = { pattern: params.pattern, ...(params.path ? { path: params.path } : {}) };
        const r: any = await postJson(`${base}/grep`, body, signal);
        const text = (r.matches || []).map((m: any) => `${m.path}:${m.line_number}: ${m.text}`).join("\n");
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
        return { content: [{ type: "text", text: (r.files || []).join("\n") }], details: {} };
      },
    }),
    defineTool({
      name: "ls",
      label: "List",
      description: "List directory contents on the remote workspace",
      parameters: Type.Object({ path: Type.String({ description: "Directory path" }) }),
      execute: async (_id, params: { path: string }, signal) => {
        const r: any = await postJson(`${base}/ls`, { path: params.path }, signal);
        // ponytail: minimal listing — names with / for dirs, no JSON bloat
        const text = (r.entries || []).map((e: any) => e.is_dir ? `${e.name}/` : e.name).join("  ");
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
        return { content: [{ type: "text", text: `session: ${r.session_id}` }], details: {} };
      },
    }),
    defineTool({
      name: "logs",
      label: "Logs",
      description: "Get stdout/stderr from a background session on the remote workspace",
      parameters: Type.Object({ session_id: Type.String({ description: "Session ID from spawn" }) }),
      execute: async (_id, params: { session_id: string }, signal) => {
        const r: any = await postJson(`${base}/logs`, { session_id: params.session_id }, signal);
        const text = r.stdout || r.stderr || "(no output)";
        return { content: [{ type: "text", text }], details: {} };
      },
    }),
    defineTool({
      name: "status",
      label: "Status",
      description: "Check status of a background session on the remote workspace",
      parameters: Type.Object({ session_id: Type.String({ description: "Session ID from spawn" }) }),
      execute: async (_id, params: { session_id: string }, signal) => {
        const r: any = await postJson(`${base}/status`, { session_id: params.session_id }, signal);
        const text = r.running ? `running: ${r.command}` : `done (exit ${r.exit_code})`;
        return { content: [{ type: "text", text }], details: {} };
      },
    }),
    defineTool({
      name: "kill",
      label: "Kill",
      description: "Kill a background session on the remote workspace",
      parameters: Type.Object({ session_id: Type.String({ description: "Session ID from spawn" }) }),
      execute: async (_id, params: { session_id: string }, signal) => {
        const r: any = await postJson(`${base}/kill`, { session_id: params.session_id }, signal);
        return { content: [{ type: "text", text: r.killed ? "killed" : "not running" }], details: {} };
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
          `${s.session_id.slice(0, 8)}  ${s.running ? "running" : "done"}  ${s.command}`
        ).join("\n") || "(none)";
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
        const text = (r || []).map((s: any) => `${s.id} (${s.language_id}): ${s.extensions.join(", ")}`).join("\n") || "(none)";
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
        const text = (r || []).map(([server, root]: [string, string]) => `${server}  ${root}`).join("\n") || "(none)";
        return { content: [{ type: "text", text }], details: {} };
      },
    }),
    defineTool({
      name: "lsp",
      label: "LSP",
      description: "Preferred tool for code intelligence: symbol/type/function meaning, hover docs, definition, references, and completion. Use this before read/grep for supported code files.",
      parameters: Type.Object({
        method: Type.Union(
          [Type.Literal("textDocument/hover"), Type.Literal("textDocument/definition"),
           Type.Literal("textDocument/references"), Type.Literal("textDocument/completion")],
          { description: "LSP method" }
        ),
        path: Type.String({ description: "File path (relative or absolute)" }),
        line: Type.Number({ description: "Line number (0-indexed)" }),
        column: Type.Number({ description: "Column number (0-indexed)" }),
      }),
      execute: async (_id, params: { method: string; path: string; line: number; column: number }, signal) => {
        const r: any = await postJson(`${base}/lsp/request`, {
          method: params.method, path: params.path,
          line: params.line, column: params.column,
        }, signal);
        const text = JSON.stringify(r.result ?? r);
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
        const text = (r || []).map((d: any) =>
          `${d.range.start.line}:${d.range.start.character}: ${d.message}`).join("\n") || "(none)";
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
        return { content: [{ type: "text", text: r.ok ? `installed ${params.package}` : r.error || "failed" }], details: { running: false } };
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
        const text = (r.packages || []).join("\n") || "No results";
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
        const text = (r.packages || []).join("\n") || "(empty)";
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
        return { content: [{ type: "text", text: r.ok ? `removed ${params.package}` : r.error || "failed" }], details: { running: false } };
      },
      renderCall: (args) => ({ render: () => [`pkg_remove: ${args.package}`], invalidate: () => {} }),
    }),
  ];
}
