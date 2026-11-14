import { spawn } from "node:child_process";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

export interface HookResult {
  code: number;
  output: string;
}

export function shouldBlock(code: number, strict: boolean): boolean {
  return code === 2 || (strict && code !== 0);
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

function report(
  pi: ExtensionAPI,
  ctx: ExtensionContext,
  output: string,
  level: "info" | "warning" = "warning",
) {
  if (!output) return;
  if (ctx.hasUI) ctx.ui.notify(output, level);
  pi.sendMessage({
    customType: "daybook-hook",
    content: output,
    display: true,
  }, { triggerTurn: false });
}

export default function daybook(pi: ExtensionAPI) {
  pi.on("session_start", async (_event, ctx) => {
    process.env.PI_SESSION_ID = ctx.sessionManager.getSessionId();
    const result = await runHook("session-start", hookInput(ctx, "SessionStart"));
    report(pi, ctx, result.output, "info");
    if (result.code !== 0 && !result.output) {
      report(pi, ctx, `daybook session_start failed with exit code ${result.code}`);
    }
  });

  pi.on("tool_call", async (event, ctx) => {
    if (event.toolName !== "write" && event.toolName !== "edit") return;
    const toolInput = event.input as { path?: unknown };
    if (typeof toolInput.path !== "string") return;
    const result = await runHook("pre-tool-use", {
      ...hookInput(ctx, "PreToolUse"),
      tool_name: event.toolName,
      tool_input: { file_path: toolInput.path },
    });
    const strict = process.env.DAYBOOK_LOCK_ENFORCE?.toLowerCase() === "strict"
      || ["1", "true", "yes"].includes(process.env.DAYBOOK_STRICT_LOCKS ?? "");
    if (shouldBlock(result.code, strict)) {
      return { block: true, reason: result.output || "Daybook blocked this edit" };
    }
    report(pi, ctx, result.output);
  });

  pi.on("tool_result", async (event, ctx) => {
    if (event.isError || (event.toolName !== "write" && event.toolName !== "edit")) return;
    const toolInput = event.input as { path?: unknown };
    if (typeof toolInput.path !== "string") return;
    const result = await runHook("post-tool-use", {
      ...hookInput(ctx, "PostToolUse"),
      tool_name: event.toolName,
      tool_input: { file_path: toolInput.path },
    });
    report(pi, ctx, result.output);
  });

  pi.on("session_shutdown", async (event, ctx) => {
    if (event.reason === "reload") return;
    await runHook("session-end", {
      ...hookInput(ctx, "SessionEnd"),
      reason: event.reason,
    }, "daybook", 10000);
  });
}
