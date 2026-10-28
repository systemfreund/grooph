// https://grooph.app/mcp: grooph's MCP server as a Cloudflare Pages Function
// (see mcp/server.js). `wrangler pages deploy` picks up this directory from
// the repository root and bundles the imports, the link check's WebAssembly
// module included: the web build writes it to dist/ (Trunk.toml), which the
// deploy uploads.

import { linkChecker } from "../mcp/link-check.js";
import { handleRequest } from "../mcp/server.js";
import linkCheckWasm from "../dist/embed/link-check.wasm";

const checkLink = linkChecker(linkCheckWasm);

export const onRequest = (context) => handleRequest(context.request, context.env, checkLink);
