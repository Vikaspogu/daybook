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
