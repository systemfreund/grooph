// Smoke test of a deployed MCP server with the MCP Inspector's CLI, the
// official client, over HTTP: what a host such as Claude sees.
//
//   node mcp/smoke.mjs https://grooph.app/mcp
//
// CI runs it after each deploy (.github/workflows/rust.yml) against the
// deployment's own URL. It checks the tool list, a call that opens, one
// that does not (isError, by the link check) and the view's resource.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";

// Pinned, so a new release cannot break the pipeline.
const INSPECTOR = "@modelcontextprotocol/inspector@2.12.0";
const url = process.argv[2];
if (!url) {
  console.error("usage: node mcp/smoke.mjs <server url>, e.g. https://grooph.app/mcp");
  process.exit(2);
}
const origin = new URL(url).origin;

function inspect(...args) {
  try {
    const out = execFileSync("npx", ["--yes", INSPECTOR, "--cli", url, "--transport", "http", ...args], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return JSON.parse(out);
  } catch (e) {
    // The CLI exits with an error when a tool answers isError, but still
    // prints the result.
    if (/tool_is_error/.test(e.stderr || "") && e.stdout) return JSON.parse(e.stdout);
    throw e;
  }
}

// A fresh deployment may take a moment to answer.
async function firstAnswer(...args) {
  for (let attempt = 1; ; attempt++) {
    try {
      return inspect(...args);
    } catch (e) {
      if (attempt >= 5) throw e;
      console.log(`no answer yet (${attempt}), retrying`);
      await new Promise((resolve) => setTimeout(resolve, 5000));
    }
  }
}

const { tools } = await firstAnswer("--method", "tools/list");
const tool = tools.find((t) => t.name === "show_metronome");
assert.ok(tool, "tools/list has show_metronome");
assert.equal(tool.annotations?.readOnlyHint, true);
assert.equal(tool.annotations?.title, tool.title);
const uri = tool._meta?.ui?.resourceUri;
assert.ok(uri, "show_metronome has a view");
console.log("tools/list: show_metronome");

const call = (...args) =>
  inspect("--method", "tools/call", "--tool-name", "show_metronome", ...args.flatMap((a) => ["--tool-arg", a]));
const opens = call("ts=7/8", "bpm=140", "poly=3:7");
assert.ok(!opens.isError, opens.content?.[0]?.text);
assert.ok(opens.structuredContent?.link?.startsWith(`${origin}/?v=2&bpm=140&ts=7/8`), opens.structuredContent?.link);
console.log("tools/call 7/8, 140 bpm, 3:7:", opens.structuredContent.link);

const misfit = call("ts=4/4", "poly=7:5");
assert.equal(misfit.isError, true, "a poly that does not fit is an error");
assert.match(misfit.content[0].text, /does not fit 4\/4/);
console.log("tools/call 4/4, 7:5: isError,", misfit.content[0].text.slice(0, 60) + "…");

const { contents } = inspect("--method", "resources/read", "--uri", uri);
const [view] = contents;
assert.equal(view.mimeType, "text/html;profile=mcp-app");
assert.deepEqual(view._meta?.ui?.csp, { connectDomains: [origin], resourceDomains: [origin] });
assert.match(view.text, /id="grooph-canvas"/);
console.log("resources/read:", view.mimeType, "with this origin in its CSP");

console.log("smoke test passed:", url);
