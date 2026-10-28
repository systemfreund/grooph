// grooph's MCP server (https://grooph.app/mcp), run as a Cloudflare Pages
// Function (functions/mcp.js): a stateless server over Streamable HTTP with
// JSON answers. Its one tool, `show_metronome`, shows the metronome inside
// the chat as an MCP App (https://modelcontextprotocol.io/extensions/apps/overview):
// the host reads the view (view.js) as the resource `ui://grooph/metronome.html`
// and runs it in a sandboxed iframe, which loads the web build of this same
// deployment, so a preview serves its own build.

import { MAX_LENGTH, argumentError, metronomeQuery } from "./link.js";
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
  description:
    "Shows grooph's interactive metronome in the chat, set to a tempo and time signature, optionally with an accent " +
    "pattern, a polyrhythm against the beat and swing. Playback starts when the user taps play. The user can change " +
    "the settings in the view; the current settings come back as model context. The result includes a link that " +
    "opens the same metronome on grooph.app.",
  inputSchema: {
    type: "object",
    properties: {
      bpm: {
        type: "integer",
        minimum: 20,
        maximum: 300,
        description: "Tempo in quarter notes per minute, also in x/8: 7/8 at 70 clicks 140 eighths per minute. Default 120.",
      },
      ts: {
        type: "string",
        maxLength: MAX_LENGTH.ts,
        description: "Time signature: 1-17 beats over 4, 8 or 16, e.g. 4/4, 7/8 or 2+2+3/8. Default 4/4.",
      },
      click: {
        type: "string",
        maxLength: MAX_LENGTH.click,
        description:
          "The click bar, replacing the default of one click per beat group with the first one strong: one bar of the " +
          "meter in grooph's rhythm notation, without the time signature. Tokens separated by spaces: h q e s (half to " +
          "sixteenth), dotted q. e., r: makes a rest, > an accent (strong click), (q) a ghost note (soft click). The bar " +
          'must add up exactly to the meter. Example in 7/8, middle group silent and the last soft: ">q r:q. (q)".',
      },
      poly: {
        type: "string",
        maxLength: MAX_LENGTH.poly,
        pattern: "^\\s*[0-9]+\\s*:\\s*[0-9]+\\s*$",
        description:
          "Polyrhythm n:m against the click: n even clicks in the time of m beat units of the meter (quarters in x/4, " +
          "eighths in x/8), repeated through the bar. m must divide the meter's beats; n is 2 to 16 and must not reduce " +
          'to one click (2:4 is 1:2). Examples: "3:2", "3:4", "5:4" in 4/4; "2:3", "4:3" in 3/4; "2:3" in 6/8 (two ' +
          'against each dotted quarter); "7:5" in 5/8.',
      },
      swing: { type: "integer", minimum: 50, maximum: 75, description: "Swing in percent: 50 is straight, 66 a triplet feel, 75 dotted." },
      su: { type: "integer", enum: [8, 16], description: "Note value that swings: 8 (default) or 16. Needs swing." },
    },
    additionalProperties: false,
  },
  annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  _meta: { ui: { resourceUri: RESOURCE_URI } },
};


/** Answers a request to `/mcp`. `env.ASSETS` serves the web build;
 * `checkLink(query)` says why a link would not open in the app, or `null`
 * (`linkChecker` in link-check.js). */
export async function handleRequest(request, env, checkLink) {
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
  const context = { origin: new URL(request.url).origin, env, checkLink };
  if (Array.isArray(message)) {
    const replies = (await Promise.all(message.map((m) => dispatch(m, context)))).filter(Boolean);
    return replies.length ? json(replies) : new Response(null, { status: 202, headers: CORS });
  }
  const reply = await dispatch(message, context);
  return reply ? json(reply) : new Response(null, { status: 202, headers: CORS });
}

async function dispatch(message, { origin, env, checkLink }) {
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
        instructions:
          "grooph is a metronome and rhythm trainer. Its tool show_metronome shows an interactive metronome for a " +
          "tempo, time signature, accent pattern, polyrhythm and swing.",
      });
    case "ping":
      return ok({});
    case "tools/list":
      return ok({ tools: [TOOL] });
    case "tools/call":
      if (params.name !== TOOL.name) return fail(-32602, `Unknown tool: ${params.name}`);
      return ok(showMetronome(params.arguments || {}, origin, checkLink));
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

/** The tool's result: the link, for hosts that show no view as well. Wrong
 * arguments come back as an error that says what to fix: the numbers from
 * `argumentError`, the notation and the polyrhythm from `checkLink`, by the
 * app's own parser. */
export function showMetronome(args, origin, checkLink) {
  const error = argumentError(args, Object.keys(TOOL.inputSchema.properties));
  if (error) return { isError: true, content: [{ type: "text", text: error }] };
  const query = metronomeQuery(args);
  // Without it the tool would answer success for links that do not open.
  if (typeof checkLink !== "function") throw new Error("showMetronome needs checkLink (link-check.js)");
  const problem = checkLink(query);
  if (problem) {
    const text = `These settings do not open in grooph: ${problem}. Fix the argument and call the tool again.`;
    return { isError: true, content: [{ type: "text", text }] };
  }
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
