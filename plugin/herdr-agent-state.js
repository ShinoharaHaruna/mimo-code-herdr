// MiMo Code -> herdr custom-agent state reporter.
//
// Reports semantic lifecycle state over herdr's official custom-integration
// path (`herdr pane report-agent --source custom:...`, see
// https://herdr.dev/docs/integrations/#integrate-your-own-agent) instead of
// the raw socket API, so MiMo Code shows up in the herdr agents sidebar as
// `mimo` without any process spoofing.
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
//    with a seq stamped +1s ahead so it outranks every report we made while
//    losing to reports from any mimo started later.
//  - A mimo spawned as a tool of another agent (e.g. a Claude Code Bash
//    session) inherits the pane's HERDR_* env and would hijack that pane's
//    label; we stay silent when CLAUDECODE is set.

import { spawn } from "node:child_process";
import fs from "node:fs";

const SOURCE = "custom:mimo-herdr";
const AGENT = "mimo";
const WATCHDOG_BIN = "__MIMO_HERDR_BIN__"; // replaced at install time

// Debug logging: set MIMO_HERDR_DEBUG=1 in the pane env to trace reports.
const DEBUG = process.env.MIMO_HERDR_DEBUG === "1";
function debugLog(...parts) {
  if (!DEBUG) return;
  try {
    fs.appendFileSync(
      "/tmp/mimo-herdr-plugin.log",
      `${new Date().toISOString()} ${parts.join(" ")}\n`,
    );
  } catch {}
}

let reportSeq = Date.now() * 1000;
function nextSeq() {
  reportSeq += 1;
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
const busy = new Set();
let blocked = false;
let currentSessionID;
let lastReportedState;
let lastReportedSession;

function syncState() {
  const next = blocked ? "blocked" : busy.size > 0 ? "working" : "idle";
  if (next === lastReportedState) return Promise.resolve();
  lastReportedState = next;
  const message = blocked ? "waiting for user input" : undefined;
  return reportAgent(next, currentSessionID, message);
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
    }),
  );
  if (
    process.env.HERDR_ENV !== "1" ||
    !process.env.HERDR_SOCKET_PATH ||
    !process.env.HERDR_PANE_ID
  ) {
    return {};
  }
  if (process.env.CLAUDECODE) {
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
            if (busy.size === 0) blocked = false;
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
          if (busy.size === 0) blocked = false;
          await syncState();
          break;
        }
        case "permission.asked":
        case "question.asked":
          blocked = true;
          await syncState();
          break;
        case "permission.replied":
        case "question.replied":
        case "question.rejected":
          blocked = false;
          await syncState();
          break;
        case "session.created":
          await reportSession(properties.info?.id ?? properties.sessionID);
          break;
        default:
          break;
      }
    },
  };
};
