// Sushi for pi: shows pi's sessions, steps and diffs on the notch.
// Installed by `sushi install --agent pi --write`; reinstalling overwrites this file.
//
// Every event is handed to `sushi-hook --agent pi`, which forwards it to the daemon and exits at
// once if the daemon is not running. pi never waits for Sushi unless you opt in to approvals
// from the notch with SUSHI_PI_APPROVE=1 (pi has no permission prompt of its own).
// @ts-nocheck

import { spawn } from "node:child_process";

const HOOK = process.env.SUSHI_HOOK || "__SUSHI_HOOK__";
const APPROVE = process.env.SUSHI_PI_APPROVE === "1";

/** Run `sushi-hook --agent pi` with `payload`; resolves with what it printed (parsed), if anything. */
function send(payload: Record<string, unknown>): Promise<any> {
  return new Promise((resolve) => {
    let out = "";
    let child;
    try {
      child = spawn(HOOK, ["--agent", "pi"], { stdio: ["pipe", "pipe", "ignore"] });
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

/** The text of a message or tool result (`content` is a string or a list of parts). */
function textOf(content: any): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .filter((p) => p && p.type === "text" && typeof p.text === "string")
    .map((p) => p.text)
    .join("\n");
}

export default function (pi) {
  if (process.env.SUSHI_CHAT) return; // Sushi's own chat runs pi too: not a session to watch
  let root = false; // only the interactive session is shown
  let lastAnswer = "";

  const base = (name: string, ctx: any) => {
    let id = "";
    try {
      id = ctx?.sessionManager?.getSessionId?.() ?? "";
    } catch {}
    return { hook_event_name: name, session_id: id || String(process.pid), cwd: ctx?.cwd ?? process.cwd(), pid: process.pid };
  };

  pi.on("session_start", async (_event, ctx) => {
    if (ctx?.mode !== "tui") return;
    root = true;
    await send(base("SessionStart", ctx));
  });

  pi.on("session_shutdown", async (_event, ctx) => {
    if (!root) return;
    await send(base("SessionEnd", ctx));
  });

  pi.on("agent_start", async (_event, ctx) => {
    if (!root) return;
    lastAnswer = "";
    await send(base("UserPromptSubmit", ctx));
  });

  pi.on("agent_end", async (event) => {
    const last = [...(event?.messages ?? [])].reverse().find((m) => m?.role === "assistant");
    if (last) lastAnswer = textOf(last.content);
  });

  pi.on("agent_settled", async (_event, ctx) => {
    if (!root || ctx?.isIdle?.() !== true) return;
    await send({ ...base("Stop", ctx), last_assistant_message: lastAnswer });
  });

  pi.on("tool_call", async (event, ctx) => {
    if (!root) return;
    const call = { ...base("PreToolUse", ctx), tool_name: event.toolName, tool_input: event.input, tool_use_id: event.toolCallId };
    await send(call);
    if (!APPROVE) return;
    const answer = await send({ ...call, hook_event_name: "PermissionRequest" });
    if (answer?.decision === "deny") {
      return { block: true, reason: answer.message || "Denied from Sushi" };
    }
  });

  pi.on("tool_result", async (event, ctx) => {
    if (!root) return;
    await send({
      ...base(event.isError ? "PostToolUseFailure" : "PostToolUse", ctx),
      tool_name: event.toolName,
      tool_input: event.input,
      tool_use_id: event.toolCallId,
      tool_response: textOf(event.content),
    });
  });
}
