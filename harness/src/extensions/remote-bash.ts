/**
 * ws-harness remote extension
 * - Routes !commands to the remote ws server
 * - Routes @ file autocomplete to remote filesystem
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const serverUrl = process.env.WS_SERVER_URL;

// ── Bash: intercept !commands ──

function createRemoteBash() {
  return {
    operations: {
      exec: (command: string, _cwd: string, { onData, signal, timeout }: any) => {
        return new Promise<any>(async (resolve, reject) => {
          const controller = new AbortController();
          const timer = timeout
            ? setTimeout(() => controller.abort(), timeout * 1000)
            : undefined;
          signal?.addEventListener?.("abort", () => controller.abort(), { once: true });

          try {
            const resp = await fetch(`${serverUrl}/bash`, {
              method: "POST",
              headers: { "content-type": "application/json" },
              body: JSON.stringify({ command }),
              signal: controller.signal,
            });
            if (timer) clearTimeout(timer);
            const result = await resp.json();
            if (result.stdout) onData?.(Buffer.from(result.stdout));
            if (result.stderr) onData?.(Buffer.from(result.stderr));
            resolve({ exitCode: result.exit_code ?? 0 });
          } catch (err: any) {
            if (timer) clearTimeout(timer);
            reject(err.name === "AbortError" ? new Error("aborted") : err);
          }
        });
      },
    },
  };
}

// ── Autocomplete: intercept @ file browsing ──

interface AutocompleteItem {
  value: string;
  label: string;
  description?: string;
}

interface AutocompleteProvider {
  triggerCharacters?: string[];
  getSuggestions(lines: string[], cursorLine: number, cursorCol: number, options: {
    signal: AbortSignal;
    force?: boolean;
  }): Promise<{ items: AutocompleteItem[]; prefix: string } | null>;
  applyCompletion(lines: string[], cursorLine: number, cursorCol: number, item: AutocompleteItem, prefix: string): {
    lines: string[];
    cursorLine: number;
    cursorCol: number;
  };
  shouldTriggerFileCompletion?(lines: string[], cursorLine: number, cursorCol: number): boolean;
}

function createRemoteAutocompleteProvider(inner: AutocompleteProvider): AutocompleteProvider {
  return {
    triggerCharacters: inner.triggerCharacters,

    shouldTriggerFileCompletion: inner.shouldTriggerFileCompletion,

    getSuggestions: async (lines, cursorLine, cursorCol, options) => {
      // Let the inner provider try first
      const original = await inner.getSuggestions(lines, cursorLine, cursorCol, options);
      if (!original) return null;

      // Only intercept @ file completions (prefix starts with @)
      if (!original.prefix.startsWith("@")) {
        return original;
      }

      // Query remote filesystem via ws server
      const searchTerm = original.prefix.slice(1); // strip @

      try {
        let items: AutocompleteItem[];

        if (searchTerm.length === 0) {
          // Just @ — list root of workspace
          const resp = await fetch(`${serverUrl}/ls`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ path: "/workspace" }),
            signal: options.signal,
          });
          const result = await resp.json();
          const entries: Array<{ name: string; is_dir: boolean }> = result.entries ?? [];
          items = entries.map((e) => ({
            value: e.is_dir ? `${e.name}/` : e.name,
            label: e.is_dir ? `${e.name}/` : e.name,
          }));
        } else {
          // @partial — find matching files
          const resp = await fetch(`${serverUrl}/find`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ path: "/workspace", name: `*${searchTerm}*` }),
            signal: options.signal,
          });
          const result = await resp.json();
          const paths: string[] = result.files ?? [];
          // Strip /workspace prefix for shorter display
          items = paths.map((p) => ({
            value: p.replace(/^\/workspace\//, ""),
            label: p.replace(/^\/workspace\//, ""),
          }));
        }

        return items.length > 0
          ? { items, prefix: original.prefix }
          : original;
      } catch {
        return original; // fall back to local on error
      }
    },

    applyCompletion: (lines, cursorLine, cursorCol, item, prefix) => {
      // Insert the full remote path after stripping @ prefix from the current token
      const line = lines[cursorLine] || "";
      const beforeCursor = line.slice(0, cursorCol);
      const afterCursor = line.slice(cursorCol);
      const atIdx = beforeCursor.lastIndexOf("@");
      if (atIdx === -1) return { lines, cursorLine, cursorCol };

      const newLine = beforeCursor.slice(0, atIdx) + item.value + afterCursor;
      lines[cursorLine] = newLine;
      return { lines, cursorLine, cursorCol: atIdx + item.value.length };
    },
  };
}

// ── Clipboard image paste: intercept local temp paths, inject as base64 ──

// ponytail: sync ops for file reads at submit time; images are small
import { readFileSync } from "fs";

const CLIPBOARD_PATTERN = /\S*pi-clipboard-\S+\.(png|jpg|jpeg|gif|webp)\b/i;
const CLIPBOARD_PATTERN_GLOBAL = /\S*pi-clipboard-\S+\.(png|jpg|jpeg|gif|webp)\b/gi;

function mimeTypeFor(ext: string): string {
  const m: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp" };
  return m[ext.toLowerCase()] ?? "image/png";
}

function handleClipboardImages(text: string, existingImages: any[]): { text: string; images: any[] } {
  const matches = text.matchAll(CLIPBOARD_PATTERN_GLOBAL);
  let cleaned = text;
  const newImages = [...(existingImages ?? [])];

  for (const match of matches) {
    const localPath = match[0];
    const ext = (match[1] || "png").toLowerCase();

    try {
      const data = readFileSync(localPath);
      const base64 = data.toString("base64");
      newImages.push({ type: "image", data: base64, mimeType: mimeTypeFor(ext) });
      cleaned = cleaned.replace(localPath, ""); // remove local path from text
    } catch {
      // ponytail: file deleted/perm error — leave path in text, agent will fail gracefully
    }
  }

  return { text: cleaned.trim(), images: newImages };
}

// ── Extension ──

export default function (pi: ExtensionAPI) {
  if (!serverUrl) return;

  pi.on("user_bash", () => createRemoteBash());

  pi.on("session_start", async (_event, ctx) => {
    ctx.ui?.addAutocompleteProvider?.((current: AutocompleteProvider) =>
      createRemoteAutocompleteProvider(current)
    );
  });

  // ponytail: intercept submit to replace local clipboard image paths with base64
  pi.on("input", async (event) => {
    if (!CLIPBOARD_PATTERN.test(event.text)) return { action: "continue" as const };
    const { text, images } = handleClipboardImages(event.text, event.images ?? []);
    return { action: "transform" as const, text, images };
  });
}
