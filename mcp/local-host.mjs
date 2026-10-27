// A local stand-in for an MCP Apps host, to try the metronome view without
// deploying:
//
//   trunk build --release
//   node mcp/local-host.mjs [dist-dir]
//
// Serves the web build (default `dist/`) on :8090 as grooph.app would, with
// the MCP server on /mcp and CORS on the app's files as in _headers. The
// host page on :8080 calls `show_metronome`, reads the view and shows it in
// an iframe on :8081, with the CSP the spec builds from the resource's
// `_meta.ui.csp`. Query parameters of the host page:
//
//   wasm=1          also allow 'wasm-unsafe-eval' (without it the view must
//                   show its message instead of the app)
//   theme=dark      the host's theme (light by default); window.setTheme(t)
//                   switches it later
//   w=360           width of the frame in CSS pixels
//   tall=1          room below the frame, so the page scrolls as a chat does
//   fullscreen=0    a host without fullscreen (else window.exitFullscreen()
//                   brings a fullscreen view back inline)
//   styles=0        no hostContext.styles (by default Claude-like color
//                   variables with light-dark(), as the spec suggests)
//   bpm, ts, click, poly, swing, su      the tool's arguments
//
// With GROOPH_ORIGIN set (e.g. http://localhost:8788 for `npx wrangler pages
// dev dist`), the host uses that deployment instead of serving dist itself:
// this runs the function as Cloudflare bundles it.
//
// The page logs the messages from the view; `window.modelContext` holds the
// last model context and `window.opened` the links the view asked to open.

import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { handleRequest } from "./server.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const dist = path.resolve(process.argv[2] || path.join(here, "../dist"));
const GROOPH = process.env.GROOPH_ORIGIN || "http://localhost:8090";
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".css": "text/css", ".json": "application/json" };

function staticFile(pathname) {
  const p = path.join(dist, decodeURIComponent(pathname));
  const file = fs.existsSync(p) && fs.statSync(p).isDirectory() ? path.join(p, "index.html") : p;
  if (!file.startsWith(dist) || !fs.existsSync(file)) return new Response("Not found", { status: 404 });
  const headers = { "Content-Type": TYPES[path.extname(file)] || "application/octet-stream" };
  if (path.basename(file).startsWith("grooph-app-") || pathname.startsWith("/embed/")) {
    headers["Access-Control-Allow-Origin"] = "*";
  }
  return new Response(fs.readFileSync(file), { headers });
}

const env = { ASSETS: { fetch: async (request) => staticFile(new URL(request.url).pathname) } };

async function readBody(req) {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  return Buffer.concat(chunks);
}

// "grooph.app": the build and the Pages Function, unless GROOPH_ORIGIN is one.
if (!process.env.GROOPH_ORIGIN) http
  .createServer(async (req, res) => {
    const url = new URL(req.url, GROOPH);
    let response;
    if (url.pathname === "/mcp") {
      const body = ["GET", "HEAD", "OPTIONS"].includes(req.method) ? undefined : await readBody(req);
      response = await handleRequest(new Request(url, { method: req.method, headers: req.headers, body }), env);
    } else {
      response = staticFile(url.pathname);
    }
    res.writeHead(response.status, Object.fromEntries(response.headers));
    res.end(Buffer.from(await response.arrayBuffer()));
  })
  .listen(8090);

// The sandbox origin: serves the view posted by the host page, with the CSP from its metadata.
const views = new Map();
http
  .createServer(async (req, res) => {
    const url = new URL(req.url, "http://localhost:8081");
    if (req.method === "OPTIONS") {
      return res.writeHead(204, { "Access-Control-Allow-Origin": "*", "Access-Control-Allow-Headers": "*" }).end();
    }
    if (req.method === "POST" && url.pathname === "/view") {
      const id = String(views.size + 1);
      views.set(id, JSON.parse(await readBody(req)));
      return res.writeHead(200, { "Access-Control-Allow-Origin": "*" }).end(id);
    }
    const view = views.get(url.searchParams.get("id"));
    if (!view) return res.writeHead(404).end();
    const csp = view.csp || {};
    const resources = (csp.resourceDomains || []).join(" ");
    const wasm = url.searchParams.get("wasm") === "1" ? " 'wasm-unsafe-eval'" : "";
    const policy = [
      "default-src 'none'",
      `script-src 'self' 'unsafe-inline'${wasm} ${resources}`,
      `style-src 'self' 'unsafe-inline' ${resources}`,
      `img-src 'self' data: ${resources}`,
      `font-src 'self' ${resources}`,
      `media-src 'self' data: ${resources}`,
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
<style>
  body { font: 13px system-ui; margin: 8px; }
  body.dark { background: #262624; color: #ddd; }
  iframe { display: block; border: 1px solid #888; border-radius: 12px; height: 200px; }
  pre { white-space: pre-wrap; }
</style>
<iframe id="view" sandbox="allow-scripts allow-same-origin allow-forms" allow="autoplay"></iframe>
<pre id="log"></pre>
<script type="module">
const log = (...a) => { document.getElementById("log").textContent += a.join(" ") + "\\n"; };
const q = new URLSearchParams(location.search);
let theme = q.get("theme") || "light";
document.body.classList.toggle("dark", theme === "dark");
const frame = document.getElementById("view");
frame.style.width = (q.get("w") || "720") + "px";
if (q.get("tall") === "1") document.body.style.paddingBottom = "2000px";
window.opened = [];
let id = 1;
async function rpc(method, params) {
  const r = await fetch("${GROOPH}/mcp", { method: "POST", headers: { "Content-Type": "application/json", Accept: "application/json, text/event-stream" },
    body: JSON.stringify({ jsonrpc: "2.0", id: id++, method, params }) });
  const j = await r.json();
  if (j.error) throw new Error(j.error.message);
  return j.result;
}
await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: { extensions: { "io.modelcontextprotocol/ui": { mimeTypes: ["text/html;profile=mcp-app"] } } }, clientInfo: { name: "local-host", version: "0" } });
const { tools } = await rpc("tools/list", {});
const tool = tools.find((t) => t.name === "show_metronome");
const args = {};
for (const key of ["ts", "click", "poly"]) if (q.has(key)) args[key] = q.get(key);
for (const key of ["bpm", "swing", "su"]) if (q.has(key)) args[key] = Number(q.get(key));
const callResult = await rpc("tools/call", { name: tool.name, arguments: args });
log("tool result:", callResult.content[0].text);
const { contents: [res] } = await rpc("resources/read", { uri: tool._meta.ui.resourceUri });
const viewId = await (await fetch("http://localhost:8081/view", { method: "POST", body: JSON.stringify({ html: res.text, csp: res._meta?.ui?.csp }) })).text();
const post = (m) => frame.contentWindow.postMessage({ jsonrpc: "2.0", ...m }, "*");
window.setTheme = (t) => {
  theme = t;
  document.body.classList.toggle("dark", t === "dark");
  post({ method: "ui/notifications/host-context-changed", params: { theme: t } });
};
// Color variables in the style of Claude's, light and dark in one value.
const styles = q.get("styles") === "0" ? undefined : { variables: {
  "--color-background-primary": "light-dark(#ffffff, #30302e)",
  "--color-background-secondary": "light-dark(#f5f4ed, #262624)",
  "--color-text-primary": "light-dark(#141413, #faf9f5)",
  "--color-text-secondary": "light-dark(#3d3d3a, #c2c0b6)",
  "--color-border-primary": "light-dark(#1f1e1d26, #dedcd126)",
} };
const modes = q.get("fullscreen") === "0" ? ["inline"] : ["inline", "fullscreen"];
const inlineWidth = frame.style.width;
const display = (mode) => {
  const full = mode === "fullscreen";
  frame.style.width = full ? "100%" : inlineWidth;
  const height = full ? 700 : undefined;
  if (full) frame.style.height = height + "px";
  post({ method: "ui/notifications/host-context-changed", params: { displayMode: mode,
    containerDimensions: full ? { width: frame.clientWidth, height } : { width: frame.clientWidth, maxHeight: 1000 } } });
};
window.exitFullscreen = () => display("inline");
window.addEventListener("message", (e) => {
  if (e.source !== frame.contentWindow) return;
  const m = e.data;
  const reply = (result) => post({ id: m.id, result });
  if (m.method === "ui/initialize") {
    reply({ protocolVersion: "2026-01-26", hostInfo: { name: "local-host", version: "0" }, hostCapabilities: { openLinks: {} },
      hostContext: { theme, styles, platform: "web", displayMode: "inline", availableDisplayModes: modes, containerDimensions: { width: frame.clientWidth, maxHeight: 1000 } } });
  } else if (m.method === "ui/notifications/initialized") {
    post({ method: "ui/notifications/tool-input", params: { arguments: args } });
    post({ method: "ui/notifications/tool-result", params: callResult });
  } else if (m.method === "ui/notifications/size-changed") {
    frame.style.height = m.params.height + "px";
  } else if (m.method === "ui/update-model-context") {
    log("model context:", m.params.content?.[0]?.text);
    window.modelContext = m.params;
    reply({});
  } else if (m.method === "ui/request-display-mode") {
    const mode = modes.includes(m.params.mode) ? m.params.mode : "inline";
    log("display mode:", mode);
    reply({ mode });
    display(mode);
  } else if (m.method === "ui/open-link") {
    log("open link:", m.params.url);
    window.opened.push(m.params.url);
    reply({});
  } else if (m.id !== undefined) {
    log("request", m.method);
    reply({});
  }
});
frame.src = "http://localhost:8081/?id=" + viewId + (q.get("wasm") === "1" ? "&wasm=1" : "");
</script>`;
http.createServer((req, res) => res.writeHead(200, { "Content-Type": "text/html" }).end(hostPage)).listen(8080);

console.log(`grooph build ${dist} and /mcp on ${GROOPH}; host on http://localhost:8080 (add ?wasm=1 for 'wasm-unsafe-eval')`);
