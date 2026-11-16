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
  wake = false,
) {
  if (!output) return;
  if (ctx.hasUI) ctx.ui.notify(output, level);
  pi.sendMessage({
    customType: "daybook-hook",
    content: output,
    display: true,
  }, { deliverAs: "steer", triggerTurn: wake || !ctx.isIdle() });
}

export default function daybook(pi: ExtensionAPI) {
  let timer: ReturnType<typeof setInterval> | undefined;
  let polling = false;
  let stopped = false;
  let generation = 0;
  let retryAfter = 0;

  async function receive(ctx: ExtensionContext, input: Record<string, unknown>, idle = false) {
    const currentGeneration = generation;
    const result = await runHook("post-tool-use", input);
    if (currentGeneration !== generation) return;
    if (result.code !== 0) retryAfter = Date.now() + 60_000;
    if (!result.output || result.code !== 0 || stopped) return;
    try {
      const notice = JSON.parse(result.output);
      const text = notice.hookSpecificOutput?.additionalContext;
      if (typeof text !== "string" || !text) return;
      if (idle && !ctx.isIdle()) return;
      report(pi, ctx, text, "warning", idle && notice.daybookWake === true);
      // Record acceptance only after sendMessage succeeds. Fetching alone never acknowledges.
      if (Array.isArray(notice.daybookDelivery) && notice.daybookDelivery.length > 0
        || Number.isSafeInteger(notice.daybookLegacyCursor)) {
        await runHook("post-tool-use", {
          ...hookInput(ctx, "PostToolUse"),
          ...(notice.daybookDelivery?.length ? { board_delivered: notice.daybookDelivery } : {}),
          board_legacy_cursor: notice.daybookLegacyCursor,
        });
      }
    } catch {
      // Failed wake attempts are not retried automatically; the pending inbox remains available.
    }
  }

  pi.on("session_start", async (_event, ctx) => {
    generation++;
    stopped = false;
    retryAfter = 0;
    process.env.PI_SESSION_ID = ctx.sessionManager.getSessionId();
    if (timer) clearInterval(timer);
    // isIdle excludes runs, retries and queued continuations (extensions.md:1255-1257).
    timer = setInterval(async () => {
      if (stopped || polling || !ctx.isIdle() || Date.now() < retryAfter) return;
      polling = true;
      try {
        await receive(ctx, { ...hookInput(ctx, "PostToolUse"), board_idle: true }, true);
      } finally {
        polling = false;
      }
    }, 15_000);
    timer.unref();
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
    const toolInput = event.input as { path?: unknown } | undefined;
    await receive(ctx, {
      ...hookInput(ctx, event.isError ? "PostToolUseFailure" : "PostToolUse"),
      tool_name: event.toolName,
      tool_input: !event.isError && typeof toolInput?.path === "string" ? { file_path: toolInput.path } : {},
    });
  });

  pi.on("session_shutdown", async (event, ctx) => {
    generation++;
    stopped = true;
    if (timer) clearInterval(timer);
    timer = undefined;
    if (event.reason === "reload") return;
    await runHook("session-end", {
      ...hookInput(ctx, "SessionEnd"),
      reason: event.reason,
    }, "daybook", 10000);
  });
}
