// The view of `show_metronome` (`ui://grooph/metronome.html`): a page the
// MCP Apps host runs in a sandboxed iframe on its own origin. It talks the
// MCP Apps protocol with the host over postMessage (JSON-RPC 2.0, spec
// 2026-01-26), puts the tool's link into the address bar with `embed=mcp`
// and the host's theme, and starts the web build of `origin` in embed mode
// (crates/app/src/embed.rs). `window.groophHost` is its bridge to the app:
// the app reports its settings through `updateModelContext`, opens links
// through `openLink`, and reads `appUrl` and `theme`.

import { metronomeQuery } from "./link.js";

/** Inline height of the view, in CSS pixels: card, staff, ring and play
 * button, with the options folded. Less if the host allows less. */
export const VIEW_HEIGHT = 520;

/** The view's HTML for the web build at `origin` (`js` and `wasm` are the
 * hashed file names from embed.json). */
export function viewHtml({ origin, js, wasm }) {
  // Safe inside <script>: no "</" can close it early.
  const config = JSON.stringify({ origin, js, wasm, height: VIEW_HEIGHT }).replace(/</g, "\\u003c");
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, user-scalable=no">
<meta name="color-scheme" content="light dark">
<title>grooph metronome</title>
<style>
  html, body { margin: 0; padding: 0; overflow: hidden; touch-action: manipulation; }
  body { font: 15px/1.4 system-ui, sans-serif; background: #f8f8f8; color: #222; }
  html.dark body { background: #1b1b1b; color: #ddd; }
  #stage { position: relative; height: ${VIEW_HEIGHT}px; }
  canvas { position: absolute; inset: 0; width: 100%; height: 100%; display: block; outline: none; }
  #loading_text { position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%); opacity: 0.7; }
  #fallback { padding: 16px 20px; }
  #fallback button { font: inherit; padding: 8px 16px; margin-top: 4px; cursor: pointer; }
  #fallback code { font-size: 13px; word-break: break-all; }
</style>
</head>
<body>
<div id="stage"><canvas id="grooph-canvas"></canvas><div id="loading_text">Loading the metronome…</div></div>
<div id="fallback" hidden>
  <p><strong>The metronome cannot run here.</strong> <span id="reason"></span></p>
  <p><button id="open">Open in grooph</button></p>
  <p><code id="link"></code></p>
</div>
<!-- Keeps the click audible with the iOS ring/silent switch on, as in the app's index.html. -->
<script src="${origin}/ios-audio.js"></script>
<script type="module">
const CONFIG = ${config};
${metronomeQuery.toString()}

// --- MCP Apps protocol over postMessage ---
let nextId = 1;
let connected = false;
let latestContext = null;
const pending = new Map();
const handlers = {};
window.addEventListener("message", (event) => {
  if (event.source !== window.parent) return;
  const m = event.data;
  if (!m || m.jsonrpc !== "2.0") return;
  if (m.method === undefined && pending.has(m.id)) {
    const { resolve, reject } = pending.get(m.id);
    pending.delete(m.id);
    m.error ? reject(new Error(m.error.message || "error")) : resolve(m.result);
  } else if (m.method && m.id !== undefined) {
    const answer = m.method === "ui/resource-teardown" ? { result: {} } : { error: { code: -32601, message: "Method not found" } };
    window.parent.postMessage({ jsonrpc: "2.0", id: m.id, ...answer }, "*");
  } else if (m.method) {
    handlers[m.method]?.(m.params || {});
  }
});
function request(method, params, timeout = 10000) {
  const id = nextId++;
  window.parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*");
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    setTimeout(() => pending.delete(id) && reject(new Error("timeout")), timeout);
  });
}
function notify(method, params) {
  window.parent.postMessage({ jsonrpc: "2.0", method, params }, "*");
}

// --- Theme and size ---
const systemDark = () => window.matchMedia?.("(prefers-color-scheme: dark)").matches;
let theme = systemDark() ? "dark" : "light";
function setTheme(next) {
  if (next !== "light" && next !== "dark") return;
  theme = next;
  document.documentElement.classList.toggle("dark", theme === "dark");
  if (window.groophHost) {
    window.groophHost.theme = theme;
    window.groophHost.repaint?.();
  }
}
function fitHeight(dimensions) {
  const limit = dimensions?.height ?? dimensions?.maxHeight;
  const height = Math.max(240, Math.min(CONFIG.height, Number.isFinite(limit) ? limit : CONFIG.height));
  document.getElementById("stage").style.height = height + "px";
}
function reportSize() {
  if (!connected) return;
  const { width, height } = document.body.getBoundingClientRect();
  notify("ui/notifications/size-changed", { width: Math.ceil(width), height: Math.ceil(height) });
}
handlers["ui/notifications/host-context-changed"] = (context) => {
  if (context.theme) setTheme(context.theme);
  if (context.containerDimensions) fitHeight(context.containerDimensions);
};

let toolArgs = null;
const toolInput = new Promise((resolve) => {
  handlers["ui/notifications/tool-input"] = (params) => resolve(params.arguments || {});
  setTimeout(() => resolve(null), 4000);
});

try {
  const init = await request("ui/initialize", {
    appInfo: { name: "grooph", version: "1.0.0" },
    appCapabilities: { availableDisplayModes: ["inline"] },
    protocolVersion: "2026-01-26",
  });
  connected = true;
  notify("ui/notifications/initialized", {});
  const context = init?.hostContext || {};
  if (context.theme) setTheme(context.theme);
  fitHeight(context.containerDimensions);
} catch (e) {
  console.warn("grooph: no MCP Apps host", e);
}
setTheme(theme);
new ResizeObserver(reportSize).observe(document.body);
reportSize();

toolArgs = await toolInput;
const query = metronomeQuery(toolArgs || {});
const appUrl = CONFIG.origin + "/";
const link = appUrl + "?" + query;

function openLink(url) {
  if (!connected) return window.open(url, "_blank", "noopener");
  request("ui/open-link", { url }).catch(() => window.open(url, "_blank", "noopener"));
}

window.groophHost = {
  appUrl,
  theme,
  // Returns whether it took the context; the app tries again if not. A
  // request the host fails is sent again a few seconds later, unless newer
  // settings replaced it.
  updateModelContext(text, fields) {
    if (!connected) return false;
    const params = { content: [{ type: "text", text }], structuredContent: fields };
    latestContext = params;
    const send = (tries) =>
      request("ui/update-model-context", params).catch((e) => {
        console.warn("grooph: model context not updated", e);
        if (tries > 1) setTimeout(() => latestContext === params && send(tries - 1), 3000);
      });
    send(3);
    return true;
  },
  openLink,
};

function fail(reason) {
  document.getElementById("stage").hidden = true;
  document.getElementById("reason").textContent = reason;
  document.getElementById("link").textContent = link;
  document.getElementById("open").onclick = () => openLink(link);
  document.getElementById("fallback").hidden = false;
  window.groophHost.updateModelContext(
    "The metronome view could not start: " + reason + " The user can open it on grooph.app: " + link,
    { link, error: reason },
  );
  reportSize();
}
// The app reads its link from the address bar once at start.
history.replaceState(null, "", "?" + query + "&embed=mcp&theme=" + theme);

// A tiny module first: a host whose CSP forbids WebAssembly fails here,
// before the 7 MB download.
const wasmAllowed = (() => {
  try {
    return new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])) instanceof WebAssembly.Module;
  } catch {
    return false;
  }
})();
if (!wasmAllowed) {
  fail("This chat app does not allow WebAssembly in its views (Content Security Policy), and grooph needs it.");
} else {
  try {
    const app = await import(CONFIG.origin + "/" + CONFIG.js);
    await app.default({ module_or_path: CONFIG.origin + "/" + CONFIG.wasm });
  } catch (e) {
    console.error("grooph: start failed", e);
    fail("It could not be loaded from " + CONFIG.origin + ": " + (e?.message || e) + ".");
  }
}
</script>
</body>
</html>
`;
}
