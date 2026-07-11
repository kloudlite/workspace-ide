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

// ponytail: resolve skill/extension dirs relative to this file via URL (no path/url imports needed)
const distDir = new URL(".", import.meta.url).pathname; // dist/
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

if (!serverUrl) serverUrl = "http://localhost:8321";

// ponytail: hash server URL into a safe dir name — different connections get isolated sessions
const sessionDir = join(homedir(), ".ws-sessions", createHash("sha256").update(serverUrl).digest("hex").slice(0, 12));

async function main() {
  // ponytail: extension reads WS_SERVER_URL from env
  process.env.WS_SERVER_URL = serverUrl;
  // --list
  if (args.includes("--list")) {
    const sessions = await SessionManager.list(homedir(), sessionDir);
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
    sm = SessionManager.open(sessionFile, sessionDir, homedir());
  } else if (newSession) {
    sm = SessionManager.create(homedir(), sessionDir);
  } else {
    sm = SessionManager.continueRecent(homedir(), sessionDir);
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
        additionalExtensionPaths: [distDir],
        appendSystemPrompt: [
          "A user message wrapped in <skill name=\"...\"> is an explicit /skill invocation. Immediately execute the skill instructions against the current remote workspace. Do not merely acknowledge, say it was loaded, ask the user to invoke it again, or wait for a second request. Skill arguments, when present, appear after the skill content as User: ...",
          "For repository-wide audits/reviews, scan breadth through bounded inventories/searches, then inspect only the strongest candidate files with ranged read offset/limit. Do not ingest the repository. Avoid repeated equivalent searches, require concrete evidence for every finding, inspect at most 20 candidate files, report at most 10 highest-impact findings, then stop.",
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
    cwd: "/workspace",
    agentDir: getAgentDir(),
    sessionManager: sm,
  });

  if (prompts.length === 0) {
    const mode = new InteractiveMode(runtime);
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
