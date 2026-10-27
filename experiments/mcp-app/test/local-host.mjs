// Local stand-in for an MCP Apps host, to try the spike without deploying.
//
//   node experiments/mcp-app/test/local-host.mjs [dist-dir]
//
// Serves the grooph web build (default `dist/`, from `trunk build --release`)
// on :8090 as "grooph.app", runs worker.js on :8787, and a host page on
// :8080 that calls the tool, reads the UI resource and shows it in an iframe
// on :8081. The view is served there with the CSP an MCP Apps host builds
// from the resource's `_meta.ui.csp`, following the spec's defaults; add
// `?wasm=1` to the host page URL to also allow 'wasm-unsafe-eval'. The host
// answers `ui/initialize`, sends the tool input and logs everything else.

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import worker from "../worker.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const dist = path.resolve(process.argv[2] || path.join(here, "../../../dist"));
const GROOPH = "http://localhost:8090";
const WORKER = "http://localhost:8787";
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".css": "text/css" };

// "grooph.app": the static build.
http
  .createServer((req, res) => {
    const p = path.join(dist, decodeURIComponent(new URL(req.url, GROOPH).pathname));
    const file = fs.existsSync(p) && fs.statSync(p).isDirectory() ? path.join(p, "index.html") : p;
    if (!file.startsWith(dist) || !fs.existsSync(file)) return res.writeHead(404).end();
    res.writeHead(200, { "Content-Type": TYPES[path.extname(file)] || "application/octet-stream" });
    fs.createReadStream(file).pipe(res);
  })
  .listen(8090);

// The Worker, through Node's fetch API types.
http
  .createServer(async (req, res) => {
    const body = await new Promise((r) => {
      const chunks = [];
      req.on("data", (c) => chunks.push(c)).on("end", () => r(Buffer.concat(chunks)));
    });
    const request = new Request(WORKER + req.url, {
      method: req.method,
      headers: req.headers,
      body: ["GET", "HEAD", "OPTIONS"].includes(req.method) ? undefined : body,
    });
    const response = await worker.fetch(request, { GROOPH_ORIGIN: GROOPH });
    res.writeHead(response.status, Object.fromEntries(response.headers));
    res.end(Buffer.from(await response.arrayBuffer()));
  })
  .listen(8787);

// The sandbox origin: serves the view HTML posted by the host page with the CSP from its metadata.
const views = new Map();
http
  .createServer((req, res) => {
    const url = new URL(req.url, "http://localhost:8081");
    if (req.method === "POST" && url.pathname === "/view") {
      const chunks = [];
      req.on("data", (c) => chunks.push(c)).on("end", () => {
        const id = String(views.size + 1);
        views.set(id, JSON.parse(Buffer.concat(chunks)));
        res.writeHead(200, { "Access-Control-Allow-Origin": "*" }).end(id);
      });
      return;
    }
    if (req.method === "OPTIONS") {
      return res.writeHead(204, { "Access-Control-Allow-Origin": "*", "Access-Control-Allow-Headers": "*" }).end();
    }
    const view = views.get(url.searchParams.get("id"));
    if (!view) return res.writeHead(404).end();
    const csp = view.csp || {};
    const res_ = (csp.resourceDomains || []).join(" ");
    const wasm = url.searchParams.get("wasm") === "1" ? " 'wasm-unsafe-eval'" : "";
    const policy = [
      "default-src 'none'",
      `script-src 'self' 'unsafe-inline'${wasm} ${res_}`,
      `style-src 'self' 'unsafe-inline' ${res_}`,
      `img-src 'self' data: ${res_}`,
      `font-src 'self' ${res_}`,
      `media-src 'self' data: ${res_}`,
      `connect-src ${(csp.connectDomains || []).join(" ") || "'none'"}`,
      "frame-src 'none'",
      "object-src 'none'",
      "base-uri 'self'",
    ].join("; ");
    res.writeHead(200, { "Content-Type": "text/html", "Content-Security-Policy": policy });
    res.end(view.html);
  })
  .listen(8081);

const hostPage = `<!DOCTYPE html><meta charset="utf-8"><title>local MCP Apps host</title>
<style>body{font:13px system-ui;margin:8px} iframe{width:100%;border:1px solid #888} pre{white-space:pre-wrap}</style>
<iframe id="view" sandbox="allow-scripts allow-same-origin allow-forms" allow="microphone; autoplay"></iframe>
<pre id="log"></pre>
<script type="module">
const log = (...a) => { document.getElementById("log").textContent += a.join(" ") + "\\n"; };
window.mcpLog = [];
let id = 1;
async function rpc(method, params) {
  const r = await fetch("${WORKER}/mcp", { method: "POST", headers: { "Content-Type": "application/json", Accept: "application/json, text/event-stream" },
    body: JSON.stringify({ jsonrpc: "2.0", id: id++, method, params }) });
  const j = await r.json();
  if (j.error) throw new Error(j.error.message);
  return j.result;
}
const q = new URLSearchParams(location.search);
await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: { extensions: { "io.modelcontextprotocol/ui": { mimeTypes: ["text/html;profile=mcp-app"] } } }, clientInfo: { name: "local-host", version: "0" } });
const { tools } = await rpc("tools/list", {});
const tool = tools[0];
const args = { rhythm: q.get("r") || "4/4 e e >s s s s q. e", bpm: 96 };
const callResult = await rpc("tools/call", { name: tool.name, arguments: args });
const { contents: [res] } = await rpc("resources/read", { uri: tool._meta.ui.resourceUri });
log("resource meta:", JSON.stringify(res._meta));
const viewId = await (await fetch("http://localhost:8081/view", { method: "POST", body: JSON.stringify({ html: res.text, csp: res._meta?.ui?.csp }) })).text();
const frame = document.getElementById("view");
window.addEventListener("message", (e) => {
  if (e.source !== frame.contentWindow) return;
  const m = e.data;
  window.mcpLog.push(m);
  const reply = (result) => frame.contentWindow.postMessage({ jsonrpc: "2.0", id: m.id, result }, "*");
  if (m.method === "ui/initialize") {
    reply({ protocolVersion: "2026-01-26", hostInfo: { name: "local-host", version: "0" }, hostCapabilities: {},
      hostContext: { theme: "light", platform: "web", displayMode: "inline", availableDisplayModes: ["inline", "fullscreen"], containerDimensions: { width: 800, maxHeight: 1000 } } });
  } else if (m.method === "ui/notifications/initialized") {
    frame.contentWindow.postMessage({ jsonrpc: "2.0", method: "ui/notifications/tool-input", params: { arguments: args } }, "*");
    frame.contentWindow.postMessage({ jsonrpc: "2.0", method: "ui/notifications/tool-result", params: callResult }, "*");
  } else if (m.method === "ui/notifications/size-changed") {
    frame.style.height = m.params.height + "px";
  } else if (m.method === "ui/update-model-context") {
    log("model context:", m.params.content?.[0]?.text);
    window.modelContext = m.params;
    reply({});
  } else if (m.id !== undefined) {
    log("request", m.method); reply({});
  }
});
frame.src = "http://localhost:8081/?id=" + viewId + (q.get("wasm") === "1" ? "&wasm=1" : "");
</script>`;
http
  .createServer((req, res) => res.writeHead(200, { "Content-Type": "text/html" }).end(hostPage))
  .listen(8080);

console.log(`grooph build ${dist} on ${GROOPH}, worker on ${WORKER}, host on http://localhost:8080 (add ?wasm=1 for 'wasm-unsafe-eval')`);
