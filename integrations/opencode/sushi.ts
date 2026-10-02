// Sushi for opencode: shows opencode's sessions, steps and diffs on the notch and lets you
// allow or deny permissions from it.
// Installed by `sushi install --agent opencode --write`; reinstalling overwrites this file.
//
// Every event is handed to `sushi-hook --agent opencode`, which forwards it to the daemon and
// exits at once if the daemon is not running.
// @ts-nocheck

import { spawn } from "node:child_process";

const HOOK = process.env.SUSHI_HOOK || "__SUSHI_HOOK__";

/** Run `sushi-hook --agent opencode` with `payload`; resolves with what it printed (parsed), if anything. */
function send(payload) {
  return new Promise((resolve) => {
    let out = "";
    let child;
    try {
      child = spawn(HOOK, ["--agent", "opencode"], { stdio: ["pipe", "pipe", "ignore"] });
    } catch {
      return resolve(undefined);
    }
    child.on("error", () => resolve(undefined));
    child.stdout.on("data", (d) => (out += d));
    child.on("close", () => {
      try {
        resolve(out.trim() ? JSON.parse(out) : undefined);
      } catch {
        resolve(undefined);
      }
    });
    child.stdin.on("error", () => {});
    child.stdin.end(JSON.stringify(payload));
  });
}

export const SushiPlugin = async ({ directory }) => {
  if (process.env.SUSHI_CHAT) return {}; // Sushi's own chat: not a session to watch
  const base = (name, sessionID) => ({
    hook_event_name: name,
    session_id: sessionID,
    cwd: directory,
    pid: process.pid,
  });
  const lastAnswer = new Map(); // session id -> text of the last assistant message
  const callArgs = new Map(); // call id -> arguments (the "after" hook may not repeat them)

  return {
    event: async ({ event }) => {
      const p = event?.properties ?? {};
      switch (event?.type) {
        case "session.created":
          if (p.info?.parentID) return; // subagent sessions are part of their parent's turn
          return void (await send({ ...base("SessionStart", p.info?.id), cwd: p.info?.directory ?? directory }));
        case "session.deleted":
          return void (await send(base("SessionEnd", p.info?.id)));
        case "message.part.updated":
          if (p.part?.type === "text" && p.part.sessionID && typeof p.part.text === "string") {
            lastAnswer.set(p.part.sessionID, p.part.text);
          }
          return;
        case "session.idle":
          return void (await send({ ...base("Stop", p.sessionID), last_assistant_message: lastAnswer.get(p.sessionID) ?? "" }));
      }
    },

    "chat.message": async (input) => {
      lastAnswer.delete(input.sessionID);
      await send(base("UserPromptSubmit", input.sessionID));
    },

    "tool.execute.before": async (input, output) => {
      callArgs.set(input.callID, output.args);
      await send({ ...base("PreToolUse", input.sessionID), tool_name: input.tool, tool_input: output.args, tool_use_id: input.callID });
    },

    "tool.execute.after": async (input, output) => {
      const args = input.args ?? callArgs.get(input.callID) ?? {};
      callArgs.delete(input.callID);
      await send({
        ...base("PostToolUse", input.sessionID),
        tool_name: input.tool,
        tool_input: args,
        tool_use_id: input.callID,
        tool_response: output.output,
      });
    },

    // opencode asks before running something it was not told to allow: the notch answers.
    "permission.ask": async (input, output) => {
      const meta = input.metadata ?? {};
      const answer = await send({
        ...base("PermissionRequest", input.sessionID),
        tool_name: input.type,
        tool_input: { command: input.title, ...meta },
        tool_use_id: input.callID,
      });
      if (answer?.decision === "allow") output.status = "allow";
      else if (answer?.decision === "deny") output.status = "deny";
    },
  };
};
