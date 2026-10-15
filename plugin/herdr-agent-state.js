// MiMo Code -> herdr custom-agent state reporter.
//
// Reports semantic lifecycle state over herdr's official custom-integration
// path (`herdr pane report-agent --source custom:...`, see
// https://herdr.dev/docs/add-herdr-support/) instead of the raw socket API,
// so MiMo Code shows up in the herdr agents sidebar as `mimo` without any
// process spoofing.
//
// Design notes (state aggregation and watchdog pattern follow
// https://github.com/junliu-mde/mimo-code-herdr-plugin, MIT):
//  - MiMo's event stream is noisy: busy fires repeatedly while the main
//    session runs and child sessions emit unguarded idle when they finish.
//    We aggregate (blocked > working-if-any-busy > idle) and report only on
//    real changes.
//  - MiMo never calls plugin dispose hooks and runs plugins in a worker
//    thread that quit terminate()s, so no in-process exit hook is reliable.
//    A detached watchdog holds our stdin pipe; the kernel closes it on ANY
//    death mode (quit, crash, SIGKILL), the watchdog then releases the label
//    with a seq just above the last report this process emitted, so it
//    outranks every report the dead process made while losing to reports
//    from any mimo started later. Seq updates travel over the pipe and the
//    watchdog persists a floor file the next process starts above.
//  - A mimo spawned as a tool of another agent (e.g. a Claude Code Bash
//    session) inherits the pane's HERDR_* env and would hijack that pane's
//    label; we stay silent when a parent-agent marker env is set.

import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SOURCE = "custom:mimo-herdr";
const AGENT = "mimo";
const WATCHDOG_BIN = "__MIMO_HERDR_BIN__"; // replaced at install time (JS-escaped)

// Parent-agent markers. CLAUDECODE is the documented Claude Code marker;
// CLAUDE_CODE_ENTRYPOINT is its CLI entrypoint companion. Both mean this
// mimo is a tool of another agent, not the pane's primary agent.
const PARENT_AGENT_ENVS = ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT"];

function isNestedUnderAnotherAgent() {
  return PARENT_AGENT_ENVS.some((k) => Boolean(process.env[k]));
}

// Debug logging: set MIMO_HERDR_DEBUG=1 in the pane env to trace reports.
// State lives under XDG_STATE_HOME (mode 0600), never /tmp.
const DEBUG = process.env.MIMO_HERDR_DEBUG === "1";
const DEBUG_LOG = path.join(stateHome(), "mimo-herdr", "plugin.log");

function stateHome() {
  const xdg = process.env.XDG_STATE_HOME;
  if (xdg && xdg.trim()) return xdg;
  return path.join(os.homedir(), ".local", "state");
}

function seqFloorPath() {
  return path.join(stateHome(), "mimo-herdr", "watch-seq");
}

function debugLog(...parts) {
  if (!DEBUG) return;
  try {
    fs.mkdirSync(path.dirname(DEBUG_LOG), { recursive: true, mode: 0o700 });
    fs.appendFileSync(
      DEBUG_LOG,
      `${new Date().toISOString()} ${parts.join(" ")}\n`,
      { mode: 0o600 },
    );
  } catch {}
}

// Seq base: wall-clock so two panes/sources do not collide, floored above any
// release seq the previous watchdog wrote so a fast restart is not outranked.
let reportSeq = Date.now() * 1000;
try {
  const floor = Number.parseInt(
    fs.readFileSync(seqFloorPath(), "utf8").trim(),
    10,
  );
  if (Number.isFinite(floor) && floor > reportSeq) reportSeq = floor;
} catch {}

function nextSeq() {
  reportSeq += 1;
  // Tell the watchdog our latest seq so its release can sit just above it.
  try {
    watchdog?.stdin.write(`${reportSeq}\n`);
  } catch {}
  return reportSeq;
}

// Serialize CLI invocations so seq order always matches emission order.
let chain = Promise.resolve();
function serial(fn) {
  const next = chain.then(fn);
  chain = next.catch(() => {});
  return next;
}

function runHerdr(args) {
  const bin = process.env.HERDR_BIN_PATH || "herdr";
  debugLog("runHerdr", bin, args.join(" "));
  return new Promise((resolve) => {
    const child = spawn(bin, args, { stdio: ["ignore", "ignore", "ignore"] });
    child.on("error", (e) => {
      debugLog("runHerdr error", String(e));
      resolve(false);
    });
    child.on("exit", (code) => {
      debugLog("runHerdr exit", String(code));
      resolve(code === 0);
    });
  });
}

const paneId = () => process.env.HERDR_PANE_ID;

function reportAgent(state, sessionID, message) {
  const args = [
    "pane",
    "report-agent",
    paneId(), // positional: herdr pane report-agent <PANE_ID> --source ...
    "--source",
    SOURCE,
    "--agent",
    AGENT,
    "--state",
    state,
    "--seq",
    String(nextSeq()),
  ];
  if (sessionID) args.push("--agent-session-id", sessionID);
  if (message) args.push("--message", message);
  return serial(() => runHerdr(args));
}

function reportSession(sessionID) {
  if (!sessionID) return Promise.resolve(false);
  return serial(() =>
    runHerdr([
      "pane",
      "report-agent-session",
      paneId(), // positional: herdr pane report-agent-session <PANE_ID> ...
      "--source",
      SOURCE,
      "--agent",
      AGENT,
      "--seq",
      String(nextSeq()),
      "--agent-session-id",
      sessionID,
    ]),
  );
}

// Aggregate state: blocked > working (any busy session) > idle.
// `blockedDepth` counts overlapping permission/question dialogs so one reply
// cannot clear a still-pending sibling.
const busy = new Set();
let blockedDepth = 0;
let currentSessionID;
let lastReportedState;
let lastReportedSession;

function syncState() {
  const blocked = blockedDepth > 0;
  const next = blocked ? "blocked" : busy.size > 0 ? "working" : "idle";
  if (next === lastReportedState) return Promise.resolve();
  const message = blocked ? "waiting for user input" : undefined;
  // Only remember the state once herdr accepted the report, so a failed
  // spawn/CLI call is retried on the next transition instead of sticking.
  return reportAgent(next, currentSessionID, message).then((ok) => {
    if (ok) lastReportedState = next;
  });
}

// Crash-proof exit cleanup: see design notes above.
let watchdog;
function spawnWatchdog() {
  watchdog = spawn(
    WATCHDOG_BIN,
    ["watch", "--pane", paneId(), "--source", SOURCE, "--agent", AGENT],
    { detached: true, stdio: ["pipe", "ignore", "ignore"] },
  );
  watchdog.on("error", () => {});
  watchdog.stdin.on("error", () => {});
  watchdog.stdin.unref?.();
  watchdog.unref();
}

export const MimoHerdrStatePlugin = async () => {
  debugLog(
    "plugin init env=",
    JSON.stringify({
      HERDR_ENV: process.env.HERDR_ENV,
      HERDR_SOCKET_PATH: process.env.HERDR_SOCKET_PATH,
      HERDR_PANE_ID: process.env.HERDR_PANE_ID,
      HERDR_BIN_PATH: process.env.HERDR_BIN_PATH,
      CLAUDECODE: process.env.CLAUDECODE,
      CLAUDE_CODE_ENTRYPOINT: process.env.CLAUDE_CODE_ENTRYPOINT,
    }),
  );
  if (
    process.env.HERDR_ENV !== "1" ||
    !process.env.HERDR_SOCKET_PATH ||
    !process.env.HERDR_PANE_ID
  ) {
    return {};
  }
  if (isNestedUnderAnotherAgent()) {
    debugLog("nested under another agent; staying silent");
    return {};
  }

  spawnWatchdog();
  await syncState(); // claim the label before any session event fires
  debugLog("plugin init done, initial state reported");

  return {
    dispose: async () => {
      try {
        watchdog?.stdin.end();
      } catch {}
    },
    event: async ({ event }) => {
      const type = event?.type;
      const properties = event?.properties ?? {};

      switch (type) {
        case "session.status": {
          const status =
            typeof properties.status === "string"
              ? properties.status
              : properties.status?.type;
          const id = properties.sessionID;
          if (status === "idle") {
            if (id) busy.delete(id);
            else busy.clear();
            if (busy.size === 0) blockedDepth = 0;
          } else if (status && id) {
            busy.add(id);
            if (id !== currentSessionID) {
              currentSessionID = id;
              if (id !== lastReportedSession) {
                lastReportedSession = id;
                await reportSession(id);
              }
            }
          }
          await syncState();
          break;
        }
        case "session.idle":
        case "session.deleted": {
          const id = properties.sessionID ?? properties.info?.id;
          if (id) busy.delete(id);
          if (busy.size === 0) blockedDepth = 0;
          await syncState();
          break;
        }
        case "permission.asked":
        case "question.asked":
          blockedDepth += 1;
          await syncState();
          break;
        case "permission.replied":
        case "question.replied":
        case "question.rejected":
          blockedDepth = Math.max(0, blockedDepth - 1);
          await syncState();
          break;
        case "session.created": {
          const id = properties.info?.id ?? properties.sessionID;
          if (id) await reportSession(id);
          break;
        }
        default:
          break;
      }
    },
  };
};
