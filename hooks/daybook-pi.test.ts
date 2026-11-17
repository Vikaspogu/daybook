import assert from "node:assert/strict";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { runHook } from "./daybook-pi.ts";

test("runHook sends JSON on stdin and returns hook output", async () => {
  const dir = await mkdtemp(join(tmpdir(), "daybook-pi-"));
  const fake = join(dir, "fake-daybook");
  await writeFile(fake, "#!/bin/sh\nread payload\nprintf '%s %s %s' \"$1\" \"$2\" \"$payload\"\nprintf ' warning'\n", { mode: 0o700 });
  const result = await runHook("session-start", { cwd: dir, session_id: "abc" }, fake);
  assert.equal(result.code, 0);
  assert.match(result.output, /^hook session-start /);
  assert.match(result.output, /"session_id":"abc"/);
  assert.match(result.output, /warning/);
});

test("timeouts settle with code 124", async () => {
  const dir = await mkdtemp(join(tmpdir(), "daybook-pi-"));
  const fake = join(dir, "fake-daybook");
  await writeFile(fake, "#!/bin/sh\nsleep 10\n", { mode: 0o700 });
  const result = await runHook("session-start", { cwd: dir }, fake, 20);
  assert.equal(result.code, 124);
});

test("session_start reports recall and session_shutdown archives", async () => {
  const { default: daybook } = await import("./daybook-pi.ts");
  const dir = await mkdtemp(join(tmpdir(), "daybook-pi-"));
  const fake = join(dir, "daybook");
  const log = join(dir, "calls");
  await writeFile(fake, `#!/bin/sh
read payload
printf '%s\\n' "$payload" >> '${log}'
case "$payload" in
  *SessionStart*) printf '%s' '[daybook] Relevant prior work' ;;
esac
`, { mode: 0o700 });
  const oldPath = process.env.PATH;
  process.env.PATH = `${dir}:${oldPath}`;
  const handlers = {};
  const sent = [];
  const notices = [];
  const pi = {
    on: (name, fn) => { handlers[name] = fn; },
    sendMessage: (msg) => sent.push(msg),
  };
  daybook(pi);
  const ctx = {
    cwd: dir,
    hasUI: true,
    ui: { notify: (text) => notices.push(text) },
    sessionManager: { getSessionId: () => "s1", getSessionFile: () => join(dir, "s1.jsonl") },
  };
  try {
    await handlers.session_start({}, ctx);
    assert.deepEqual(sent.map((m) => m.content), ["[daybook] Relevant prior work"]);
    assert.equal(notices.length, 1);
    await handlers.session_shutdown({ reason: "exit" }, ctx);
    const { readFile } = await import("node:fs/promises");
    const calls = await readFile(log, "utf8");
    assert.match(calls, /SessionStart/);
    assert.match(calls, /SessionEnd/);
    // No board payload fields survive the hook rewrite.
    assert.doesNotMatch(calls, /board_/);
  } finally {
    process.env.PATH = oldPath;
  }
});
