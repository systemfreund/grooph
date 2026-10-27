# grooph as an MCP App: spike

Checks whether grooph can run inside a chat as an
[MCP App](https://modelcontextprotocol.io/extensions/apps/overview). Not
part of the build or CI.

`worker.js` is a stateless MCP server (Streamable HTTP, JSON responses) for
Cloudflare Workers, without dependencies. Its one tool, `grooph_mcp_test`
(optional `rhythm`, `bpm`, `ts`), opens a view that checks in turn:

1. the `ui/initialize` handshake with the host (and shows the host context),
2. compiling a tiny WebAssembly module, which the host's CSP may forbid
   (the spec's default `script-src` has no `'wasm-unsafe-eval'`),
3. loading the current grooph build from `GROOPH_ORIGIN` (grooph.app) and
   starting it with the tool's rhythm as link parameters; the Worker proxies
   the JS and WASM under `/app/` because grooph.app sends no CORS headers,
4. playing a click through Web Audio (after a click on the button).

The view shows each result and reports them to the model with
`ui/update-model-context`, so you can ask Claude what came out.

## Deploy and connect

```sh
cd experiments/mcp-app
npx wrangler deploy
```

Then add `https://grooph-mcp-spike.<account>.workers.dev/mcp` in Claude under
Settings → Connectors → Add custom connector, and ask Claude to call
`grooph_mcp_test`, for example with a rhythm.

## Try locally

`test/local-host.mjs` stands in for a host: it serves `dist/` as grooph.app,
runs the Worker and shows the view in an iframe on another origin, with the
CSP the spec prescribes for the declared domains.

```sh
trunk build --release
node experiments/mcp-app/test/local-host.mjs
# http://localhost:8080        spec default CSP
# http://localhost:8080/?wasm=1 the same plus 'wasm-unsafe-eval'
```
