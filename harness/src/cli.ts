#!/usr/bin/env node
import { createHash } from "crypto";
import { homedir } from "os";
import { join } from "path";
import {
  createAgentSessionFromServices,
  createAgentSessionRuntime,
  createAgentSessionServices,
  getAgentDir,
  InteractiveMode,
  runPrintMode,
  SessionManager,
} from "@earendil-works/pi-coding-agent";
import { createWsTools } from "./index.js";

// ponytail: resolve bundled resources relative to this file
const extensionPath = new URL("./extensions/remote-bash.js", import.meta.url).pathname;
const skillDir = new URL("../skills/ws-harness", import.meta.url).pathname;

// ponytail: CLI parsing by hand — no clap/yargs dep
const args = process.argv.slice(2);
let serverUrl = process.env.WS_SERVER_URL || "";
let newSession = false;
let sessionFile: string | undefined;
const prompts: string[] = [];

for (let i = 0; i < args.length; i++) {
  if (args[i] === "--server" && i + 1 < args.length) {
    serverUrl = args[++i];
  } else if (args[i] === "--new") {
    newSession = true;
  } else if (args[i] === "--session" && i + 1 < args.length) {
    sessionFile = args[++i];
  } else if (args[i] === "--list") {
    // handled before main
  } else if (args[i] === "--help" || args[i] === "-h") {
    console.log(`usage: ws-pi [--server <url>] [--new] [--session <path>] [--list] [<prompt>...]

  --server <url>   ws HTTP server URL (also WS_SERVER_URL env)
  --new            Start a fresh session (default: continue most recent)
  --session <path> Open a specific session file
  --list           List sessions for this server connection
  With prompts:    single-shot (print mode)
  Without:         full interactive TUI
`);
    process.exit(0);
  } else {
    prompts.push(args[i]);
  }
}

if (!serverUrl) serverUrl = "http://kmac.khost.dev:18765";
if (!serverUrl.includes("://")) serverUrl = `http://${serverUrl}${serverUrl.includes(":") ? "" : ":18765"}`;
const serverLabel = serverUrl.replace(/^https?:\/\//, "").replace(/:18765$/, "");

// ponytail: hash normalized server URL into a safe dir name — different connections get isolated sessions
const sessionDir = join(homedir(), ".ws-sessions", createHash("sha256").update(serverUrl).digest("hex").slice(0, 12));
// Pi requires an existing local cwd for sessions and TUI links. Remote tools use /workspace server-side.
const localCwd = process.cwd();
const remoteCwd = "/workspace";

function showRemoteWorkspace(mode: InteractiveMode) {
  const footer = (mode as any).footer;
  const render = footer.render.bind(footer);
  footer.render = (width: number) => {
    const lines = render(width);
    const remote = `${serverUrl.replace(/^[a-z]+:\/\//i, "")}:${remoteCwd}`;
    const display = remote.length > width ? `${remote.slice(0, Math.max(0, width - 3))}...` : remote;
    // Preserve Pi's dim ANSI wrapper from the built-in cwd line.
    const plain = lines[0].replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "");
    lines[0] = lines[0].replace(plain, display);
    return lines;
  };
}

async function main() {
  // ponytail: extension reads WS_SERVER_URL from env
  process.env.WS_SERVER_URL = serverUrl;
  // --list
  if (args.includes("--list")) {
    const sessions = await SessionManager.listAll(sessionDir);
    if (sessions.length === 0) {
      console.log("No sessions found.");
    } else {
      for (const s of sessions) {
        const date = s.modified.toISOString().replace("T", " ").slice(0, 19);
        const label = s.name ? ` [${s.name}]` : "";
        console.log(`${date}  ${s.id.slice(0, 8)}  ${s.firstMessage.slice(0, 60)}${label}`);
      }
    }
    process.exit(0);
  }

  // Default: continue most recent session. --new forces fresh. --session opens a specific one.
  let sm: SessionManager;
  if (sessionFile) {
    sm = SessionManager.open(sessionFile, sessionDir, localCwd);
  } else if (newSession) {
    sm = SessionManager.create(localCwd, sessionDir);
  } else {
    const [recent] = await SessionManager.listAll(sessionDir);
    sm = recent ? SessionManager.open(recent.path, sessionDir, localCwd) : SessionManager.create(localCwd, sessionDir);
  }

  // Let user know which session is active
  if (!newSession && sm.getSessionFile()) {
    console.log(`session: ${sm.getSessionFile()}`);
  }

  const createRuntime = async ({
    cwd,
    sessionManager,
    sessionStartEvent,
  }: {
    cwd: string;
    sessionManager: SessionManager;
    sessionStartEvent?: any;
  }) => {
    const services = await createAgentSessionServices({
      cwd,
      resourceLoaderOptions: {
        noExtensions: true,
        additionalSkillPaths: [skillDir],
        additionalExtensionPaths: [extensionPath],
        appendSystemPrompt: [
          "A user message wrapped in <skill name=\"...\"> is an explicit /skill invocation. Immediately execute the skill instructions against the current remote workspace. Do not merely acknowledge, say it was loaded, ask the user to invoke it again, or wait for a second request. Skill arguments, when present, appear after the skill content as User: ...",
          "For repository-wide audits/reviews, scope the codebase to tracked source/config (`git ls-files`) by default; ignore generated, build, ignored, and untracked workspace artifacts unless cleanup is explicitly requested, and never count them in code-reduction totals. Use bounded text search only to discover candidates. Before claiming a code symbol/interface/helper is dead, duplicated, single-implementation, or removable, verify it semantically with LSP references/definition/implementation; grep counts are not proof. Inspect only the strongest candidates with ranged read offset/limit. Do not ingest the repository. Avoid repeated equivalent searches, require concrete evidence for every finding, inspect at most 20 candidate files, report at most 10 highest-impact findings, then stop.",
        ],
      },
    });
    const localSkillDirs = services.resourceLoader.getSkills().skills.map((skill) => skill.baseDir);
    return {
      ...(await createAgentSessionFromServices({
        services,
        sessionManager,
        sessionStartEvent,
        noTools: "builtin",
        customTools: createWsTools({ serverUrl, localSkillDirs }),
      })),
      services,
      diagnostics: services.diagnostics,
    };
  };

  const runtime = await createAgentSessionRuntime(createRuntime, {
    cwd: localCwd,
    agentDir: getAgentDir(),
    sessionManager: sm,
  });

  if (prompts.length === 0) {
    const write = process.stdout.write.bind(process.stdout);
    const resumable = Boolean(sm.getSessionFile());
    let wroteResume = false;
    process.stdout.write = ((chunk: any, ...rest: any[]) => {
      if (String(chunk).replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "").startsWith("To resume this session:")) {
        wroteResume = true;
        return resumable ? write(`To continue: ws-pi --server ${serverLabel}\n`) : true;
      }
      return write(chunk, ...rest);
    }) as typeof process.stdout.write;
    process.once("exit", () => {
      if (resumable && !wroteResume) write(`To continue: ws-pi --server ${serverLabel}\n`);
    });
    const mode = new InteractiveMode(runtime);
    showRemoteWorkspace(mode);
    await mode.run();
  } else {
    const exitCode = await runPrintMode(runtime, {
      mode: "text",
      initialMessage: prompts.join(" "),
    });
    process.exit(exitCode);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
