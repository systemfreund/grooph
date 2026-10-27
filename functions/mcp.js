// https://grooph.app/mcp: grooph's MCP server as a Cloudflare Pages Function
// (see mcp/server.js). `wrangler pages deploy` picks up this directory from
// the repository root and bundles the imports.

import { handleRequest } from "../mcp/server.js";

export const onRequest = (context) => handleRequest(context.request, context.env);
