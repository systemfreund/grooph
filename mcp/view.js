// The view of `show_metronome` (`ui://grooph/metronome.html`): a page the
// MCP Apps host runs in a sandboxed iframe on its own origin. It talks the
// MCP Apps protocol with the host over postMessage (JSON-RPC 2.0, spec
// 2026-01-26), puts the tool's link into the address bar with `embed=mcp`
// and the host's theme, and starts the web build of `origin` in embed mode
// (crates/app/src/embed.rs). `window.groophHost` is its bridge to the app:
// the app reports its settings through `updateModelContext`, opens links
// through `openLink`, and reads `appUrl`, `theme` and `colors` (the host's
// card colors, see `resolveColors`).

/** Inline height of the view, in CSS pixels, less if the host allows less:
 * the compact card (tempo, staff, ring with play in the middle) where the
 * host offers fullscreen for the rest, else the whole metronome with its
 * options folded. Fullscreen takes the host's height. */
export const INLINE_HEIGHT = 400;
export const INLINE_HEIGHT_WITHOUT_FULLSCREEN = 520;

/** The view's HTML for the web build at `origin` (`js` and `wasm` are the
 * hashed file names from embed.json). */
export function viewHtml({ origin, js, wasm }) {
  // Safe inside <script>: no "</" can close it early.
  const heights = { compact: INLINE_HEIGHT, whole: INLINE_HEIGHT_WITHOUT_FULLSCREEN };
  const config = JSON.stringify({ origin, js, wasm, heights }).replace(/</g, "\\u003c");
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, user-scalable=no">
<meta name="color-scheme" content="light dark">
<title>grooph metronome</title>
<style>
  html, body { margin: 0; padding: 0; overflow: hidden; }
  /* The host's colors (hostContext.styles.variables), else grooph's own. */
  body {
    font: 15px/1.4 system-ui, sans-serif;
    background: var(--color-background-primary, light-dark(#f8f8f8, #1b1b1b));
    color: var(--color-text-primary, light-dark(#222, #ddd));
  }
  #stage { position: relative; height: ${INLINE_HEIGHT}px; }
  /* Vertical drags scroll the chat; taps and horizontal drags go to the app. */
  html, canvas { touch-action: pan-y; }
  canvas { position: absolute; inset: 0; width: 100%; height: 100%; display: block; outline: none; }
  /* The skeleton while the app loads, laid out like the card it becomes
     (eframe removes #loading_text once the app runs). */
  #loading_text {
    --sk-card: var(--color-background-secondary, light-dark(#e6e6e6, #323232));
    --sk-ring: light-dark(#c8c8c8, #4a4a4a);
    --sk-accent: light-dark(#90d1ff, #005c80);
    position: absolute; inset: 0; display: flex; flex-direction: column;
    padding: 14px 20px 18px; box-sizing: border-box; gap: 10px;
  }
  .sk-row { display: flex; align-items: center; gap: 8px; height: 44px; flex: none; }
  .sk-tempo { font-size: 22px; margin-right: 4px; white-space: nowrap; }
  .sk-button { width: 44px; height: 44px; border-radius: 8px; background: var(--sk-card); }
  .sk-full { margin-left: auto; width: 18px; height: 18px; border: 2px solid var(--sk-ring); border-radius: 2px; }
  .sk-staff { display: flex; align-items: center; gap: 10px; height: 70px; flex: none; }
  .sk-meter { font: bold 26px/0.9 Georgia, serif; text-align: center; }
  .sk-line { flex: 1; height: 1px; background: currentColor; opacity: 0.5; }
  .sk-ring-box { flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; }
  .sk-ring {
    height: 88%; max-width: 88%; aspect-ratio: 1; border-radius: 50%; box-sizing: border-box;
    border: 14px solid var(--sk-ring); display: flex; align-items: center; justify-content: center;
  }
  .sk-play { width: 46%; aspect-ratio: 1; border-radius: 50%; background: var(--sk-accent); opacity: 0.6; }
  .sk-bar { display: none; height: 92px; flex: none; border-radius: 16px; background: var(--sk-card); }
  /* The whole metronome (fullscreen, or a host without it): the bar below
     instead of the tempo row. */
  #loading_text.whole .sk-row { display: none; }
  #loading_text.whole .sk-bar { display: block; }
  #loading_text.whole .sk-play, #loading_text.whole .sk-meter { display: none; }
  .sk-note { width: 14px; height: 26px; fill: currentColor; vertical-align: -3px; }
  .sk-ring, .sk-button, .sk-bar { animation: sk-pulse 1.4s ease-in-out infinite alternate; }
  @keyframes sk-pulse { from { opacity: 1; } to { opacity: 0.55; } }
  @media (prefers-reduced-motion: reduce) { .sk-ring, .sk-button, .sk-bar { animation: none; } }
  #fallback { padding: 16px 20px; }
  /* For screen readers only (the canvas has no accessibility tree); the
     button shows when it has the keyboard focus. */
  .sr-only:not(:focus) {
    position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0;
    overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; border: 0;
  }
  #a11y-play:focus { position: absolute; top: 8px; left: 8px; z-index: 1; font: inherit; padding: 8px 16px; }
  #fallback button { font: inherit; padding: 8px 16px; margin-top: 4px; cursor: pointer; }
  #fallback code { font-size: 13px; word-break: break-all; }
</style>
</head>
<body>
<div id="stage"><canvas id="grooph-canvas" aria-hidden="true"></canvas>
<div id="loading_text" role="status" aria-label="Loading the metronome…">
  <div class="sk-row" aria-hidden="true"><span class="sk-tempo"><svg class="sk-note" viewBox="0 0 14 26"><ellipse cx="6" cy="21" rx="5.6" ry="4" transform="rotate(-20 6 21)"/><rect x="10" y="1" width="1.6" height="20"/></svg><span> =</span></span><span class="sk-button"></span><span class="sk-button"></span><span class="sk-full"></span></div>
  <div class="sk-staff" aria-hidden="true"><span class="sk-meter"></span><span class="sk-line"></span></div>
  <div class="sk-ring-box" aria-hidden="true"><div class="sk-ring"><div class="sk-play"></div></div></div>
  <div class="sk-bar" aria-hidden="true"></div>
</div></div>
<button id="a11y-play" class="sr-only" type="button" aria-pressed="false">Play metronome</button>
<p id="a11y-status" class="sr-only" role="status" aria-live="polite"></p>
<div id="fallback" hidden>
  <p><strong>The metronome cannot run here.</strong> <span id="reason"></span></p>
  <p><button id="open">Open in grooph</button></p>
  <p><code id="link"></code></p>
</div>
<!-- Keeps the click audible with the iOS ring/silent switch on, as in the app's index.html. -->
<script src="${origin}/ios-audio.js"></script>
<script type="module">
const CONFIG = ${config};

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
// The host's style variables the app takes, by the name the app knows them.
const HOST_COLORS = {
  background: "--color-background-primary",
  card: "--color-background-secondary",
  text: "--color-text-primary",
  textWeak: "--color-text-secondary",
  border: "--color-border-primary",
};
let hostVariables = {};
let colors = "";
// The host's colors as the app reads them: "background=#rrggbbaa;card=…",
// only those the host gave. Values may use light-dark(), so they are
// resolved by the browser in the current color scheme.
function resolveColors() {
  const probe = document.createElement("span");
  document.body.appendChild(probe);
  const resolved = [];
  for (const [name, variable] of Object.entries(HOST_COLORS)) {
    if (!hostVariables[variable]) continue;
    probe.style.color = "var(" + variable + ")";
    const hex = cssColorToHex(getComputedStyle(probe).color);
    if (hex) resolved.push(name + "=" + hex);
  }
  probe.remove();
  return resolved.join(";");
}
// "rgb(1, 2, 3)", "rgba(1, 2, 3, 0.5)" or "rgb(1 2 3 / 50%)" as #rrggbbaa.
function cssColorToHex(css) {
  const parts = css.match(/rgba?\\(([^)]*)\\)/)?.[1].split(/[\\s,/]+/).filter(Boolean);
  if (!parts || parts.length < 3) return null;
  const alpha = parts[3] === undefined ? 1 : parts[3].endsWith("%") ? parseFloat(parts[3]) / 100 : parseFloat(parts[3]);
  const bytes = [...parts.slice(0, 3).map(Number), Math.round(alpha * 255)];
  return "#" + bytes.map((b) => Math.max(0, Math.min(255, Math.round(b))).toString(16).padStart(2, "0")).join("");
}
function applyHostStyles(styles) {
  for (const [variable, value] of Object.entries(styles?.variables || {})) {
    if (!value) continue;
    hostVariables[variable] = value;
    document.documentElement.style.setProperty(variable, value);
  }
}
function setTheme(next) {
  if (next !== "light" && next !== "dark") return;
  theme = next;
  // light-dark() in the host's variables follows the color scheme.
  document.documentElement.style.colorScheme = theme;
  colors = resolveColors();
  if (window.groophHost) {
    window.groophHost.theme = theme;
    window.groophHost.colors = colors;
    window.groophHost.repaint?.();
  }
}
// What the host covers at the view's edges (hostContext.safeAreaInsets,
// CSS pixels), e.g. Claude's composer over the bottom in fullscreen; the app
// keeps its controls out of it (groophHost.insets: "top,right,bottom,left").
let insets = "";
function setInsets(next) {
  if (!next) return;
  insets = ["top", "right", "bottom", "left"].map((side) => Math.max(0, Number(next[side]) || 0)).join(",");
  if (window.groophHost) {
    window.groophHost.insets = insets;
    window.groophHost.repaint?.();
  }
}

// Display mode: "inline" or "fullscreen", and the modes the host offers.
let displayMode = "inline";
let hostModes = [];
let dimensions = null;
const canFullscreen = () => hostModes.includes("fullscreen");
function fitHeight(next) {
  if (next) dimensions = next;
  const stage = document.getElementById("stage");
  if (displayMode === "fullscreen") {
    stage.style.height = Number.isFinite(dimensions?.height) ? dimensions.height + "px" : "100vh";
    return;
  }
  const base = canFullscreen() ? CONFIG.heights.compact : CONFIG.heights.whole;
  const limit = dimensions?.height ?? dimensions?.maxHeight;
  const height = Math.max(240, Math.min(base, Number.isFinite(limit) ? limit : base));
  stage.style.height = height + "px";
}
function setDisplay(mode, modes) {
  if (modes) hostModes = modes;
  if (mode) displayMode = mode;
  document.getElementById("loading_text")?.classList.toggle("whole", !canFullscreen() || displayMode === "fullscreen");
  fitHeight();
  if (window.groophHost) {
    window.groophHost.displayMode = displayMode;
    window.groophHost.displayModes = hostModes.join(",");
    window.groophHost.repaint?.();
  }
}
function reportSize() {
  if (!connected) return;
  const { width, height } = document.body.getBoundingClientRect();
  notify("ui/notifications/size-changed", { width: Math.ceil(width), height: Math.ceil(height) });
}
handlers["ui/notifications/host-context-changed"] = (context) => {
  console.info("grooph: host context changed", context);
  if (context.safeAreaInsets) setInsets(context.safeAreaInsets);
  if (context.styles) applyHostStyles(context.styles);
  if (context.theme || context.styles) setTheme(context.theme || theme);
  if (context.containerDimensions) fitHeight(context.containerDimensions);
  if (context.displayMode || context.availableDisplayModes) {
    setDisplay(context.displayMode, context.availableDisplayModes);
  }
};

let toolArgs = null;
const toolInput = new Promise((resolve) => {
  handlers["ui/notifications/tool-input"] = (params) => resolve(params.arguments || {});
  setTimeout(() => resolve(null), 4000);
});

try {
  const init = await request("ui/initialize", {
    appInfo: { name: "grooph", version: "1.0.0" },
    appCapabilities: { availableDisplayModes: ["inline", "fullscreen"] },
    protocolVersion: "2026-01-26",
  });
  connected = true;
  notify("ui/notifications/initialized", {});
  const context = init?.hostContext || {};
  console.info("grooph: host context", context);
  setInsets(context.safeAreaInsets);
  applyHostStyles(context.styles);
  if (context.theme) setTheme(context.theme);
  dimensions = context.containerDimensions || null;
  setDisplay(context.displayMode, context.availableDisplayModes);
} catch (e) {
  console.warn("grooph: no MCP Apps host", e);
}
setTheme(theme);
new ResizeObserver(reportSize).observe(document.body);
reportSize();

toolArgs = await toolInput;
// The skeleton shows the tool's tempo and meter while the app loads.
(() => {
  const bpm = Number(toolArgs?.bpm) || 120;
  const [top, unit] = String(toolArgs?.ts || "4/4").trim().split("/");
  const beats = String(top).split("+").reduce((sum, g) => sum + (Number(g) || 0), 0) || 4;
  const tempo = document.querySelector("#loading_text .sk-tempo");
  const meter = document.querySelector("#loading_text .sk-meter");
  if (tempo) tempo.lastChild.textContent = " = " + bpm;
  if (meter) meter.innerHTML = beats + "<br>" + (Number(unit) || 4);
})();
// The link builder the server uses too, served unbundled by the build
// (crates/site writes mcp/link.js to /embed/link.js).
let query;
try {
  const { metronomeQuery } = await import(CONFIG.origin + "/embed/link.js");
  query = metronomeQuery(toolArgs || {});
} catch (e) {
  console.error("grooph: link builder not loaded", e);
  query = "v=2&ts=4/4";
}
const appUrl = CONFIG.origin + "/";
const link = appUrl + "?" + query;

function openLink(url) {
  if (!connected) return window.open(url, "_blank", "noopener");
  request("ui/open-link", { url }).catch(() => window.open(url, "_blank", "noopener"));
}

window.groophHost = {
  appUrl,
  theme,
  colors,
  insets,
  displayMode,
  displayModes: hostModes.join(","),
  // The compact card's fullscreen button; the host answers with the mode
  // it chose (and may say so again in host-context-changed).
  requestFullscreen() {
    if (!connected || !canFullscreen()) return false;
    request("ui/request-display-mode", { mode: "fullscreen" })
      .then((result) => setDisplay(result?.mode))
      .catch((e) => console.warn("grooph: no fullscreen", e));
    return true;
  },
  // Returns whether it took the context; the app tries again if not. A
  // request the host fails is sent again a few seconds later, unless newer
  // settings replaced it.
  updateModelContext(text, fields) {
    announce(text, fields);
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

// What a screen reader gets of the canvas: the settings as the model gets
// them (without the link), read out when they change, and a play button
// that drives the app (groophHost.togglePlay, defined by the app).
let announced = "";
function announce(text, fields) {
  const status = text.split(" Link: ")[0];
  if (status !== announced) {
    announced = status;
    document.getElementById("a11y-status").textContent = status;
  }
  const playing = fields?.playing === true;
  const play = document.getElementById("a11y-play");
  play.setAttribute("aria-pressed", String(playing));
  play.textContent = playing ? "Stop metronome" : "Play metronome";
}
document.getElementById("a11y-play").addEventListener("click", () => window.groophHost?.togglePlay?.());

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
// before the WASM download (about 3 MB compressed).
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
    // eframe's hidden text field holds the keyboard focus; name it, so a
    // screen reader that lands on it says what it is.
    setTimeout(() => document.querySelector("#stage input")?.setAttribute("aria-label", "Metronome. Space plays or stops it."), 0);
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
