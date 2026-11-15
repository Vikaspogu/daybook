import assert from "node:assert/strict";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { runHook, shouldBlock } from "./daybook-pi.ts";

test("runHook sends JSON on stdin and returns hook output", async () => {
  const dir = await mkdtemp(join(tmpdir(), "daybook-pi-"));
  const fake = join(dir, "fake-daybook");
  await writeFile(fake, "#!/bin/sh\nread payload\nprintf '%s %s %s' \"$1\" \"$2\" \"$payload\"\nprintf ' warning'\n", { mode: 0o700 });
  const result = await runHook("pre-tool-use", { cwd: dir, session_id: "abc" }, fake);
  assert.equal(result.code, 0);
  assert.match(result.output, /^hook pre-tool-use /);
  assert.match(result.output, /"session_id":"abc"/);
  assert.match(result.output, /warning/);
});

test("timeouts settle and advisory failures do not block", async () => {
  const dir = await mkdtemp(join(tmpdir(), "daybook-pi-"));
  const fake = join(dir, "fake-daybook");
  await writeFile(fake, "#!/bin/sh\nsleep 10\n", { mode: 0o700 });
  const result = await runHook("pre-tool-use", { cwd: dir }, fake, 20);
  assert.equal(result.code, 124);
  assert.equal(shouldBlock(1, false), false);
  assert.equal(shouldBlock(124, false), false);
  assert.equal(shouldBlock(2, false), true);
  assert.equal(shouldBlock(1, true), true);
});

test("read-only tool results steer mid-run, never wake an idle session, and record acceptance", async () => {
  const { default: daybook } = await import("./daybook-pi.ts");
  const dir = await mkdtemp(join(tmpdir(), "daybook-pi-"));
  const fake = join(dir, "daybook");
  const log = join(dir, "calls");
  await writeFile(fake, `#!/bin/sh
read payload
printf '%s\\n' "$payload" >> '${log}'
case "$payload" in
  *board_delivered*) ;;
  *) printf '%s' '{"hookSpecificOutput":{"additionalContext":"Untrusted peer message #42: preview"},"daybookDelivery":[42],"daybookLegacyCursor":81}' ;;
esac
`, { mode: 0o700 });
  const oldPath = process.env.PATH;
  process.env.PATH = `${dir}:${oldPath}`;
  const handlers = new Map<string, Function>();
  const messages: unknown[] = [];
  const pi = {
    on: (event: string, handler: Function) => handlers.set(event, handler),
    sendMessage: (message: unknown, options: unknown) => messages.push({ message, options }),
  };
  let idle = false;
  const ctx = { cwd: dir, hasUI: false, isIdle: () => idle, sessionManager: { getSessionId: () => "receiver", getSessionFile: () => "" } };
  try {
    daybook(pi as never);
    assert.equal(messages.length, 0);
    await handlers.get("tool_result")!({ toolName: "read", input: { path: "README.md" }, isError: false }, ctx);
    assert.equal(messages.length, 1);
    assert.deepEqual(messages[0], {
      message: { customType: "daybook-hook", content: "Untrusted peer message #42: preview", display: true },
      options: { deliverAs: "steer", triggerTurn: true },
    });
    idle = true;
    await handlers.get("tool_result")!({ toolName: "read", input: { path: "README.md" }, isError: false }, ctx);
    assert.deepEqual((messages[1] as { options: unknown }).options, { deliverAs: "steer", triggerTurn: false });
    const { readFile } = await import("node:fs/promises");
    const calls = (await readFile(log, "utf8")).trim().split("\n").map((line) => JSON.parse(line));
    assert.equal(calls.length, 4);
    assert.deepEqual(calls[1].board_delivered, [42]);
    assert.equal(calls[1].board_legacy_cursor, 81);
  } finally {
    process.env.PATH = oldPath;
  }
});

test("empty notices and rejected Pi delivery do not record acceptance", async () => {
  const { default: daybook } = await import("./daybook-pi.ts");
  const { readFile } = await import("node:fs/promises");
  const dir = await mkdtemp(join(tmpdir(), "daybook-pi-"));
  const log = join(dir, "calls");
  const fake = join(dir, "daybook");
  const oldPath = process.env.PATH;
  process.env.PATH = `${dir}:${oldPath}`;
  const handlers = new Map<string, Function>();
  let sends = 0;
  const pi = {
    on: (event: string, handler: Function) => handlers.set(event, handler),
    sendMessage: () => { sends++; throw new Error("adapter rejected delivery"); },
  };
  const ctx = { cwd: dir, hasUI: false, isIdle: () => false, sessionManager: { getSessionId: () => "receiver", getSessionFile: () => "" } };
  try {
    daybook(pi as never);
    await writeFile(fake, "#!/bin/sh\nread payload\n", { mode: 0o700 });
    await handlers.get("tool_result")!({ toolName: "read", input: {} }, ctx);
    assert.equal(sends, 0);
    await writeFile(fake, `#!/bin/sh
read payload
printf '%s\\n' "$payload" >> '${log}'
printf '%s' '{"hookSpecificOutput":{"additionalContext":"Untrusted peer message #42"},"daybookDelivery":[42],"daybookLegacyCursor":81}'
`, { mode: 0o700 });
    await handlers.get("tool_result")!({ toolName: "read" }, ctx);
    assert.equal(sends, 1);
    const calls = (await readFile(log, "utf8")).trim().split("\n");
    assert.equal(calls.length, 1);
    assert.equal(JSON.parse(calls[0]).board_delivered, undefined);
  } finally {
    process.env.PATH = oldPath;
  }
});


test("failed tool uses failure event and empty delivery IDs skip acceptance", async () => {
  const { default: daybook } = await import("./daybook-pi.ts");
  const { readFile } = await import("node:fs/promises");
  const dir = await mkdtemp(join(tmpdir(), "daybook-pi-"));
  const log = join(dir, "calls");
  await writeFile(join(dir, "daybook"), `#!/bin/sh
read payload
printf '%s\\n' "$payload" >> '${log}'
printf '%s' '{"hookSpecificOutput":{"additionalContext":"startup context"},"daybookDelivery":[]}'
`, { mode: 0o700 });
  const oldPath = process.env.PATH;
  process.env.PATH = `${dir}:${oldPath}`;
  const handlers = new Map<string, Function>();
  let sends = 0;
  const pi = {
    on: (event: string, handler: Function) => handlers.set(event, handler),
    sendMessage: () => { sends++; },
  };
  const ctx = { cwd: dir, hasUI: false, isIdle: () => false, sessionManager: { getSessionId: () => "receiver", getSessionFile: () => "" } };
  try {
    daybook(pi as never);
    await handlers.get("tool_result")!({ toolName: "edit", input: { path: "README.md" }, isError: true }, ctx);
    const calls = (await readFile(log, "utf8")).trim().split("\n").map((line) => JSON.parse(line));
    assert.equal(calls[0].hook_event_name, "PostToolUseFailure");
    assert.deepEqual(calls[0].tool_input, {});
    assert.equal(calls.length, 1);
    assert.equal(sends, 1);
    await writeFile(join(dir, "daybook"), `#!/bin/sh
read payload
printf '%s\\n' "$payload" >> '${log}'
printf '%s' '{"hookSpecificOutput":{"additionalContext":"legacy preview"},"daybookLegacyCursor":81}'
`, { mode: 0o700 });
    await handlers.get("tool_result")!({ toolName: "edit", input: { path: "README.md" }, isError: false }, ctx);
    const accepted = (await readFile(log, "utf8")).trim().split("\n").map((line) => JSON.parse(line));
    assert.equal(accepted.length, 3);
    assert.equal(accepted[2].board_legacy_cursor, 81);
    assert.equal(accepted[2].board_delivered, undefined);
  } finally {
    process.env.PATH = oldPath;
  }
});
