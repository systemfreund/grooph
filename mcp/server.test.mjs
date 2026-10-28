// Tests of the MCP server without Cloudflare: node --test mcp/server.test.mjs
// They need the link check's module, built with
//   cargo build --release --target wasm32-unknown-unknown -p grooph-link-check
// (or LINK_CHECK_WASM pointing at it).

import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { linkChecker } from "./link-check.js";
import { metronomeQuery } from "./link.js";
import { RESOURCE_MIME, RESOURCE_URI, handleRequest, showMetronome } from "./server.js";

const ORIGIN = "https://feature-x.grooph.pages.dev";
const here = path.dirname(fileURLToPath(import.meta.url));

const wasmPath =
  process.env.LINK_CHECK_WASM || path.join(here, "../target/wasm32-unknown-unknown/release/grooph_link_check.wasm");
assert.ok(fs.existsSync(wasmPath), `build the link check first: ${wasmPath} is missing (see the top of this file)`);
const checkLink = linkChecker(fs.readFileSync(wasmPath));

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
    checkLink,
  );
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("Access-Control-Allow-Origin"), "*");
  return response.json();
}

test("initialize advertises tools, resources and MCP Apps", async () => {
  const { result } = await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "t", version: "0" } });
  assert.equal(result.protocolVersion, "2025-06-18");
  const { result: newer } = await rpc("initialize", { protocolVersion: "2099-01-01", capabilities: {} });
  assert.equal(newer.protocolVersion, "2025-11-25");
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
  for (const args of [{ bpm: 500 }, { swing: 90 }, { su: 16 }, { swing: 60, su: 4 }, { ts: "seven" }, { poly: "three" }, { poly: "2:4" }, { poly: "17:4" }, { tempo: 90 }]) {
    const { result } = await rpc("tools/call", { name: "show_metronome", arguments: args });
    assert.equal(result.isError, true, JSON.stringify(args));
  }
  const { result: counter } = await rpc("tools/call", { name: "show_metronome", arguments: { counter: "t4 t4 t4 t4 t4 t4" } });
  assert.match(counter.content[0].text, /pass the polyrhythm as poly/);
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
  // The skeleton inside it, which eframe removes once the app runs.
  for (const part of ["sk-row", "sk-staff", "sk-ring", "sk-play", "sk-bar"]) assert.ok(content.text.includes(`class="${part}"`), part);
  assert.ok(content.text.includes(`"origin":"${ORIGIN}"`));
  assert.ok(content.text.includes('"js":"grooph-app-abc123.js"'));
  // The link builder is imported, not inlined: a bundled function's source
  // carries esbuild helpers (__name) the view does not have.
  assert.ok(content.text.includes("/embed/link.js"));
  assert.ok(!content.text.includes("function metronomeQuery"));
  assert.ok(!/__name/.test(content.text));
  // The inline script parses (the view is a module; check its syntax as one).
  const script = content.text.match(/<script type="module">([\s\S]*)<\/script>/)[1];
  assert.doesNotThrow(() => new Function(`return async () => { ${script.replace(/^import .*$/gm, "")} }`));
});

test("the view reads the host's colors as #rrggbbaa", async () => {
  const { result } = await rpc("resources/read", { uri: RESOURCE_URI });
  const script = result.contents[0].text.match(/<script type="module">([\s\S]*)<\/script>/)[1];
  const source = script.match(/function cssColorToHex[\s\S]*?\n}\n/)[0];
  const cssColorToHex = new Function(source + "return cssColorToHex")();
  assert.equal(cssColorToHex("rgb(20, 20, 19)"), "#141413ff");
  assert.equal(cssColorToHex("rgba(255, 0, 16, 0.5)"), "#ff001080");
  assert.equal(cssColorToHex("rgb(10 20 30 / 50%)"), "#0a141e80");
  assert.equal(cssColorToHex("transparent"), null);
});

test("tools/call checks notation and polyrhythm by the app's parser", async () => {
  const call = async (args) => (await rpc("tools/call", { name: "show_metronome", arguments: args })).result;
  const tooLong = await call({ ts: "7/8", click: "q q q q" });
  assert.equal(tooLong.isError, true);
  assert.match(tooLong.content[0].text, /click: measure 1 is too long/);
  const poly = await call({ ts: "4/4", poly: "7:5" });
  assert.equal(poly.isError, true);
  assert.match(poly.content[0].text, /poly 7:5 does not fit 4\/4/);
  assert.equal((await call({ ts: "99/4" })).isError, true);
  const fine = await call({ ts: "7/8", bpm: 140, click: ">q r:q. (q)", poly: "3:7" });
  assert.ok(!fine.isError, fine.content[0].text);
});

test("the link check survives its memory growing", () => {
  // A query (3 MB) far beyond the memory the module starts with.
  const long = "v=2&ts=4/4&click=" + "q+".repeat(1500000);
  assert.match(checkLink(long), /click/);
  assert.equal(checkLink("v=2&ts=4/4"), null);
});

test("the tool does not answer without its link check", () => {
  assert.throws(() => showMetronome({ ts: "4/4" }, ORIGIN), /checkLink/);
  assert.ok(!showMetronome({ ts: "4/4" }, ORIGIN, checkLink).isError);
});

test("every link case opens in the app", () => {
  const cases = JSON.parse(fs.readFileSync(path.join(here, "link-cases.json"), "utf8"));
  for (const { query } of cases) assert.equal(checkLink(query), null, query);
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
