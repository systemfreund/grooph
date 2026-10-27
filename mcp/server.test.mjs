// Tests of the MCP server without Cloudflare: node --test mcp/

import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { metronomeQuery } from "./link.js";
import { RESOURCE_MIME, RESOURCE_URI, handleRequest } from "./server.js";

const ORIGIN = "https://feature-x.grooph.pages.dev";
const here = path.dirname(fileURLToPath(import.meta.url));

const env = {
  ASSETS: {
    fetch: async (request) =>
      new URL(request.url).pathname === "/embed.json"
        ? Response.json({ js: "grooph-app-abc123.js", wasm: "grooph-app-abc123_bg.wasm" })
        : new Response("Not found", { status: 404 }),
  },
};

async function rpc(method, params, environment = env) {
  const response = await handleRequest(
    new Request(`${ORIGIN}/mcp`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Accept: "application/json, text/event-stream" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
    }),
    environment,
  );
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("Access-Control-Allow-Origin"), "*");
  return response.json();
}

test("initialize advertises tools, resources and MCP Apps", async () => {
  const { result } = await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "t", version: "0" } });
  assert.equal(result.protocolVersion, "2025-06-18");
  assert.ok(result.capabilities.tools);
  assert.ok(result.capabilities.resources);
  assert.ok(result.capabilities.extensions["io.modelcontextprotocol/ui"]);
});

test("tools/list has show_metronome with its view", async () => {
  const { result } = await rpc("tools/list", {});
  const tool = result.tools.find((t) => t.name === "show_metronome");
  assert.ok(tool);
  assert.equal(tool._meta.ui.resourceUri, RESOURCE_URI);
  assert.deepEqual(Object.keys(tool.inputSchema.properties).sort(), ["bpm", "click", "poly", "su", "swing", "ts"]);
  assert.match(tool.description, /poly "3:2"/);
});

test("tools/call returns the link of this deployment", async () => {
  const { result } = await rpc("tools/call", {
    name: "show_metronome",
    arguments: { ts: "7/8", bpm: 70, click: ">q r:q. (q)", swing: 60, su: 16, poly: "3:7" },
  });
  const link = `${ORIGIN}/?v=2&bpm=70&ts=7/8&swing=60&su=16&click=%3Eq+r:q.+%28q%29&poly=3:7`;
  assert.equal(result.structuredContent.link, link);
  assert.ok(result.content[0].text.includes(`Open in grooph: ${link}`));
  assert.ok(!result.isError);
});

test("tools/call reports wrong numbers to the model", async () => {
  for (const args of [{ bpm: 500 }, { swing: 90 }, { su: 16 }, { swing: 60, su: 4 }, { ts: "seven" }, { poly: "three" }]) {
    const { result } = await rpc("tools/call", { name: "show_metronome", arguments: args });
    assert.equal(result.isError, true, JSON.stringify(args));
  }
  const { error } = await rpc("tools/call", { name: "other", arguments: {} });
  assert.equal(error.code, -32602);
});

test("resources/read returns the view with this origin in its CSP", async () => {
  const { result } = await rpc("resources/read", { uri: RESOURCE_URI });
  const [content] = result.contents;
  assert.equal(content.mimeType, RESOURCE_MIME);
  assert.deepEqual(content._meta.ui.csp, { connectDomains: [ORIGIN], resourceDomains: [ORIGIN] });
  assert.ok(content.text.includes('id="grooph-canvas"'));
  assert.ok(content.text.includes('id="loading_text"'));
  assert.ok(content.text.includes(`"origin":"${ORIGIN}"`));
  assert.ok(content.text.includes('"js":"grooph-app-abc123.js"'));
  assert.ok(content.text.includes("function metronomeQuery"));
  // The inline script parses (the view is a module; check its syntax as one).
  const script = content.text.match(/<script type="module">([\s\S]*)<\/script>/)[1];
  assert.doesNotThrow(() => new Function(`return async () => { ${script.replace(/^import .*$/gm, "")} }`));
});

test("resources/read without a web build is an error", async () => {
  const missing = { ASSETS: { fetch: async () => new Response("", { status: 404 }) } };
  const { error } = await rpc("resources/read", { uri: RESOURCE_URI }, missing);
  assert.equal(error.code, -32603);
});

test("notifications get 202, GET gets 405", async () => {
  const note = await handleRequest(
    new Request(`${ORIGIN}/mcp`, { method: "POST", body: JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) }),
    env,
  );
  assert.equal(note.status, 202);
  const get = await handleRequest(new Request(`${ORIGIN}/mcp`), env);
  assert.equal(get.status, 405);
});

test("llms.txt describes the tool's arguments, not written-out counter rhythms", () => {
  const llms = fs.readFileSync(path.join(here, "../crates/app/assets/seo/llms.txt"), "utf8");
  assert.ok(llms.includes("`show_metronome`") && llms.includes("`poly`"));
  assert.ok(!llms.includes("counter="), "llms.txt should offer poly, not counter");
});

// The same cases go through SharedLink::to_url in crates/link (test
// `links_of_the_mcp_tool_are_the_apps_own`), so both write one format.
test("the tool's queries are the app's link format (mcp/link-cases.json)", () => {
  const cases = JSON.parse(fs.readFileSync(path.join(here, "link-cases.json"), "utf8"));
  for (const { args, query } of cases) assert.equal(metronomeQuery(args), query, JSON.stringify(args));
});

test("the query is the app's own link format", () => {
  assert.equal(metronomeQuery({}), "v=2&ts=4/4");
  assert.equal(metronomeQuery({ ts: " 2+2+3/8 ", bpm: "90" }), "v=2&bpm=90&ts=2%2B2%2B3/8");
  assert.equal(metronomeQuery({ ts: "4/4", poly: " 3 : 2 " }), "v=2&ts=4/4&poly=3:2");
});
