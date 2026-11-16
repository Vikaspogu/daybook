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

test("ordinary notices steer active runs, leave idle sessions idle, and record acceptance", async () => {
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

test("idle timer wakes once for a server-approved question and stops on reload", async () => {
  const { default: daybook } = await import("./daybook-pi.ts");
  const { readFile } = await import("node:fs/promises");
  const dir = await mkdtemp(join(tmpdir(), "daybook-pi-wake-"));
  const log = join(dir, "calls");
  const claimed = join(dir, "claimed");
  await writeFile(join(dir, "daybook"), `#!/bin/sh
read payload
printf '%s\\n' "$payload" >> '${log}'
case "$payload" in
  *board_idle*)
    if [ ! -f '${claimed}' ]; then
      touch '${claimed}'
      printf '%s' '{"hookSpecificOutput":{"additionalContext":"Untrusted peer message #42: question"},"daybookDelivery":[42],"daybookWake":true}'
    fi ;;
esac
`, { mode: 0o700 });
  const oldPath = process.env.PATH;
  const oldSession = process.env.PI_SESSION_ID;
  const oldInterval = globalThis.setInterval;
  const oldClear = globalThis.clearInterval;
  let tick: () => Promise<void>;
  let clears = 0;
  let idle = false;
  const messages: unknown[] = [];
  const handlers = new Map<string, Function>();
  globalThis.setInterval = ((fn: () => Promise<void>, ms: number) => {
    assert.equal(ms, 15_000);
    tick = fn;
    return { unref() {} };
  }) as never;
  globalThis.clearInterval = (() => { clears++; }) as never;
  process.env.PATH = `${dir}:${oldPath}`;
  const ctx = { cwd: dir, hasUI: false, isIdle: () => idle, sessionManager: { getSessionId: () => "receiver", getSessionFile: () => "" } };
  try {
    daybook({ on: (name: string, fn: Function) => handlers.set(name, fn), sendMessage: (_message: unknown, options: unknown) => messages.push(options) } as never);
    await handlers.get("session_start")!({}, ctx);
    await tick!();
    assert.equal((await readFile(log, "utf8")).trim().split("\n").length, 1);
    idle = true;
    await tick!();
    await tick!();
    assert.deepEqual(messages, [{ deliverAs: "steer", triggerTurn: true }]);
    const calls = (await readFile(log, "utf8")).trim().split("\n").map((line) => JSON.parse(line));
    assert.equal(calls[1].board_idle, true);
    assert.deepEqual(calls[2].board_delivered, [42]);
    await handlers.get("session_shutdown")!({ reason: "reload" }, ctx);
    assert.equal(clears, 1);
    await tick!();
    assert.equal((await readFile(log, "utf8")).trim().split("\n").length, calls.length);
  } finally {
    globalThis.setInterval = oldInterval;
    globalThis.clearInterval = oldClear;
    process.env.PATH = oldPath;
    if (oldSession === undefined) delete process.env.PI_SESSION_ID;
    else process.env.PI_SESSION_ID = oldSession;
  }
});


test("session replacement drops an in-flight idle poll without recording acceptance", async () => {
  const { default: daybook } = await import("./daybook-pi.ts");
  const { readFile, access } = await import("node:fs/promises");
  const { setTimeout: delay } = await import("node:timers/promises");
  const dir = await mkdtemp(join(tmpdir(), "daybook-pi-replace-"));
  const log = join(dir, "calls");
  const started = join(dir, "started");
  const release = join(dir, "release");
  await writeFile(join(dir, "daybook"), `#!/bin/sh
read payload
printf '%s\\n' "$payload" >> '${log}'
case "$payload" in
  *board_idle*)
    touch '${started}'
    while [ ! -f '${release}' ]; do sleep 0.01; done
    printf '%s' '{"hookSpecificOutput":{"additionalContext":"old session question"},"daybookDelivery":[42],"daybookWake":true}' ;;
esac
`, { mode: 0o700 });
  const oldPath = process.env.PATH;
  const oldSession = process.env.PI_SESSION_ID;
  const oldInterval = globalThis.setInterval;
  const oldClear = globalThis.clearInterval;
  let tick: () => Promise<void>;
  let pending: Promise<void> | undefined;
  const messages: unknown[] = [];
  const handlers = new Map<string, Function>();
  globalThis.setInterval = ((fn: () => Promise<void>) => {
    tick = fn;
    return { unref() {} };
  }) as never;
  globalThis.clearInterval = (() => {}) as never;
  process.env.PATH = `${dir}:${oldPath}`;
  const context = (id: string) => ({ cwd: dir, hasUI: false, isIdle: () => true, sessionManager: { getSessionId: () => id, getSessionFile: () => "" } });
  try {
    daybook({ on: (name: string, fn: Function) => handlers.set(name, fn), sendMessage: (message: unknown) => messages.push(message) } as never);
    const oldCtx = context("old");
    await handlers.get("session_start")!({}, oldCtx);
    pending = tick!();
    const deadline = Date.now() + 2000;
    while (await access(started).then(() => false, () => true)) {
      assert.ok(Date.now() < deadline, "idle poll did not start");
      await delay(10);
    }
    await handlers.get("session_start")!({}, context("new"));
    await writeFile(release, "");
    await pending;
    assert.deepEqual(messages, []);
    const calls = (await readFile(log, "utf8")).trim().split("\n").map((line) => JSON.parse(line));
    assert.equal(calls.length, 3);
    assert.equal(calls[1].session_id, "old");
    assert.equal(calls[2].session_id, "new");
    assert.ok(calls.every((call) => call.board_delivered === undefined));
  } finally {
    await writeFile(release, "");
    await pending;
    globalThis.setInterval = oldInterval;
    globalThis.clearInterval = oldClear;
    process.env.PATH = oldPath;
    if (oldSession === undefined) delete process.env.PI_SESSION_ID;
    else process.env.PI_SESSION_ID = oldSession;
  }
});
