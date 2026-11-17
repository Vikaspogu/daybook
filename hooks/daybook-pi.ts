import { spawn } from "node:child_process";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

export interface HookResult {
  code: number;
  output: string;
}

export function runHook(
  hook: string,
  input: Record<string, unknown>,
  command = "daybook",
  timeoutMs = 5000,
): Promise<HookResult> {
  return new Promise((resolve) => {
    const child = spawn(command, ["hook", hook], {
      cwd: String(input.cwd || process.cwd()),
      env: { ...process.env, PI_SESSION_ID: String(input.session_id || "") },
      stdio: ["pipe", "pipe", "pipe"],
    });
    let output = "";
    let settled = false;
    const finish = (code: number) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve({ code, output: output.trim() });
    };
    const timer = setTimeout(() => {
      output += "daybook hook timed out";
      finish(124);
      child.kill("SIGKILL");
    }, timeoutMs);
    child.stdout.on("data", (chunk) => { output += chunk.toString(); });
    child.stderr.on("data", (chunk) => { output += chunk.toString(); });
    child.on("error", (error) => {
      output += `daybook hook failed: ${error.message}`;
      finish(1);
    });
    child.on("close", (code) => finish(code ?? 1));
    child.stdin.on("error", () => {});
    child.stdin.end(JSON.stringify(input));
  });
}

function hookInput(ctx: ExtensionContext, hookEventName: string): Record<string, unknown> {
  return {
    session_id: ctx.sessionManager.getSessionId(),
    transcript_path: ctx.sessionManager.getSessionFile() ?? "",
    cwd: ctx.cwd,
    hook_event_name: hookEventName,
  };
}

export default function daybook(pi: ExtensionAPI) {
  pi.on("session_start", async (_event, ctx) => {
    process.env.PI_SESSION_ID = ctx.sessionManager.getSessionId();
    const result = await runHook("session-start", hookInput(ctx, "SessionStart"));
    // Recall arrives as plain stdout; show it and keep it in the conversation.
    if (result.output && ctx.hasUI) ctx.ui.notify(result.output, "info");
    if (result.output) {
      pi.sendMessage({ customType: "daybook-hook", content: result.output, display: true });
    }
    if (result.code !== 0 && !result.output) {
      if (ctx.hasUI) ctx.ui.notify(`daybook session_start failed with exit code ${result.code}`, "warning");
    }
  });

  pi.on("session_shutdown", async (event, ctx) => {
    if (event.reason === "reload") return;
    await runHook("session-end", {
      ...hookInput(ctx, "SessionEnd"),
      reason: event.reason,
    }, "daybook", 10000);
  });
}
