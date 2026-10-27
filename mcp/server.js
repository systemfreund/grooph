// grooph's MCP server (https://grooph.app/mcp), run as a Cloudflare Pages
// Function (functions/mcp.js): a stateless server over Streamable HTTP with
// JSON answers. Its one tool, `show_metronome`, shows the metronome inside
// the chat as an MCP App (https://modelcontextprotocol.io/extensions/apps/overview):
// the host reads the view (view.js) as the resource `ui://grooph/metronome.html`
// and runs it in a sandboxed iframe, which loads the web build of this same
// deployment, so a preview serves its own build.

import { POLYRHYTHMS, argumentError, metronomeQuery } from "./link.js";
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

const polyrhythmTable = POLYRHYTHMS.map((p) => `- ${p.ratio} in ${p.ts}: ts "${p.ts}", counter "${p.counter}"`).join("\n");

export const TOOL = {
  name: "show_metronome",
  title: "Show a metronome",
  description: `Shows grooph's metronome in the chat, set to a tempo and time signature, optionally with its own click pattern, a polyrhythm (a second click against the beat) and swing. The user starts it with the play button and can change the settings there; the current settings come back as model context. The result also has a link that opens the same metronome on https://grooph.app.

Tempo is always in quarter notes per minute: 7/8 at bpm 70 clicks 140 eighths per minute.

click and counter are one bar of the meter in grooph's rhythm notation, without the time signature: tokens separated by spaces, h q e s th (half to thirty-second), dotted q. e. s., triplets t2 t4 t8 t16, quintuplets qt4 qt8 qt16, septuplets spt4 spt8 spt16, any other tuplet n:m plus a note value (7:5e). r: makes a rest, > an accent (strong click), (e) a ghost note (soft click). The bar must add up exactly to the meter.

click replaces the main click: e.g. 7/8 with the middle group silent and the last soft is ">q r:q. (q)". Default: one click per beat group, the first one strong.

counter is a polyrhythm: n clicks evenly in the time of m beat units of the meter, repeated until the bar is full. Common ones:
${polyrhythmTable}
Any other ratio: one bar with n notes spread evenly over every m beats, using a tuplet when the clicks do not fall on plain note values (5/8, 7:5: "7:5e 7:5e 7:5e 7:5e 7:5e 7:5e 7:5e"). More at https://grooph.app/llms.txt.`,
  inputSchema: {
    type: "object",
    properties: {
      bpm: { type: "integer", minimum: 20, maximum: 300, description: "Tempo in quarter notes per minute. Default 120." },
      ts: { type: "string", description: "Time signature: 1-17 beats over 4, 8 or 16, e.g. 4/4, 7/8 or 2+2+3/8. Default 4/4." },
      click: { type: "string", description: "The click bar in rhythm notation without the time signature, e.g. \">q r:q. (q)\" in 7/8." },
      counter: { type: "string", description: "A counter rhythm (polyrhythm) in rhythm notation without the time signature, e.g. \"t4 t4 t4 t4 t4 t4\" for 3:2 in 4/4." },
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
        protocolVersion: PROTOCOL_VERSIONS.includes(params.protocolVersion) ? params.protocolVersion : PROTOCOL_VERSIONS[1],
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
  const error = argumentError(args);
  if (error) return { isError: true, content: [{ type: "text", text: error }] };
  const query = metronomeQuery(args);
  const link = `${origin}/?${query}`;
  return {
    content: [
      {
        type: "text",
        text: `The metronome is shown to the user; they start it with the play button. Open in grooph: ${link}`,
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
