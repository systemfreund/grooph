// grooph's MCP server (https://grooph.app/mcp), run as a Cloudflare Pages
// Function (functions/mcp.js): a stateless server over Streamable HTTP with
// JSON answers. Its one tool, `show_metronome`, shows the metronome inside
// the chat as an MCP App (https://modelcontextprotocol.io/extensions/apps/overview):
// the host reads the view (view.js) as the resource `ui://grooph/metronome.html`
// and runs it in a sandboxed iframe, which loads the web build of this same
// deployment, so a preview serves its own build.

import { argumentError, metronomeQuery } from "./link.js";
import { viewHtml } from "./view.js";

export const RESOURCE_URI = "ui://grooph/metronome.html";
export const RESOURCE_MIME = "text/html;profile=mcp-app";
const SERVER_INFO = { name: "grooph", title: "grooph", version: "1.0.0" };
const PROTOCOL_VERSIONS = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "POST, OPTIONS",
  "Access-Control-Allow-Headers": "*",
  "Access-Control-Expose-Headers": "Mcp-Session-Id",
};

export const TOOL = {
  name: "show_metronome",
  title: "Show a metronome",
  description: `Shows grooph's metronome in the chat, set to a tempo and time signature, optionally with its own click pattern, a polyrhythm (a second click against the beat) and swing. The user starts it with the play button and can change the settings there; the current settings come back as model context. The result also has a link that opens the same metronome on https://grooph.app.

Tempo is always in quarter notes per minute: 7/8 at bpm 70 clicks 140 eighths per minute.

poly is a polyrhythm n:m: n even clicks in the time of m beat units of the meter (quarters in x/4, eighths in x/8), repeated until the bar is full. m must divide the beats of the meter, n is 2 to 16, and the ratio must not reduce to one click (2:4 is 1:2); not every such ratio fits every meter, and grooph names the ones that do. Examples: "3 against 2" in 4/4 is poly "3:2", "3 against 4" is "3:4", "5 against 4" is "5:4"; in 3/4 "2:3" or "4:3"; in 6/8 "2:3" is two against each dotted quarter; in 5/8 "7:5".

click replaces the main click: one bar of the meter in grooph's rhythm notation, without the time signature. Tokens separated by spaces: h q e s (half to sixteenth), dotted q. e., r: makes a rest, > an accent (strong click), (q) a ghost note (soft click); the bar must add up exactly to the meter. E.g. 7/8 with the middle group silent and the last soft is ">q r:q. (q)". Default: one click per beat group, the first one strong. More at https://grooph.app/llms.txt.`,
  inputSchema: {
    type: "object",
    properties: {
      bpm: { type: "integer", minimum: 20, maximum: 300, description: "Tempo in quarter notes per minute. Default 120." },
      ts: { type: "string", description: "Time signature: 1-17 beats over 4, 8 or 16, e.g. 4/4, 7/8 or 2+2+3/8. Default 4/4." },
      click: { type: "string", description: "The click bar in rhythm notation without the time signature, e.g. \">q r:q. (q)\" in 7/8." },
      poly: { type: "string", pattern: "^\\s*[0-9]+\\s*:\\s*[0-9]+\\s*$", description: "Polyrhythm n:m against the click, e.g. \"3:2\" for three against two in 4/4." },
      swing: { type: "integer", minimum: 50, maximum: 75, description: "Swing in percent: 50 is straight, 66 a triplet feel, 75 dotted." },
      su: { type: "integer", enum: [8, 16], description: "Note value that swings: 8 (default) or 16. Needs swing." },
    },
    additionalProperties: false,
  },
  annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  _meta: { ui: { resourceUri: RESOURCE_URI } },
};

/** Answers a request to `/mcp`. `env.ASSETS` serves the web build. */
export async function handleRequest(request, env) {
  if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: CORS });
  if (request.method !== "POST") {
    return new Response("grooph MCP server: POST JSON-RPC messages here.\n", {
      status: 405,
      headers: { ...CORS, Allow: "POST, OPTIONS" },
    });
  }
  let message;
  try {
    message = await request.json();
  } catch {
    return json({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "Parse error" } });
  }
  const context = { origin: new URL(request.url).origin, env };
  if (Array.isArray(message)) {
    const replies = (await Promise.all(message.map((m) => dispatch(m, context)))).filter(Boolean);
    return replies.length ? json(replies) : new Response(null, { status: 202, headers: CORS });
  }
  const reply = await dispatch(message, context);
  return reply ? json(reply) : new Response(null, { status: 202, headers: CORS });
}

async function dispatch(message, { origin, env }) {
  // Notifications (no id) and answers get no answer.
  if (message?.id === undefined || message.id === null || !message.method) return null;
  const ok = (result) => ({ jsonrpc: "2.0", id: message.id, result });
  const fail = (code, text) => ({ jsonrpc: "2.0", id: message.id, error: { code, message: text } });
  const params = message.params || {};
  switch (message.method) {
    case "initialize":
      return ok({
        protocolVersion: PROTOCOL_VERSIONS.includes(params.protocolVersion) ? params.protocolVersion : PROTOCOL_VERSIONS[0],
        capabilities: { tools: {}, resources: {}, extensions: { "io.modelcontextprotocol/ui": {} } },
        serverInfo: SERVER_INFO,
        instructions: "grooph is a metronome and rhythm trainer. Call show_metronome to give the user a working metronome in the chat.",
      });
    case "ping":
      return ok({});
    case "tools/list":
      return ok({ tools: [TOOL] });
    case "tools/call":
      if (params.name !== TOOL.name) return fail(-32602, `Unknown tool: ${params.name}`);
      return ok(showMetronome(params.arguments || {}, origin));
    case "resources/list":
      return ok({ resources: [{ uri: RESOURCE_URI, name: "grooph metronome", mimeType: RESOURCE_MIME }] });
    case "resources/templates/list":
      return ok({ resourceTemplates: [] });
    case "resources/read": {
      if (params.uri !== RESOURCE_URI) return fail(-32602, `Unknown resource: ${params.uri}`);
      const assets = await appAssets(origin, env);
      if (!assets) return fail(-32603, "The web build of grooph is missing (no embed.json).");
      return ok({
        contents: [
          {
            uri: RESOURCE_URI,
            mimeType: RESOURCE_MIME,
            text: viewHtml({ origin, ...assets }),
            _meta: { ui: { csp: { connectDomains: [origin], resourceDomains: [origin] }, prefersBorder: true } },
          },
        ],
      });
    }
    default:
      return fail(-32601, `Method not found: ${message.method}`);
  }
}

/** The tool's result: the link, for hosts that show no view as well. */
export function showMetronome(args, origin) {
  const error = argumentError(args, Object.keys(TOOL.inputSchema.properties));
  if (error) return { isError: true, content: [{ type: "text", text: error }] };
  const query = metronomeQuery(args);
  const link = `${origin}/?${query}`;
  return {
    content: [
      {
        type: "text",
        text:
          `The metronome is shown to the user; they start it with the play button. Open in grooph: ${link}\n` +
          "If the click notation or the polyrhythm does not fit the meter, grooph shows an error instead; " +
          "the view then reports it as model context, so check it before telling the user the metronome is set.",
      },
    ],
    structuredContent: { link },
  };
}

/** Hashed names of the app's JS glue and WASM, from `embed.json`, which the
 * site build writes next to `index.html` (crates/site). */
async function appAssets(origin, env) {
  try {
    const response = await env.ASSETS.fetch(new Request(`${origin}/embed.json`));
    if (!response.ok) return null;
    const { js, wasm } = await response.json();
    return js && wasm ? { js, wasm } : null;
  } catch {
    return null;
  }
}

function json(body) {
  return new Response(JSON.stringify(body), { headers: { ...CORS, "Content-Type": "application/json" } });
}
