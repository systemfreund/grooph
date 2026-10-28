// Tests of the MCP server without Cloudflare: node --test mcp/server.test.mjs
// They need the link check's module, built with
//   cargo build --release --target wasm32-unknown-unknown -p grooph-link-check
// (CARGO_TARGET_DIR is honored, or LINK_CHECK_WASM points at the module).

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

const targetDir = process.env.CARGO_TARGET_DIR || path.join(here, "../target");
const wasmPath =
  process.env.LINK_CHECK_WASM || path.join(targetDir, "wasm32-unknown-unknown/release/grooph_link_check.wasm");
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
  assert.deepEqual(Object.keys(tool.inputSchema.properties).sort(), ["bpm", "click", "groups", "poly", "su", "subdivision", "swing", "ts"]);
  // Describes, points nowhere else, stays short; the details are the parameters'.
  assert.doesNotMatch(tool.description, /llms\.txt|https?:\/\//);
  assert.ok(tool.description.length < 500, tool.description.length);
  assert.match(tool.inputSchema.properties.poly.description, /"3:2"/);
  assert.match(tool.inputSchema.properties.click.description, />q r:q\. \(q\)/);
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
  for (const args of [{ bpm: 500 }, { swing: 90 }, { su: 16 }, { swing: 60, su: 4 }, { ts: "seven" }, { poly: "three" }, { poly: "2:4" }, { poly: "17:4" }, { tempo: 90 }, { ts: "4/4", click: "q ".repeat(600) }, { poly: " ".repeat(20) + "3:2" }]) {
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

test("the result says what the metronome shows", async () => {
  const call = async (args) => (await rpc("tools/call", { name: "show_metronome", arguments: args })).result;
  // The grouping from groups, not accents in the usual 2+3+2.
  const grouped = await call({ ts: "7/8", groups: "3+2+2", bpm: 140 });
  assert.ok(!grouped.isError, grouped.content[0].text);
  assert.ok(grouped.structuredContent.link.includes("ts=7/8&groups=3%2B2%2B2"), grouped.structuredContent.link);
  assert.match(grouped.structuredContent.shown, /^7\/8 grouped 3\+2\+2 \(groups start on eighths 1, 4, 6\) at 140 BPM/);
  assert.ok(grouped.content[0].text.includes(grouped.structuredContent.shown));
  const accents = await call({ ts: "7/8", click: ">e e e >e e >e e" });
  assert.match(accents.structuredContent.shown, /grouped 2\+3\+2/);
  assert.match(accents.structuredContent.shown, /strong on eighths 1, 4, 6/);
});

test("a subdivision is the picker's bar, named in the result", async () => {
  const call = async (args) => (await rpc("tools/call", { name: "show_metronome", arguments: args })).result;
  const sixteenths = await call({ ts: "7/8", groups: "3+2+2", bpm: 140, subdivision: "s" });
  assert.ok(!sixteenths.isError, sixteenths.content[0].text);
  assert.ok(sixteenths.structuredContent.link.endsWith("&subdivision=s"), sixteenths.structuredContent.link);
  assert.match(sixteenths.structuredContent.shown, /subdivision sixteenths \(subdivision=s\): click bar >s \(s\) \(s\)/);
  // Written out by hand, the bar is one of its own, and the result says what the picker offers.
  const byHand = await call({ ts: "7/8", groups: "3+2+2", click: ">s s s s s s >s s s s >s s s s" });
  assert.match(byHand.structuredContent.shown, /a click bar of its own .*; the subdivision picker offers none, e, s, t16/);
  const both = await call({ ts: "4/4", subdivision: "s", click: "q q q q" });
  assert.equal(both.isError, true);
  assert.match(both.content[0].text, /either subdivision or click/);
  const wrong = await call({ ts: "7/16", subdivision: "e" });
  assert.equal(wrong.isError, true);
  assert.match(wrong.content[0].text, /subdivision e does not fit 7\/16: it takes none, s, th, t32/);
});

test("the subdivision description names what each meter takes", async () => {
  const { result } = await rpc("tools/list", {});
  const text = result.tools[0].inputSchema.properties.subdivision.description;
  const named = [];
  for (const [, unit, list] of text.matchAll(/in x\/(\d+) ([^;.]+)/g)) {
    for (const [, code] of list.matchAll(/(?:^|, )([a-z0-9:]+)(?= \(|,|$)/g)) named.push([code, unit]);
  }
  assert.ok(named.length >= 12, JSON.stringify(named));
  for (const [code, unit] of named) {
    const query = metronomeQuery({ ts: `${unit === "16" ? 7 : 4}/${unit}`, subdivision: code });
    assert.equal(checkLink(query).problem, null, `${code} in x/${unit}`);
  }
  for (const ts of ["4/4", "7/8", "7/16"]) assert.equal(checkLink(metronomeQuery({ ts, subdivision: "none" })).problem, null, ts);
});

test("groups are checked against ts", async () => {
  const call = async (args) => (await rpc("tools/call", { name: "show_metronome", arguments: args })).result;
  const error = async (args, pattern) => {
    const result = await call(args);
    assert.equal(result.isError, true, JSON.stringify(args));
    assert.match(result.content[0].text, pattern);
  };
  await error({ ts: "7/8", groups: "3+3+2" }, /add up to 8 beats, but 7\/8 has 7/);
  await error({ ts: "2+2+3/8" }, /pass the grouping as groups/);
  await error({ ts: "6/8", groups: "2+2+2" }, /which takes 3\+3/);
  await error({ ts: "7/8", groups: "3-2-2" }, /groups must be/);
  // The meter does not take this grouping: the app's parser says so.
  await error({ ts: "6/8", groups: "2+2+2" }, /do not open in grooph/);
  assert.equal(metronomeQuery({ groups: "3 + 2 + 2", ts: "7/8" }), "v=2&ts=7/8&groups=3%2B2%2B2");
  assert.ok(!(await call({ ts: "7/16", groups: "2+2+3" })).isError);
});

test("the link check survives its memory growing", () => {
  // A query (3 MB) far beyond the memory the module starts with.
  const long = "v=2&ts=4/4&click=" + "q+".repeat(1500000);
  assert.match(checkLink(long).problem, /click/);
  assert.equal(checkLink("v=2&ts=4/4").problem, null);
});

test("the tool does not answer without its link check", () => {
  assert.throws(() => showMetronome({ ts: "4/4" }, ORIGIN), /checkLink/);
  assert.ok(!showMetronome({ ts: "4/4" }, ORIGIN, checkLink).isError);
});

test("the ts and groups descriptions match the meters the app takes", async () => {
  const { result } = await rpc("tools/list", {});
  const text = result.tools[0].inputSchema.properties.ts.description;
  for (const ts of text.match(/\b\d+\/\d+\b/g)) assert.equal(checkLink(metronomeQuery({ ts })).problem, null, ts);
  // Every grouping groups names opens in the meter it names.
  const groups = result.tools[0].inputSchema.properties.groups.description;
  const named = [...groups.matchAll(/((?:\d+\+)+\d+(?: or (?:\d+\+)+\d+)*)(?: and others)? in (\d+\/\d+)/g)];
  assert.ok(named.length >= 5, groups);
  for (const [, list, ts] of named) {
    for (const g of list.split(" or ")) assert.equal(checkLink(metronomeQuery({ ts, groups: g })).problem, null, `${g} in ${ts}`);
  }
  // Each range's ends open, one beat past them does not.
  const ranges = [...text.matchAll(/(\d+)-(\d+) (?:beats )?over (\d+)/g)];
  assert.equal(ranges.length, 3, text);
  for (const [, low, high, unit] of ranges) {
    for (const beats of [low, high]) assert.equal(checkLink(`v=2&ts=${beats}/${unit}`).problem, null, `${beats}/${unit}`);
    for (const beats of [Number(low) - 1, Number(high) + 1]) {
      if (beats > 0) assert.notEqual(checkLink(`v=2&ts=${beats}/${unit}`).problem, null, `${beats}/${unit}`);
    }
  }
});

test("every link case opens in the app", () => {
  const cases = JSON.parse(fs.readFileSync(path.join(here, "link-cases.json"), "utf8"));
  for (const { query } of cases) assert.equal(checkLink(query).problem, null, query);
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
  assert.equal(metronomeQuery({ ts: " 7/8 ", groups: " 2+2+3 ", bpm: "90" }), "v=2&bpm=90&ts=7/8&groups=2%2B2%2B3");
  assert.equal(metronomeQuery({ ts: "4/4", poly: " 3 : 2 " }), "v=2&ts=4/4&poly=3:2");
});
