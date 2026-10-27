// Spike: does grooph run as an MCP App (https://modelcontextprotocol.io/extensions/apps/overview)?
//
// A stateless MCP server (Streamable HTTP, JSON responses) as a Cloudflare
// Worker without dependencies. One tool, `grooph_mcp_test`, opens a view
// that checks step by step what the host's sandbox allows: the postMessage
// handshake, compiling a tiny WebAssembly module, loading the real grooph
// build from grooph.app (proxied through this Worker, which adds CORS) and
// playing audio. The results show in the view and go back to the model via
// `ui/update-model-context`.

const RESOURCE_URI = "ui://grooph/test.html";
const RESOURCE_MIME = "text/html;profile=mcp-app";
const SERVER_INFO = { name: "grooph-mcp-spike", version: "0.1.0" };

const TOOL = {
  name: "grooph_mcp_test",
  title: "grooph MCP App test",
  description:
    "Shows grooph, a rhythm trainer, inside the chat to test whether it runs as an MCP App. " +
    "Optionally pass a rhythm in grooph's text notation (e.g. `4/4 q e e >s s s s q q`, " +
    "see https://grooph.app/llms.txt), a tempo and a time signature.",
  inputSchema: {
    type: "object",
    properties: {
      rhythm: { type: "string", description: "Rhythm in grooph text notation, e.g. `4/4 q e e q. e`" },
      bpm: { type: "integer", minimum: 20, maximum: 300, description: "Tempo in quarter notes per minute" },
      ts: { type: "string", description: "Time signature, e.g. `7/8`" },
    },
  },
  _meta: { ui: { resourceUri: RESOURCE_URI } },
};

const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, DELETE, OPTIONS",
  "Access-Control-Allow-Headers": "*",
  "Access-Control-Expose-Headers": "Mcp-Session-Id",
};

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const groophOrigin = (env.GROOPH_ORIGIN || "https://grooph.app").replace(/\/$/, "");
    if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: CORS });
    if (url.pathname === "/mcp") {
      if (request.method !== "POST") return new Response("Method not allowed", { status: 405, headers: CORS });
      return handleMcp(request, url.origin, groophOrigin);
    }
    if (url.pathname.startsWith("/app/")) return proxyAsset(url.pathname.slice("/app/".length), groophOrigin);
    if (url.pathname === "/") {
      return new Response(`grooph MCP spike. MCP endpoint: ${url.origin}/mcp\n`, { headers: CORS });
    }
    return new Response("Not found", { status: 404, headers: CORS });
  },
};

async function handleMcp(request, selfOrigin, groophOrigin) {
  let msg;
  try {
    msg = await request.json();
  } catch {
    return json({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "Parse error" } });
  }
  if (Array.isArray(msg)) {
    const replies = (await Promise.all(msg.map((m) => dispatch(m, selfOrigin, groophOrigin)))).filter(Boolean);
    return replies.length ? json(replies) : new Response(null, { status: 202, headers: CORS });
  }
  const reply = await dispatch(msg, selfOrigin, groophOrigin);
  return reply ? json(reply) : new Response(null, { status: 202, headers: CORS });
}

async function dispatch(msg, selfOrigin, groophOrigin) {
  // Notifications (no id) get no answer.
  if (msg.id === undefined || msg.id === null) return null;
  const ok = (result) => ({ jsonrpc: "2.0", id: msg.id, result });
  const fail = (code, message) => ({ jsonrpc: "2.0", id: msg.id, error: { code, message } });
  switch (msg.method) {
    case "initialize":
      return ok({
        protocolVersion: msg.params?.protocolVersion || "2025-06-18",
        capabilities: {
          tools: {},
          resources: {},
          extensions: { "io.modelcontextprotocol/ui": {} },
        },
        serverInfo: SERVER_INFO,
      });
    case "ping":
      return ok({});
    case "tools/list":
      return ok({ tools: [TOOL] });
    case "tools/call": {
      if (msg.params?.name !== TOOL.name) return fail(-32602, `Unknown tool ${msg.params?.name}`);
      const args = msg.params.arguments || {};
      return ok({
        content: [
          {
            type: "text",
            text:
              "The grooph test view is shown to the user. It checks WebAssembly, loading grooph and audio " +
              "inside the sandbox and reports the results back as model context. " +
              `Arguments: ${JSON.stringify(args)}`,
          },
        ],
      });
    }
    case "resources/list":
      return ok({
        resources: [{ uri: RESOURCE_URI, name: "grooph test view", mimeType: RESOURCE_MIME }],
      });
    case "resources/templates/list":
      return ok({ resourceTemplates: [] });
    case "resources/read": {
      if (msg.params?.uri !== RESOURCE_URI) return fail(-32602, `Unknown resource ${msg.params?.uri}`);
      const assets = await groophAssets(groophOrigin);
      return ok({
        contents: [
          {
            uri: RESOURCE_URI,
            mimeType: RESOURCE_MIME,
            text: viewHtml(selfOrigin, groophOrigin, assets),
            _meta: {
              ui: {
                csp: { connectDomains: [selfOrigin], resourceDomains: [selfOrigin] },
                permissions: { microphone: {} },
                prefersBorder: true,
              },
            },
          },
        ],
      });
    }
    default:
      return fail(-32601, `Method not found: ${msg.method}`);
  }
}

// The hashed file names of the current grooph build, read from its index.html.
async function groophAssets(groophOrigin) {
  try {
    const res = await fetch(`${groophOrigin}/`, { cf: { cacheTtl: 60 } });
    const html = await res.text();
    const js = html.match(/grooph-app-[0-9a-f]+\.js/)?.[0];
    const wasm = html.match(/grooph-app-[0-9a-f]+_bg\.wasm/)?.[0];
    return js && wasm ? { js, wasm } : { error: `no grooph-app-*.js/_bg.wasm in ${groophOrigin}/ (HTTP ${res.status})` };
  } catch (e) {
    return { error: `fetching ${groophOrigin}/ failed: ${e}` };
  }
}

// Serves grooph's JS glue, WASM and wasm-bindgen snippets from this origin
// with CORS, which grooph.app does not send.
async function proxyAsset(path, groophOrigin) {
  if (!/^(grooph-app-[0-9a-f]+(\.js|_bg\.wasm)|snippets\/[\w.\/-]+\.js)$/.test(path) || path.includes("..")) {
    return new Response("Not found", { status: 404, headers: CORS });
  }
  const res = await fetch(`${groophOrigin}/${path}`, { cf: { cacheEverything: true, cacheTtl: 86400 } });
  const headers = new Headers(CORS);
  headers.set("Content-Type", path.endsWith(".wasm") ? "application/wasm" : "text/javascript");
  headers.set("Cache-Control", "public, max-age=86400");
  return new Response(res.body, { status: res.status, headers });
}

function json(body) {
  return new Response(JSON.stringify(body), { headers: { ...CORS, "Content-Type": "application/json" } });
}

function viewHtml(selfOrigin, groophOrigin, assets) {
  const config = JSON.stringify({ selfOrigin, groophOrigin, assets });
  return `<!DOCTYPE html>
<html lang="de">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>grooph MCP test</title>
<style>
  body { margin: 0; font: 13px/1.4 system-ui, sans-serif; }
  #report { padding: 8px 12px; }
  #report li { list-style: none; }
  #report ul { margin: 4px 0; padding: 0; }
  .ok::before { content: "✅ "; } .fail::before { content: "❌ "; } .wait::before { content: "⏳ "; }
  #stage { position: relative; height: 520px; background: #909090; }
  #stage canvas { position: absolute; inset: 0; width: 100%; height: 100%; display: block; }
  #loading_text { position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%); color: #f0f0f0; }
  button { font: inherit; }
  code { font-size: 12px; }
</style>
</head>
<body>
<div id="report">
  <strong>grooph als MCP App: Test</strong>
  <ul id="checks"></ul>
  <button id="beep">Audio-Test: Klick abspielen</button>
  <div id="env"></div>
</div>
<div id="stage"><canvas id="grooph-canvas"></canvas><div id="loading_text">grooph lädt…</div></div>
<script type="module">
const CONFIG = ${config};
const results = {};
const checks = document.getElementById("checks");
const violations = [];

function check(key, label) {
  let li = document.getElementById("c-" + key);
  if (!li) { li = document.createElement("li"); li.id = "c-" + key; checks.appendChild(li); }
  return (state, detail) => {
    li.className = state;
    li.textContent = label + (detail ? ": " + detail : "");
    results[key] = { state, detail: detail || "" };
    scheduleReport();
  };
}

document.addEventListener("securitypolicyviolation", (e) => {
  violations.push(e.violatedDirective + " blocked " + (e.blockedURI || "(inline)"));
  check("csp", "CSP-Verstöße")("fail", violations.join("; "));
});

// --- Minimal MCP Apps bridge over postMessage (no SDK) ---
let nextId = 1;
const pending = new Map();
const handlers = {};
window.addEventListener("message", (event) => {
  const m = event.data;
  if (!m || m.jsonrpc !== "2.0") return;
  if (m.id !== undefined && pending.has(m.id) && !m.method) {
    const { resolve, reject } = pending.get(m.id);
    pending.delete(m.id);
    m.error ? reject(new Error(m.error.message || JSON.stringify(m.error))) : resolve(m.result);
  } else if (m.method && handlers[m.method]) {
    handlers[m.method](m.params || {});
  } else if (m.method && m.id !== undefined) {
    // A request we do not handle (e.g. ui/resource-teardown): answer with an empty result.
    window.parent.postMessage({ jsonrpc: "2.0", id: m.id, result: {} }, "*");
  }
});
function request(method, params) {
  const id = nextId++;
  window.parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*");
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    setTimeout(() => { if (pending.delete(id)) reject(new Error("timeout")); }, 10000);
  });
}
function notify(method, params) {
  window.parent.postMessage({ jsonrpc: "2.0", method, params }, "*");
}

let initialized = false;
let reportTimer = null;
function scheduleReport() {
  if (!initialized) return;
  clearTimeout(reportTimer);
  reportTimer = setTimeout(() => {
    const lines = Object.entries(results).map(([k, v]) => k + ": " + v.state + (v.detail ? " (" + v.detail + ")" : ""));
    request("ui/update-model-context", {
      content: [{ type: "text", text: "grooph MCP App test results:\\n" + lines.join("\\n") }],
      structuredContent: results,
    }).catch(() => {});
  }, 800);
}

function reportSize() {
  notify("ui/notifications/size-changed", {
    width: Math.ceil(document.documentElement.scrollWidth),
    height: Math.ceil(document.documentElement.scrollHeight),
  });
}

// --- 1. Handshake ---
const handshake = check("handshake", "Handshake mit dem Host");
handshake("wait");
let toolArgs = null;
const toolInput = new Promise((resolve) => {
  handlers["ui/notifications/tool-input"] = (p) => { toolArgs = p.arguments || {}; resolve(toolArgs); };
  setTimeout(() => resolve(null), 3000);
});
try {
  const init = await request("ui/initialize", {
    appInfo: { name: "grooph-mcp-spike", version: "0.1.0" },
    appCapabilities: { availableDisplayModes: ["inline", "fullscreen"] },
    protocolVersion: "2026-01-26",
  });
  notify("ui/notifications/initialized", {});
  initialized = true;
  const ctx = init.hostContext || {};
  handshake("ok", (init.hostInfo?.name || "?") + " " + (init.hostInfo?.version || "") +
    ", theme " + (ctx.theme || "?") + ", " + (ctx.platform || "?") +
    ", container " + JSON.stringify(ctx.containerDimensions || {}) +
    ", modes " + JSON.stringify(ctx.availableDisplayModes || []));
  new ResizeObserver(reportSize).observe(document.documentElement);
  reportSize();
  if ((ctx.availableDisplayModes || []).includes("fullscreen")) {
    const b = document.createElement("button");
    b.textContent = "Vollbild";
    b.onclick = () => request("ui/request-display-mode", { mode: "fullscreen" }).catch((e) => alert(e));
    document.getElementById("report").appendChild(b);
  }
} catch (e) {
  handshake("fail", String(e));
}
document.getElementById("env").innerHTML =
  "<code>origin " + location.origin + " · WebAssembly " + (typeof WebAssembly) +
  " · AudioContext " + (typeof AudioContext) + "</code>";

// --- 2. Tiny WebAssembly module (add two i32) ---
const tiny = check("wasm", "WebAssembly kompilieren (Mini-Modul)");
try {
  const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,7,1,96,2,127,127,1,127,3,2,1,0,7,7,1,3,97,100,100,0,0,10,9,1,7,0,32,0,32,1,106,11]);
  const { instance } = await WebAssembly.instantiate(bytes);
  tiny(instance.exports.add(2, 3) === 5 ? "ok" : "fail", "2 + 3 = " + instance.exports.add(2, 3));
} catch (e) {
  tiny("fail", String(e));
}

// --- 3. Real grooph build ---
const glue = check("grooph_js", "grooph JS laden");
const app = check("grooph_wasm", "grooph WASM starten");
if (CONFIG.assets.error) {
  glue("fail", CONFIG.assets.error);
} else {
  const args = await toolInput;
  try {
    // grooph reads its link parameters from the address bar once at start.
    const q = new URLSearchParams({ v: "2" });
    if (args?.bpm) q.set("bpm", String(args.bpm));
    if (args?.rhythm) q.set("r", args.rhythm);
    if (args?.ts) q.set("ts", args.ts);
    if ([...q.keys()].length > 1) history.replaceState(null, "", "?" + q);
  } catch (e) {
    check("link", "Link-Parameter setzen")("fail", String(e));
  }
  try {
    glue("wait");
    const mod = await import(CONFIG.selfOrigin + "/app/" + CONFIG.assets.js);
    glue("ok", CONFIG.assets.js);
    app("wait");
    await mod.default({ module_or_path: CONFIG.selfOrigin + "/app/" + CONFIG.assets.wasm });
    // main() starts eframe asynchronously; it removes the loading text once running.
    const started = await new Promise((resolve) => {
      const t0 = Date.now();
      const poll = setInterval(() => {
        if (!document.getElementById("loading_text")) { clearInterval(poll); resolve(true); }
        else if (Date.now() - t0 > 15000) { clearInterval(poll); resolve(false); }
      }, 100);
    });
    app(started ? "ok" : "fail", started ? "läuft (" + CONFIG.groophOrigin + ")" : "eframe nicht gestartet, siehe Konsole");
  } catch (e) {
    (results.grooph_js?.state === "ok" ? app : glue)("fail", String(e));
  }
}

// --- 4. Audio (needs a click) ---
const audio = check("audio", "Audio");
audio("wait", "Knopf drücken, dann in grooph ▶");
document.getElementById("beep").onclick = async () => {
  try {
    const ctx = new AudioContext();
    await ctx.resume();
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0.3, ctx.currentTime);
    gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.15);
    osc.connect(gain).connect(ctx.destination);
    osc.start();
    osc.stop(ctx.currentTime + 0.15);
    audio(ctx.state === "running" ? "ok" : "fail", "AudioContext " + ctx.state + " (hörbar?)");
  } catch (e) {
    audio("fail", String(e));
  }
};
</script>
</body>
</html>`;
}
