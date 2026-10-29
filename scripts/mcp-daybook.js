#!/usr/bin/env node
// Minimal MCP stdio server exposing the daybook work journal to Codex.
// Wraps the `daybook` CLI (auth comes from its own config), no SDK needed.
// protocol: one JSON-RPC message per line on stdin, one per line on stdout.
"use strict";

const readline = require("readline");
const { execFile } = require("child_process");

const TOOLS = [
  {
    name: "daybook_query",
    description:
      "Query the daybook work journal: what was worked on on a given date across all projects and agent runtimes, or usage stats (sessions, tokens, cost, tests, friction) for the last 7 days. Use for recall questions like 'what did I work on Tuesday and why?'.",
    inputSchema: {
      type: "object",
      properties: {
        date: { type: "string", description: "YYYY-MM-DD to recall (default: today, UTC)" },
        stats: { type: "boolean", description: "true: return 7-day stats instead of one day's sessions" },
      },
      additionalProperties: false,
    },
  },
];

function runDaybook(args) {
  return new Promise((resolve) => {
    execFile(
      "daybook",
      args,
      { timeout: 30_000, maxBuffer: 4 * 1024 * 1024 },
      (err, stdout, stderr) =>
        resolve({ code: err && err.code ? err.code : 0, stdout: stdout || "", stderr: stderr || "", err }),
    );
  });
}

async function handle(msg) {
  const { id, method, params } = msg;
  if (method === "initialize") {
    return {
      protocolVersion: "2024-11-05",
      capabilities: { tools: {} },
      serverInfo: { name: "daybook", version: "1.0.0" },
    };
  }
  if (method === "tools/list") return { tools: TOOLS };
  if (method === "ping") return {};
  if (method === "tools/call") {
    const name = params && params.name;
    const args = (params && params.arguments) || {};
    if (name !== "daybook_query") {
      return { content: [{ type: "text", text: `unknown tool: ${name}` }], isError: true };
    }
    const cli =
      args.stats === true
        ? ["stats", "--week"]
        : ["query", "--", typeof args.date === "string" ? args.date : new Date().toISOString().slice(0, 10)];
    const r = await runDaybook(cli);
    if (r.code !== 0 || r.err) {
      return { content: [{ type: "text", text: `daybook failed: ${r.stderr || r.stdout || (r.err && r.err.message) || "unknown error"}` }], isError: true };
    }
    return { content: [{ type: "text", text: r.stdout }] };
  }
  return undefined; // unknown method: caller decides (error if request, silence if notification)
}

const rl = readline.createInterface({ input: process.stdin });
// ponytail: no SDK — one line per message, result/error per JSON-RPC 2.0
rl.on("line", async (line) => {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }
  let result;
  try {
    result = await handle(msg);
  } catch (e) {
    result = { isError: true, content: [{ type: "text", text: `handler error: ${e}` }] };
  }
  if (msg.id === undefined) return; // notification: never reply
  if (result !== undefined) {
    process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result }) + "\n");
  } else {
    process.stdout.write(
      JSON.stringify({ jsonrpc: "2.0", id: msg.id, error: { code: -32601, message: "Method not found" } }) + "\n",
    );
  }
});
